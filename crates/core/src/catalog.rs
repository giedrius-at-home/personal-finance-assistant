//! Category catalog and bulk-payment summaries (Python category_catalog.py, dashboard._bulk_payment_summaries).

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::gateway::SavedDetails;
use finance_types::report::Currency;
use finance_types::report::catalog::{
    BulkPaymentSummary, CategoryCatalog, CategoryEntry, CategoryPair,
};
use rust_decimal::Decimal;

use crate::model::{Kind, Tx};
use crate::text::casefold;

/// Review policy (`config/review-policy.json` → `categories`).
pub const SIMILARITY_THRESHOLD: f64 = 0.9;
pub const SIMILARITY_MIN_LENGTH: usize = 6;
pub const SIMILAR_PAIRS_LIMIT: usize = 30;

/// Case- and spacing-insensitive identity of a label. Python also applied
/// NFKC normalisation; the labels here are plain text, so it is left out.
pub fn category_key(label: &str) -> String {
    casefold(label)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Python `difflib.SequenceMatcher(None, a, b).ratio()` (no junk; labels are
/// far shorter than the 200 characters where its auto-junk starts).
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, c) in b.iter().enumerate() {
        b2j.entry(*c).or_default().push(j);
    }
    let longest = |alo: usize, ahi: usize, blo: usize, bhi: usize| {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, c) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut next: HashMap<usize, usize> = HashMap::new();
            for &j in b2j.get(c).map(Vec::as_slice).unwrap_or(&[]) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j
                    .checked_sub(1)
                    .and_then(|p| j2len.get(&p))
                    .copied()
                    .unwrap_or(0)
                    + 1;
                next.insert(j, k);
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = next;
        }
        (besti, bestj, bestsize)
    };
    let mut matched = 0;
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(alo, ahi, blo, bhi);
        if k == 0 {
            continue;
        }
        matched += k;
        if alo < i && blo < j {
            queue.push((alo, i, blo, j));
        }
        if i + k < ahi && j + k < bhi {
            queue.push((i + k, ahi, j + k, bhi));
        }
    }
    2.0 * matched as f64 / total as f64
}

/// Categories in use with their counts, and less-used spellings that look
/// like a more-used one (at most [`SIMILAR_PAIRS_LIMIT`]).
pub fn category_catalog(transactions: &[Tx]) -> CategoryCatalog {
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for c in transactions.iter().filter_map(|t| t.category.as_deref()) {
        if !c.is_empty() {
            *counts.entry(c).or_default() += 1;
        }
    }
    let mut names: Vec<&str> = counts.keys().copied().collect();
    names.sort_by(|a, b| {
        counts[b]
            .cmp(&counts[a])
            .then_with(|| casefold(a).cmp(&casefold(b)))
            .then_with(|| a.cmp(b))
    });
    let mut similar = Vec::new();
    for (index, source) in names.iter().enumerate() {
        let key = category_key(source);
        for target in &names[..index] {
            let other = category_key(target);
            let exact = key == other;
            let close = key.chars().count().min(other.chars().count()) >= SIMILARITY_MIN_LENGTH
                && similarity(&key, &other) >= SIMILARITY_THRESHOLD;
            if exact || close {
                similar.push(CategoryPair {
                    source: (*source).to_owned(),
                    target: (*target).to_owned(),
                    count: counts[source],
                    reason: if exact {
                        "Same spelling, different case/spacing"
                    } else {
                        "Similar spelling — check they mean the same thing"
                    }
                    .into(),
                });
                break;
            }
        }
        if similar.len() == SIMILAR_PAIRS_LIMIT {
            break;
        }
    }
    // A stable sort by case-folded name over the count order.
    let mut entries = names.clone();
    entries.sort_by_key(|n| casefold(n));
    CategoryCatalog {
        entries: entries
            .into_iter()
            .map(|n| CategoryEntry {
                name: n.to_owned(),
                count: counts[n],
            })
            .collect(),
        similar,
    }
}

/// Owner-named bulk payments: component purchases against settling payments,
/// per name (case-insensitive) and currency.
pub fn bulk_payment_summaries(
    transactions: &[Tx],
    details: &HashMap<String, SavedDetails>,
) -> Vec<BulkPaymentSummary> {
    let mut totals: BTreeMap<(String, String), BulkPaymentSummary> = BTreeMap::new();
    for t in transactions.iter().filter(|t| t.kind == Kind::Outflow) {
        let Some(saved) = details.get(&t.source_key()) else {
            continue;
        };
        let values = &saved.values;
        if values.bulk_payment_role.is_empty() {
            continue;
        }
        let key = (casefold(&values.bulk_payment_name), t.currency.code.clone());
        let row = totals.entry(key).or_insert_with(|| BulkPaymentSummary {
            name: values.bulk_payment_name.clone(),
            currency: Currency::clone(&t.currency),
            component_total: Decimal::ZERO,
            settlement_total: Decimal::ZERO,
            outstanding_amount: Decimal::ZERO,
            component_count: 0,
            settlement_count: 0,
        });
        if values.bulk_payment_role == "component" {
            row.component_total += t.amount;
            row.component_count += 1;
        } else {
            row.settlement_total += t.amount;
            row.settlement_count += 1;
        }
        row.outstanding_amount = row.component_total - row.settlement_total;
    }
    totals.into_values().collect()
}

/// Uncategorised, non-transfer transactions the daily review never covered.
pub fn historical_uncategorized_count(
    transactions: &[Tx],
    daily_review_ids: &HashSet<String>,
) -> u64 {
    transactions
        .iter()
        .filter(|t| {
            t.category.is_none()
                && t.kind != Kind::Transfer
                && !daily_review_ids.contains(&t.transaction_id)
        })
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_agrees_with_difflib() {
        // Values from Python's difflib.
        assert!((similarity("abcd", "bcde") - 0.75).abs() < 1e-12);
        assert!((similarity("transport", "transport fees") - 0.782608695652174).abs() < 1e-12);
        assert!((similarity("albert heijn", "albert hein") - 0.9565217391304348).abs() < 1e-12);
    }

    #[test]
    fn prefers_used_spelling_and_keeps_meanings_apart() {
        let base: Tx = crate::reporting::tests::tx(
            "x",
            Kind::Outflow,
            "1",
            "EUR",
            "2026-09-02T14:00:00+02:00[Europe/Amsterdam]",
        );
        let rows: Vec<Tx> = ["Food", "Food", "food", "Transport", "Transport fees"]
            .iter()
            .enumerate()
            .map(|(i, c)| Tx {
                transaction_id: i.to_string(),
                category: Some((*c).into()),
                ..base.clone()
            })
            .collect();
        let catalog = category_catalog(&rows);
        assert_eq!(catalog.similar.len(), 1);
        assert_eq!(catalog.similar[0].source, "food");
        assert_eq!(catalog.similar[0].target, "Food");
        let names: Vec<_> = catalog.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Food", "food", "Transport", "Transport fees"]);
    }
}
