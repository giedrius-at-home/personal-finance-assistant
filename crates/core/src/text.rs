//! Conservative parsing of bank memos and account identifiers.

use std::sync::LazyLock;

use jiff::civil::Date;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Card bookings can lag the purchase by a few days; a parsed date further
/// back (or after the booking) is more likely another number than the card date.
pub const CARD_BOOKING_LAG_DAYS: i64 = 10;

/// ISO 3166-1 alpha-2, plus XK as used in payment identifiers.
const COUNTRIES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO \
BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ \
DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP \
GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG \
KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML \
MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE \
PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL \
SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM \
US UY UZ VA VC VE VG VI VN VU WF WS XK YE YT ZA ZM ZW";

pub fn is_country(code: &str) -> bool {
    code.len() == 2 && COUNTRIES.split(' ').any(|c| c == code)
}

/// Where a country claim comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountryEvidence {
    pub code: String,
    /// "account_iban", "description_country", "description_iban" or "description_bic".
    pub basis: String,
}

/// An IBAN with spaces removed and upper-cased, if it is shaped like one.
pub fn compact_iban(value: &str) -> Option<String> {
    static SHAPE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}$").expect("valid regex"));
    let compact: String = value
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    SHAPE.is_match(&compact).then_some(compact)
}

/// The country of a valid IBAN (checksum verified).
pub fn iban_country(value: Option<&str>) -> Option<CountryEvidence> {
    let compact = compact_iban(value?)?;
    if !is_country(&compact[..2]) {
        return None;
    }
    let rotated = format!("{}{}", &compact[4..], &compact[..4]);
    let mut remainder: u32 = 0;
    for c in rotated.chars() {
        let digits = if c.is_ascii_alphabetic() {
            (c as u32 - 55).to_string()
        } else {
            c.to_string()
        };
        for d in digits.chars() {
            remainder = (remainder * 10 + d.to_digit(10)?) % 97;
        }
    }
    (remainder == 1).then(|| CountryEvidence {
        code: compact[..2].to_owned(),
        basis: "account_iban".into(),
    })
}

/// The date a card was used, parsed from a Swedbank or ABN AMRO card memo.
///
/// Only clearly card-shaped memos are read, and only a date 0–10 days before
/// the booking is trusted; anything else returns `None`.
pub fn card_purchase_date(description: Option<&str>, booked_on: Date) -> Option<Date> {
    static SWEDBANK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^PIRKINYS\s+\S+\s+(?:(\d{4})\.(\d{2})\.(\d{2})|(\d{2})\.(\d{2})\.(\d{2}))\s",
        )
        .expect("valid regex")
    });
    static ABN_CARD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^BEA,|\bbetaalpas\b").expect("valid regex"));
    // Python uses a `(?<!\d)` lookbehind; the regex crate has none, so the
    // preceding character is checked by hand.
    static ABN_STAMP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(\d{2})\.(\d{2})\.(\d{2})/\d{2}:\d{2}").expect("valid regex")
    });
    let description = description?;
    let num = |s: Option<regex::Match<'_>>| s.and_then(|m| m.as_str().parse::<i16>().ok());
    let parsed = if let Some(c) = SWEDBANK.captures(description) {
        if c.get(1).is_some() {
            safe_date(num(c.get(1))?, num(c.get(2))?, num(c.get(3))?)
        } else {
            safe_date(2000 + num(c.get(6))?, num(c.get(5))?, num(c.get(4))?)
        }
    } else if ABN_CARD.is_match(description) {
        let mut found = None;
        let mut start = 0;
        while let Some(c) = ABN_STAMP.captures_at(description, start) {
            let m = c.get(0)?;
            let preceded_by_digit = description[..m.start()]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_ascii_digit());
            if !preceded_by_digit {
                found = safe_date(2000 + num(c.get(3))?, num(c.get(2))?, num(c.get(1))?);
                break;
            }
            start = m.start() + 1;
        }
        found
    } else {
        None
    }?;
    let lag = (booked_on - parsed).get_days() as i64;
    (0..=CARD_BOOKING_LAG_DAYS).contains(&lag).then_some(parsed)
}

fn safe_date(year: i16, month: i16, day: i16) -> Option<Date> {
    Date::new(year, i8::try_from(month).ok()?, i8::try_from(day).ok()?).ok()
}

const PLACEHOLDER_NAMES: [&str; 3] = ["(unknown source)", "(unknown destination)", "(no name)"];

/// A counterparty account name, unless it is one of Firefly's placeholders.
pub fn counterparty_name(name: Option<&str>) -> Option<String> {
    let cleaned = name.unwrap_or("").trim();
    if cleaned.is_empty() || PLACEHOLDER_NAMES.contains(&cleaned.to_lowercase().as_str()) {
        return None;
    }
    Some(cleaned.chars().take(256).collect())
}

/// Python's `str.casefold()` for the text this app sees.
pub fn casefold(text: &str) -> String {
    text.to_lowercase().replace('ß', "ss")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn iban_checksum() {
        assert_eq!(
            iban_country(Some("NL43 ABNA 0136 0063 96")).map(|c| c.code),
            Some("NL".into())
        );
        assert!(iban_country(Some("NL44ABNA0136006396")).is_none());
        assert!(iban_country(Some("not an iban")).is_none());
    }

    #[test]
    fn card_dates() {
        let booked = date(2026, 10, 14);
        assert_eq!(
            card_purchase_date(
                Some("PIRKINYS 5168******1234 2026.10.12 18:44 4.50 EUR (123) CAFE"),
                booked
            ),
            Some(date(2026, 10, 12))
        );
        assert_eq!(
            card_purchase_date(
                Some("PIRKINYS 5168******1234 12.10.26 18:44 4.50 EUR (123) CAFE"),
                booked
            ),
            Some(date(2026, 10, 12))
        );
        assert_eq!(
            card_purchase_date(
                Some("BEA, Betaalpas AH 1234,PAS123 NR:XYZ, 11.10.26/13:05 AMSTERDAM"),
                booked
            ),
            Some(date(2026, 10, 11))
        );
        // A stamp glued to other digits is not a card date.
        assert_eq!(
            card_purchase_date(Some("BEA, Betaalpas X 911.10.26/13:05"), booked),
            None
        );
        // Too old to be the card date.
        assert_eq!(
            card_purchase_date(Some("BEA, Betaalpas X 01.09.26/13:05"), booked),
            None
        );
    }

    #[test]
    fn placeholders() {
        assert_eq!(counterparty_name(Some("(unknown destination)")), None);
        assert_eq!(
            counterparty_name(Some(" Albert Heijn ")),
            Some("Albert Heijn".into())
        );
    }
}
