//! Supervision from the ledger's own categories
//! (`build_category_history_context`): the approved vocabulary, one rule per
//! merchant that only ever had one category, prompt examples and the
//! retrieval corpus.

use std::collections::{BTreeMap, HashMap};

use super::pyjson::J;
use super::text::{has_merchant_description, normalize_merchant};
use crate::model::{Kind, Tx};
use crate::text::casefold;
use crate::writes::transactions::sha256_hex;

/// `DEFAULT_CATEGORIES`: always offered, before the ledger's own.
pub const DEFAULT_CATEGORIES: [&str; 15] = [
    "Groceries",
    "Dining",
    "Transport",
    "Shopping",
    "Subscriptions",
    "Travel",
    "Housing",
    "Utilities",
    "Health",
    "Education",
    "Entertainment",
    "Fees",
    "Income",
    "Investments",
    "Cash",
];
/// `MAX_CATEGORIES`.
pub const MAX_CATEGORIES: usize = 256;
/// `MAX_HISTORY_EXAMPLES`.
pub const MAX_HISTORY_EXAMPLES: usize = 48;

/// `CategoryHistoryContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    /// The approved vocabulary, defaults first.
    pub categories: Vec<String>,
    /// Normalised merchant → its only category (the baseline rules).
    pub rules: BTreeMap<String, String>,
    /// Category-diverse examples for prompts without retrieval.
    pub examples: Vec<(String, String)>,
    /// Every unambiguous merchant, most used first.
    pub retrieval: Vec<(String, String)>,
    /// `firefly-history-v1-<fingerprint>`: the vocabulary and policy version.
    pub version: String,
}

impl History {
    pub fn contains(&self, category: &str) -> bool {
        self.categories.iter().any(|c| c == category)
    }

    /// `baseline_categorise`: the ledger's category for this exact merchant.
    pub fn baseline(&self, normalized: &str) -> Option<&str> {
        self.rules.get(normalized).map(String::as_str)
    }
}

/// `build_category_history_context`.
pub fn build(transactions: &[Tx]) -> History {
    let mut category_counts: HashMap<&str, usize> = HashMap::new();
    // Merchants in first-seen order, each with its category counts.
    let mut merchants: Vec<(String, Vec<(&str, usize)>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for t in transactions {
        let Some(category) = t.category.as_deref() else {
            continue;
        };
        if t.kind == Kind::Transfer || !has_merchant_description(t.description.as_deref()) {
            continue;
        }
        let merchant = normalize_merchant(t.description.as_deref().unwrap_or(""));
        if merchant.is_empty() {
            continue;
        }
        *category_counts.entry(category).or_default() += 1;
        let slot = *index.entry(merchant.clone()).or_insert_with(|| {
            merchants.push((merchant, Vec::new()));
            merchants.len() - 1
        });
        let counts = &mut merchants[slot].1;
        match counts.iter_mut().find(|(c, _)| *c == category) {
            Some((_, n)) => *n += 1,
            None => counts.push((category, 1)),
        }
    }

    let mut ordered: Vec<(&str, usize)> = category_counts
        .into_iter()
        .filter(|(c, _)| !DEFAULT_CATEGORIES.contains(c))
        .collect();
    ordered.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| casefold(a.0).cmp(&casefold(b.0)))
    });
    let categories: Vec<String> = DEFAULT_CATEGORIES
        .iter()
        .copied()
        .chain(ordered.into_iter().map(|(c, _)| c))
        .take(MAX_CATEGORIES)
        .map(str::to_owned)
        .collect();

    let unambiguous: Vec<(String, String, usize)> = merchants
        .into_iter()
        .filter_map(|(merchant, counts)| match counts.as_slice() {
            [(category, n)] if categories.iter().any(|c| c == category) => {
                Some((merchant, (*category).to_owned(), *n))
            }
            _ => None,
        })
        .collect();
    let rules: BTreeMap<String, String> = unambiguous
        .iter()
        .map(|(m, c, _)| (m.clone(), c.clone()))
        .collect();

    // Round-robin over categories (case-insensitive order), most used
    // merchant first, so the prompt sees every category.
    let mut by_category: Vec<(String, Vec<(String, usize)>)> = Vec::new();
    for (merchant, category, count) in &unambiguous {
        match by_category.iter_mut().find(|(c, _)| c == category) {
            Some((_, list)) => list.push((merchant.clone(), *count)),
            None => by_category.push((category.clone(), vec![(merchant.clone(), *count)])),
        }
    }
    for (_, list) in &mut by_category {
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        list.reverse(); // pop from the end
    }
    by_category.sort_by_key(|a| casefold(&a.0));
    let mut examples = Vec::new();
    let mut active: Vec<usize> = (0..by_category.len()).collect();
    'rounds: while !active.is_empty() && examples.len() < MAX_HISTORY_EXAMPLES {
        let mut remaining = Vec::new();
        for i in active {
            let (category, list) = &mut by_category[i];
            if let Some((merchant, _)) = list.pop() {
                examples.push((merchant, category.clone()));
            }
            if !list.is_empty() {
                remaining.push(i);
            }
            if examples.len() == MAX_HISTORY_EXAMPLES {
                break 'rounds;
            }
        }
        active = remaining;
    }

    let mut sorted_rules: Vec<&(String, String, usize)> = unambiguous.iter().collect();
    sorted_rules.sort();
    let fingerprint = sha256_hex(
        &J::obj([
            ("categories", J::strs(&categories)),
            (
                "rules",
                J::Arr(
                    sorted_rules
                        .iter()
                        .map(|(m, c, _)| J::strs([m, c]))
                        .collect(),
                ),
            ),
        ])
        .dumps_sorted_compact(),
    );
    let mut retrieval = unambiguous;
    retrieval.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| casefold(&a.1).cmp(&casefold(&b.1)))
    });
    History {
        categories,
        rules,
        examples,
        retrieval: retrieval.into_iter().map(|(m, c, _)| (m, c)).collect(),
        version: format!("firefly-history-v1-{}", &fingerprint[..12]),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Tx;
    use finance_types::report::Currency;
    use rust_decimal::Decimal;

    pub fn tx(id: i64, description: &str, category: Option<&str>, kind: Kind) -> Tx {
        Tx {
            transaction_id: format!("firefly:{id}"),
            ledger_id: id,
            account_id: "firefly:1".into(),
            counterparty_account_id: None,
            kind,
            amount: Decimal::ONE,
            currency: Currency {
                code: "EUR".into(),
                minor_unit: 2,
            },
            occurred_at: jiff::civil::date(2026, 10, 1)
                .at(12, 0, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .unwrap()
                .checked_add(jiff::SignedDuration::from_mins(id))
                .unwrap(),
            purchased_on: None,
            description: Some(description.into()),
            counterparty_name: None,
            group_id: None,
            category: category.map(str::to_owned),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    #[test]
    fn builds_vocabulary_rules_and_examples() {
        let out = Kind::Outflow;
        let txs = vec![
            tx(1, "Albert Heijn 1001", Some("Groceries"), out),
            tx(2, "Albert Heijn 1002", Some("Groceries"), out),
            tx(3, "Cafe Noir", Some("Coffee"), out),
            tx(4, "Mixed Shop", Some("Shopping"), out),
            tx(5, "Mixed Shop", Some("Gifts"), out),
            tx(6, "Own account", Some("Savings"), Kind::Transfer),
            tx(7, "unknown", Some("Coffee"), out),
            tx(8, "Uncategorised", None, out),
        ];
        let h = build(&txs);
        assert_eq!(&h.categories[..15], &DEFAULT_CATEGORIES.map(str::to_owned));
        assert_eq!(&h.categories[15..], ["Coffee", "Gifts"]);
        assert_eq!(h.baseline("albert heijn number"), Some("Groceries"));
        assert_eq!(h.baseline("mixed shop"), None);
        assert_eq!(
            h.retrieval,
            vec![
                ("albert heijn number".to_owned(), "Groceries".to_owned()),
                ("cafe noir".to_owned(), "Coffee".to_owned()),
            ]
        );
        assert_eq!(h.examples.len(), 2);
        assert_eq!(h.examples[0].1, "Coffee");
        assert!(h.version.starts_with("firefly-history-v1-"));
        assert_eq!(h.version.len(), "firefly-history-v1-".len() + 12);
    }
}
