//! Merchant features and conservative memo parsing (Python
//! `domain/categorisation.extract_features` and
//! `domain/transaction_description.parse_transaction_description`).

use std::sync::LazyLock;

use regex::Regex;

use super::pyjson::J;
pub use crate::writes::review::{has_merchant_description, normalize_merchant};
use crate::writes::transactions::sha256_hex;

/// `MERCHANT_MAX_LENGTH`.
pub const MERCHANT_MAX_LENGTH: usize = 200;

/// Bounded features of one description (`MerchantFeatures`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Features {
    pub exact: String,
    pub normalized: String,
}

impl Features {
    /// `feature_hash`: sha256 of the sorted, compact JSON of both fields.
    pub fn hash(&self) -> String {
        sha256_hex(
            &J::obj([
                ("exact", J::str(&self.exact)),
                ("normalized", J::str(&self.normalized)),
            ])
            .dumps_sorted_compact(),
        )
    }
}

/// `extract_features`; `None` when the description has no usable text.
pub fn extract_features(description: &str) -> Option<Features> {
    let exact: String = description
        .trim()
        .chars()
        .take(MERCHANT_MAX_LENGTH)
        .collect();
    let normalized = normalize_merchant(description);
    (!exact.is_empty() && !normalized.is_empty()).then_some(Features { exact, normalized })
}

/// Explicit memo fields; the memo itself is never rewritten.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Description {
    pub redacted_original: String,
    pub merchant: Option<String>,
    pub purpose: Option<String>,
    pub payment_method: Option<String>,
}

impl Description {
    /// The merchant and purpose, or the redacted memo.
    pub fn retrieval_text(&self) -> String {
        let parts: Vec<&str> = [self.merchant.as_deref(), self.purpose.as_deref()]
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            self.redacted_original.clone()
        } else {
            parts.join(" ")
        }
    }
}

/// `(.+?)(?=terminator|$)` after `start`: Rust's regex has no look-ahead, so
/// the lazy capture is found by searching for the first terminator.
fn until(text: &str, start: usize, terminator: &Regex) -> Option<String> {
    let rest = &text[start..];
    let first = rest.chars().next()?;
    let from = first.len_utf8();
    let end = terminator
        .find_at(rest, from)
        .map_or(rest.len(), |m| m.start());
    let value = &rest[..end];
    (!value.contains('\n')).then(|| value.to_owned())
}

fn some_normalized(value: Option<String>) -> Option<String> {
    value
        .map(|v| normalize_merchant(&v))
        .filter(|v| !v.is_empty())
}

/// `parse_transaction_description`.
pub fn parse_description(description: &str) -> Description {
    static CARD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^BEA,\s*(Betaalpas|Apple Pay|Google Pay)\s+(.+?)(?:,\s*PAS(?:\d|number)|$)",
        )
        .expect("valid regex")
    });
    static PURCHASE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^PIRKINYS\s+\S+\s+\d{2}\.\d{2}\.\d{2}\s+\d{2}:\d{2}\s+[\d.,]+\s+[A-Z]{3}\s+\((?:\d+|number)\)\s+(.+)$",
        )
        .expect("valid regex")
    });
    static SEPA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^SEPA\b").expect("valid regex"));
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bNaam:\s*").expect("valid regex"));
    static NAME_END: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s+(?:Omschrijving|Kenmerk|Machtiging|IBAN|BIC):").expect("valid regex")
    });
    static MEMO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\bOmschrijving:\s*").expect("valid regex"));
    static MEMO_END: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s+(?:Kenmerk|Machtiging|IBAN|BIC):").expect("valid regex")
    });
    static TIKKIE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^Tikkie ID\s+(?:\d+|number),").expect("valid regex"));
    static TIKKIE_MEMO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^Tikkie ID\s+(?:\d+|number),\s*(.+?),\s*Van\b").expect("valid regex")
    });

    let mut parsed = Description {
        redacted_original: normalize_merchant(description),
        ..Description::default()
    };
    let (mut merchant, mut purpose) = (None, None);
    if let Some(card) = CARD.captures(description) {
        parsed.payment_method = Some(card[1].to_owned());
        merchant = Some(card[2].to_owned());
    } else if let Some(purchase) = PURCHASE.captures(description) {
        merchant = Some(purchase[1].to_owned());
        parsed.payment_method = Some("card".into());
    } else if SEPA.is_match(description) {
        let first = |start: &Regex, end: &Regex| {
            start
                .find_iter(description)
                .find_map(|m| until(description, m.end(), end))
        };
        merchant = first(&NAME, &NAME_END);
        purpose = first(&MEMO, &MEMO_END);
    } else if TIKKIE.is_match(description)
        && let Some(memo) = TIKKIE_MEMO.captures(description)
    {
        purpose = Some(memo[1].to_owned());
        parsed.payment_method = Some("Tikkie".into());
    }
    parsed.merchant = some_normalized(merchant);
    parsed.purpose = some_normalized(purpose);
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bank_memo_fields() {
        let card = parse_description("BEA, Betaalpas AH to go 1234,PAS123 NR:ABC");
        assert_eq!(card.payment_method.as_deref(), Some("Betaalpas"));
        assert_eq!(card.merchant.as_deref(), Some("ah to go number"));

        let sepa = parse_description(
            "SEPA Overboeking IBAN: NL91ABNA0417164300 BIC: ABNANL2A Naam: Jane Doe Omschrijving: rent may Kenmerk: 1",
        );
        assert_eq!(sepa.merchant.as_deref(), Some("jane doe"));
        assert_eq!(sepa.purpose.as_deref(), Some("rent may"));
        assert_eq!(sepa.retrieval_text(), "jane doe rent may");

        let tikkie = parse_description("Tikkie ID 0123456, pizza night, Van J Doe");
        assert_eq!(tikkie.purpose.as_deref(), Some("pizza night"));
        assert_eq!(tikkie.payment_method.as_deref(), Some("Tikkie"));

        let plain = parse_description("Albert Heijn 1234");
        assert_eq!(plain.retrieval_text(), "albert heijn number");
    }

    #[test]
    fn feature_hash_is_stable() {
        let f = extract_features("  Café Noir ").unwrap();
        assert_eq!(f.exact, "Café Noir");
        assert_eq!(f.normalized, "café noir");
        // sha256 of {"exact":"Café Noir","normalized":"café noir"}
        assert_eq!(
            f.hash(),
            sha256_hex(r#"{"exact":"Caf\u00e9 Noir","normalized":"caf\u00e9 noir"}"#)
        );
        assert!(extract_features("   ").is_none());
    }
}
