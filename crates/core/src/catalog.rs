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
    let overlaps = meaning_overlaps(&names, &counts, &similar);
    CategoryCatalog {
        overlaps,
        group_conflicts: Vec::new(),
        subscription_overlaps: Vec::new(),
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

/// Kinds of spending that often get several names, and whether they are
/// usually a subscription (where two names may mean paying twice).
const FAMILIES: &[(&str, bool, &[&str])] = &[
    (
        "eating out",
        false,
        &[
            "eating out",
            "dining",
            "dining out",
            "restaurant",
            "restaurants",
            "food out",
            "eat out",
            "food & drinks out",
        ],
    ),
    (
        "coffee",
        false,
        &["coffee", "cafe", "café", "coffee shops", "coffee shop"],
    ),
    (
        "groceries",
        false,
        &["groceries", "grocery", "supermarket", "supermarkets"],
    ),
    ("gifts", false, &["gift", "gifts", "presents", "present"]),
    ("taxi", false, &["taxi", "taxis", "uber", "bolt"]),
    (
        "cinema",
        true,
        &[
            "cinema",
            "cinemas",
            "movies",
            "films",
            "vue",
            "pathé",
            "pathe",
            "kinepolis",
            "cineville",
            "filmhuis",
        ],
    ),
    (
        "video streaming",
        true,
        &[
            "streaming",
            "netflix",
            "disney+",
            "disney plus",
            "hbo max",
            "videoland",
            "prime video",
            "skyshowtime",
        ],
    ),
    (
        "music",
        true,
        &[
            "music",
            "spotify",
            "apple music",
            "deezer",
            "tidal",
            "youtube music",
        ],
    ),
    (
        "gym",
        true,
        &[
            "gym",
            "fitness",
            "basic-fit",
            "basic fit",
            "sportcity",
            "sportschool",
        ],
    ),
    (
        "cloud storage",
        true,
        &["icloud", "google one", "dropbox", "onedrive"],
    ),
];

fn family_of(name: &str) -> Option<(&'static str, bool)> {
    let key = category_key(name);
    FAMILIES
        .iter()
        .find(|(_, _, names)| names.contains(&key.as_str()))
        .map(|(kind, subscription, _)| (*kind, *subscription))
}

/// Categories of one family: each less-used one into the most-used one,
/// unless the spelling pass already pairs it.
fn meaning_overlaps(
    names: &[&str],
    counts: &HashMap<&str, u64>,
    similar: &[CategoryPair],
) -> Vec<CategoryPair> {
    let paired: HashSet<&str> = similar.iter().map(|p| p.source.as_str()).collect();
    let mut kept: HashMap<&str, &str> = HashMap::new();
    let mut out = Vec::new();
    // `names` is most-used first, so the first of each family is kept.
    for name in names {
        let Some((kind, subscription)) = family_of(name) else {
            continue;
        };
        let Some(target) = kept.get(kind) else {
            kept.insert(kind, name);
            continue;
        };
        if paired.contains(name) {
            continue;
        }
        out.push(CategoryPair {
            source: (*name).to_owned(),
            target: (*target).to_owned(),
            count: counts[name],
            reason: if subscription {
                format!(
                    "Both are {kind}: merge them, and check you are not paying for two {kind} subscriptions"
                )
            } else {
                format!("Both mean {kind}: one name keeps one budget line and one trend")
            },
        });
    }
    out
}

/// Names claimed twice: a category or spending group in two budget lines
/// (only the first counts it), or a category whose name is also a spending
/// group it does not belong to.
pub fn group_conflicts(
    plan_groups: &[finance_types::gateway::BudgetGroup],
    mappings: &[finance_types::gateway::GroupMapping],
) -> Vec<finance_types::report::catalog::GroupConflict> {
    use finance_types::report::catalog::GroupConflict;
    let mut claims: BTreeMap<(String, &str), Vec<String>> = BTreeMap::new();
    for g in plan_groups {
        for c in &g.categories {
            claims
                .entry((category_key(c), "category"))
                .or_default()
                .push(g.name.clone());
        }
        for s in &g.source_groups {
            claims
                .entry((category_key(s), "group"))
                .or_default()
                .push(g.name.clone());
        }
    }
    let display = |key: &str| {
        plan_groups
            .iter()
            .flat_map(|g| g.categories.iter().chain(&g.source_groups))
            .find(|n| category_key(n) == key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    };
    let mut out: Vec<GroupConflict> = claims
        .into_iter()
        .filter_map(|((key, what), mut groups)| {
            groups.dedup();
            (groups.len() > 1).then(|| {
                let name = display(&key);
                GroupConflict {
                    note: format!(
                        "The {what} “{name}” is in {} budget lines; only “{}” counts it. Keep it in one.",
                        groups.len(),
                        groups[0]
                    ),
                    name,
                    groups,
                }
            })
        })
        .collect();
    let group_names: HashMap<String, &str> = mappings
        .iter()
        .filter_map(|m| m.group.as_deref())
        .map(|g| (category_key(g), g))
        .collect();
    for m in mappings {
        let (Some(own), Some(group)) = (group_names.get(&category_key(&m.category)), &m.group)
        else {
            continue;
        };
        if category_key(group) != category_key(own) {
            out.push(GroupConflict {
                name: m.category.clone(),
                groups: vec![group.clone(), (*own).to_owned()],
                note: format!(
                    "“{}” is a category in {group} and also a spending group of its own. Move the category into “{own}”, or rename one of them.",
                    m.category
                ),
            });
        }
    }
    out
}

/// Bills of one subscription family (by a family name in the bill's name).
pub fn subscription_overlaps(
    bills: &[finance_types::report::subscriptions::SubscriptionRow],
) -> Vec<finance_types::report::catalog::SubscriptionOverlap> {
    let mut found: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for bill in bills {
        let tokens: Vec<String> = casefold(&bill.name)
            .split(|c: char| !(c.is_alphanumeric() || c == '+' || c == '-'))
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        let padded = format!(" {} ", tokens.join(" "));
        let family = FAMILIES
            .iter()
            .filter(|(_, sub, _)| *sub)
            .find(|(_, _, names)| names.iter().any(|n| padded.contains(&format!(" {n} "))));
        if let Some((kind, _, _)) = family {
            let list = found.entry(kind).or_default();
            if !list.contains(&bill.name) {
                list.push(bill.name.clone());
            }
        }
    }
    found
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(
            |(kind, names)| finance_types::report::catalog::SubscriptionOverlap {
                kind: kind.to_owned(),
                names,
            },
        )
        .collect()
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
        assert!(catalog.overlaps.is_empty());
    }

    #[test]
    fn same_meaning_names_merge_into_the_most_used() {
        let base: Tx = crate::reporting::tests::tx(
            "x",
            Kind::Outflow,
            "1",
            "EUR",
            "2026-09-02T14:00:00+02:00[Europe/Amsterdam]",
        );
        let rows: Vec<Tx> = [
            "Eating out",
            "Eating out",
            "Eating out",
            "Dining",
            "Restaurant",
            "Restaurant",
            "Vue",
            "Pathé",
        ]
        .iter()
        .enumerate()
        .map(|(i, c)| Tx {
            transaction_id: i.to_string(),
            category: Some((*c).into()),
            ..base.clone()
        })
        .collect();
        let catalog = category_catalog(&rows);
        let pairs: Vec<(&str, &str)> = catalog
            .overlaps
            .iter()
            .map(|p| (p.source.as_str(), p.target.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("Restaurant", "Eating out"),
                ("Dining", "Eating out"),
                ("Vue", "Pathé")
            ]
        );
        assert!(catalog.overlaps[2].reason.contains("paying for two cinema"));
    }

    #[test]
    fn a_name_in_two_budget_lines_is_a_conflict() {
        use finance_types::gateway::{BudgetGroup, GroupMapping};
        let line = |name: &str, categories: &[&str]| BudgetGroup {
            id: name.to_lowercase(),
            name: name.into(),
            amount: Decimal::ONE,
            source_groups: Vec::new(),
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance: None,
            floor: None,
        };
        let plan = [
            line("Presents", &["Gifts"]),
            line("Birthdays", &["gifts", "Cake"]),
        ];
        let mapping = |category: &str, group: &str| GroupMapping {
            category: category.into(),
            group: Some(group.into()),
            revision: 1,
        };
        let found = group_conflicts(
            &plan,
            &[mapping("Gifts", "Shopping"), mapping("Flowers", "Gifts")],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].groups, vec!["Presents", "Birthdays"]);
        assert!(found[0].note.contains("only “Presents” counts it"));
        assert_eq!(found[1].groups, vec!["Shopping", "Gifts"]);
    }

    #[test]
    fn two_cinema_bills_overlap() {
        let row = |name: &str| {
            finance_types::report::subscriptions::SubscriptionRow {
            name: name.into(),
            ..serde_json::from_value(serde_json::json!({
                "bill_id": "1", "name": "", "currency": {"code": "EUR", "minor_unit": 2}, "amount_min": "1",
                "amount_max": "1", "frequency": "monthly", "expected_dates": [],
                "last_matched_on": null, "native_payment_count": 0, "category_payment_count": 0,
                "recorded_spending": "0", "last_recorded_on": null, "category_name_match": false
            }))
            .unwrap()
        }
        };
        let found = subscription_overlaps(&[
            row("Pathé Unlimited"),
            row("Vue cinema pass"),
            row("Spotify"),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "cinema");
        assert_eq!(found[0].names, vec!["Pathé Unlimited", "Vue cinema pass"]);
    }
}
