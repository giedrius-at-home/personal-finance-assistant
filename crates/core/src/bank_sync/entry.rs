//! One Enable Banking transaction, read as the Firefly data importer 2.3.4
//! read it (`EnableBanking/Model/Transaction.php`, `TransactionsResponse.php`),
//! so synced rows look like the ones it imported.

use std::str::FromStr;

use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `entry_reference`, or a hash of the entry when the bank sends none.
    pub id: String,
    /// Signed: negative is money out.
    pub amount: Decimal,
    pub currency: String,
    /// Booking date (or transaction date), else value date.
    pub date: Option<Date>,
    pub creditor_name: String,
    pub creditor_iban: String,
    pub debtor_name: String,
    pub debtor_iban: String,
    pub remittance: String,
    pub additional: String,
    /// `booked` or `pending`.
    pub status: String,
    pub sub_code: Option<String>,
    /// SHA-256 of the entry as received, for `source_versions`.
    pub content_hash: String,
}

fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    path.iter()
        .try_fold(value, |v, key| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn day(value: &Value, key: &str) -> Option<Date> {
    let raw = value.get(key)?.as_str()?;
    raw.get(..10)?.parse().ok()
}

impl Entry {
    /// `Transaction::fromArray` with the status the response wrapper set.
    pub fn parse(value: &Value, status: &str) -> Self {
        let mut amount = value
            .get("transaction_amount")
            .and_then(|a| a.get("amount"))
            .and_then(|a| match a {
                Value::String(s) => Decimal::from_str(s.trim()).ok(),
                Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
                _ => None,
            })
            .unwrap_or_default();
        if text(value, &["credit_debit_indicator"]) == "DBIT" && !amount.is_sign_negative() {
            amount = -amount;
        }
        let remittance = match value.get("remittance_information") {
            Some(Value::Array(lines)) => lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_owned(),
            Some(Value::String(s)) => s.trim().to_owned(),
            _ => String::new(),
        };
        let additional = value
            .get("additional_information")
            .and_then(Value::as_str)
            .or_else(|| value.get("note").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned();
        let encoded = value.to_string();
        let content_hash = super::sha256_hex(&encoded);
        let reference = text(value, &["entry_reference"]);
        let id = if reference.is_empty() {
            format!("eb-{}", &content_hash[..32])
        } else {
            let collapsed = reference.split_whitespace().collect::<Vec<_>>().join(" ");
            collapsed.chars().take(125).collect()
        };
        let sub_code = value
            .get("bank_transaction_code")
            .and_then(|c| c.get("sub_code"))
            .and_then(|c| match c {
                Value::String(s) => Some(s.clone()),
                Value::Null => None,
                other => Some(other.to_string()),
            });
        Self {
            id,
            amount,
            currency: text(value, &["transaction_amount", "currency"]).to_owned(),
            date: day(value, "booking_date")
                .or_else(|| day(value, "transaction_date"))
                .or_else(|| day(value, "value_date")),
            creditor_name: text(value, &["creditor", "name"]).to_owned(),
            creditor_iban: text(value, &["creditor_account", "iban"]).to_owned(),
            debtor_name: text(value, &["debtor", "name"]).to_owned(),
            debtor_iban: text(value, &["debtor_account", "iban"]).to_owned(),
            remittance,
            additional,
            status: status.to_owned(),
            sub_code,
            content_hash,
        }
    }

    /// Every entry of one response page: either a flat list whose `status`
    /// is `BOOK` or not, or `{"booked": [...], "pending": [...]}`. `None`
    /// when the page has no transaction list at all.
    pub fn page(page: &Value) -> Option<Vec<Self>> {
        let transactions = page.get("transactions")?;
        match transactions {
            Value::Array(list) => Some(
                list.iter()
                    .map(|t| {
                        let status = t.get("status").and_then(Value::as_str).unwrap_or("BOOK");
                        Self::parse(
                            t,
                            if status == "BOOK" {
                                "booked"
                            } else {
                                "pending"
                            },
                        )
                    })
                    .collect(),
            ),
            Value::Object(nested) => {
                let side = |key: &str, status: &str| {
                    nested
                        .get(key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(move |t| Self::parse(t, status))
                        .collect::<Vec<_>>()
                };
                let mut all = side("booked", "booked");
                all.extend(side("pending", "pending"));
                Some(all)
            }
            _ => None,
        }
    }

    /// `getDescription` after `cleanStringAndNewlines`, at most 1000
    /// characters (Firefly's limit).
    pub fn description(&self) -> String {
        let raw = if !self.remittance.is_empty() {
            &self.remittance
        } else if !self.additional.is_empty() {
            &self.additional
        } else {
            "(no description)"
        };
        let clean = clean(raw);
        if clean.is_empty() {
            "(no description)".to_owned()
        } else {
            clean.chars().take(1000).collect()
        }
    }

    /// `getNotes`: the additional information when it says something else.
    pub fn notes(&self) -> Option<String> {
        (!self.additional.is_empty() && self.additional != self.remittance)
            .then(|| self.additional.trim().to_owned())
            .filter(|n| !n.is_empty())
    }

    /// The counterparty name: the creditor for money out, the debtor for
    /// money in, or Firefly's placeholder.
    pub fn counterparty(&self) -> String {
        let (name, unknown) = if self.amount.is_sign_negative() {
            (&self.creditor_name, "(unknown destination)")
        } else {
            (&self.debtor_name, "(unknown source)")
        };
        let clean = clean(name);
        if clean.is_empty() {
            unknown.to_owned()
        } else {
            clean
        }
    }

    /// The counterparty IBAN, normalised (upper case, no spaces).
    pub fn counterparty_iban(&self) -> Option<String> {
        let iban = if self.amount.is_sign_negative() {
            &self.creditor_iban
        } else {
            &self.debtor_iban
        };
        let iban = super::normalize_iban(iban);
        (!iban.is_empty()).then_some(iban)
    }
}

/// Firefly's `cleanStringAndNewlines`: control characters, line breaks and
/// the odd Unicode spaces become plain spaces; the ends are trimmed.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control()
                || c.is_whitespace()
                || matches!(c, '\u{200B}' | '\u{FEFF}' | '\u{180E}')
            {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_debit_like_the_importer() {
        let e = Entry::parse(
            &json!({
                "entry_reference": "  ABC  123 ",
                "transaction_amount": {"amount": "12.30", "currency": "EUR"},
                "credit_debit_indicator": "DBIT",
                "booking_date": "2026-10-20",
                "value_date": "2026-10-21",
                "creditor": {"name": "Gran Cafe\nTorino"},
                "creditor_account": {"iban": "nl91 abna 0417 1643 00"},
                "remittance_information": ["Card", "payment"],
                "additional_information": "Card payment",
                "bank_transaction_code": {"sub_code": "POSD"}
            }),
            "booked",
        );
        assert_eq!(e.id, "ABC 123");
        assert_eq!(e.amount, Decimal::from_str("-12.30").unwrap());
        assert_eq!(e.date, Some(jiff::civil::date(2026, 10, 20)));
        assert_eq!(e.description(), "Card payment");
        assert_eq!(e.notes(), None);
        assert_eq!(e.counterparty(), "Gran Cafe Torino");
        assert_eq!(e.counterparty_iban().as_deref(), Some("NL91ABNA0417164300"));
        assert_eq!(e.sub_code.as_deref(), Some("POSD"));
    }

    #[test]
    fn credit_without_reference_or_names() {
        let e = Entry::parse(
            &json!({"transaction_amount": {"amount": "5", "currency": "EUR"},
                    "credit_debit_indicator": "CRDT", "value_date": "2026-10-02",
                    "note": "Interest"}),
            "booked",
        );
        assert!(e.id.starts_with("eb-"));
        assert_eq!(e.amount, Decimal::from(5));
        assert_eq!(e.date, Some(jiff::civil::date(2026, 10, 2)));
        assert_eq!(e.description(), "Interest");
        assert_eq!(e.notes().as_deref(), Some("Interest"));
        assert_eq!(e.counterparty(), "(unknown source)");
    }

    #[test]
    fn reads_both_page_shapes() {
        let flat = json!({"transactions": [
            {"entry_reference": "a", "status": "BOOK"},
            {"entry_reference": "b", "status": "PDNG"}
        ]});
        let statuses: Vec<String> = Entry::page(&flat)
            .unwrap()
            .into_iter()
            .map(|e| e.status)
            .collect();
        assert_eq!(statuses, ["booked", "pending"]);
        let nested =
            json!({"transactions": {"booked": [{"entry_reference": "a"}], "pending": [{}]}});
        assert_eq!(Entry::page(&nested).unwrap().len(), 2);
        assert!(Entry::page(&json!({})).is_none());
    }
}
