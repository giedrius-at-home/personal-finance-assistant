//! Spending groups and fixed (recurring) spending detection (Python domain/spending_groups.py).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use finance_types::gateway::{GroupMapping, SavedDetails, SubscriptionCategoryRule};
use finance_types::report::Currency;
use finance_types::report::reporting::{
    CurrencySpending, GroupSpending, MonthlySpending, StoreSpending,
};
use jiff::Zoned;
use rust_decimal::Decimal;

use crate::load::Bill;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::reporting::py_div;
use crate::text::casefold;

/// The group bills and subscriptions belong to.
pub const SUBSCRIPTIONS: &str = "Subscriptions & recurring";

/// The built-in groups, in the order the owner sees them; custom groups follow.
pub const DEFAULT_GROUPS: [&str; 13] = [
    SUBSCRIPTIONS,
    "Groceries",
    "Eating out",
    "Home & household",
    "Housing & utilities",
    "Transport",
    "Travel",
    "Entertainment",
    "Shopping",
    "Health & personal care",
    "Education & work",
    "Fees & taxes",
    "Other",
];

/// The reviewed group of `category`, if the owner assigned one.
pub fn group_of<'a>(mappings: &'a [GroupMapping], category: &str) -> Option<&'a str> {
    mappings
        .iter()
        .find(|m| m.category == category)
        .and_then(|m| m.group.as_deref())
}

/// Category → reviewed group, for repeated lookups.
pub fn group_index(mappings: &[GroupMapping]) -> HashMap<&str, Option<&str>> {
    mappings
        .iter()
        .map(|m| (m.category.as_str(), m.group.as_deref()))
        .collect()
}

/// Categories used by spending or refunds (the ones that need a group).
pub fn expense_categories(transactions: &[Tx]) -> BTreeSet<String> {
    transactions
        .iter()
        .filter(|t| matches!(t.kind, Kind::Outflow | Kind::Refund))
        .filter_map(|t| t.category.clone())
        .collect()
}

fn fold_key(text: &str) -> String {
    casefold(text.trim())
}

/// Case-folded categories the budget treats as fixed: bill names (active or
/// not), categories reviewed into the subscriptions group, and categories a
/// bill's category rule claims.
pub fn fixed_category_names(
    bills: &[Bill],
    mappings: &[GroupMapping],
    rules: &[SubscriptionCategoryRule],
) -> HashSet<String> {
    bills
        .iter()
        .map(|b| fold_key(&b.name))
        .chain(
            mappings
                .iter()
                .filter(|m| m.group.as_deref() == Some(SUBSCRIPTIONS))
                .map(|m| fold_key(&m.category)),
        )
        .chain(
            rules
                .iter()
                .flat_map(|r| r.categories.iter().map(|c| fold_key(c))),
        )
        .collect()
}

/// Case-folded categories the owner has predicted like bills; the budget
/// counts their payments as fixed spending.
pub fn predicted_category_names(categories: &[String]) -> impl Iterator<Item = String> + '_ {
    categories.iter().map(|c| fold_key(c))
}

fn in_fixed_category(t: &Tx, fixed_categories: &HashSet<String>) -> bool {
    t.category
        .as_deref()
        .is_some_and(|c| fixed_categories.contains(&fold_key(c)))
}

/// Transactions the budget counts as fixed: bill payments and fixed categories
/// (Python `budget_inputs.budget_fixed_ids`).
pub fn fixed_transaction_ids(
    transactions: &[Tx],
    fixed_categories: &HashSet<String>,
) -> HashSet<String> {
    transactions
        .iter()
        .filter(|t| t.subscription_id.is_some() || in_fixed_category(t, fixed_categories))
        .map(|t| t.transaction_id.clone())
        .collect()
}

/// The bank's own recurring-collection marker; merchant names are never guessed.
pub fn is_recurring_debit(description: Option<&str>) -> bool {
    let text = casefold(description.unwrap_or(""));
    [
        "incasso algemeen doorlopend",
        "recurring direct debit",
        "recurring debit",
        "lastschrift dauer",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// Transactions that are not variable spending: bills, uncategorised rows,
/// bank recurring debits and fixed categories. The dashboard leaves them out
/// of the variable trends and the forecasts (Python dashboard `fixed_ids`).
pub fn non_variable_ids(
    transactions: &[Tx],
    fixed_categories: &HashSet<String>,
) -> HashSet<String> {
    transactions
        .iter()
        .filter(|t| {
            t.subscription_id.is_some()
                || t.category.is_none()
                || is_recurring_debit(t.description.as_deref())
                || in_fixed_category(t, fixed_categories)
        })
        .map(|t| t.transaction_id.clone())
        .collect()
}

/// Payments the owner marked as settling a bulk payment: the components
/// already count as spending, so the settlement must not.
pub fn settlement_ids(
    transactions: &[Tx],
    details: &HashMap<String, SavedDetails>,
) -> HashSet<String> {
    transactions
        .iter()
        .filter(|t| {
            details
                .get(&t.source_key())
                .is_some_and(|d| d.values.bulk_payment_role == "settlement")
        })
        .map(|t| t.transaction_id.clone())
        .collect()
}

/// `transactions` without bulk-payment settlements. The dashboard then nets
/// linked reimbursements (planned payments) before computing spending.
pub fn without_settlements(
    transactions: &[Tx],
    details: &HashMap<String, SavedDetails>,
) -> Vec<Tx> {
    let settled = settlement_ids(transactions, details);
    transactions
        .iter()
        .filter(|t| !settled.contains(&t.transaction_id))
        .cloned()
        .collect()
}

/// Outflows count, refunds subtract.
pub fn net(rows: &[&Tx]) -> Decimal {
    rows.iter()
        .map(|t| {
            if t.kind == Kind::Outflow {
                t.amount
            } else {
                -t.amount
            }
        })
        .sum()
}

fn gross(rows: &[&Tx]) -> Decimal {
    rows.iter()
        .filter(|t| t.kind == Kind::Outflow)
        .map(|t| t.amount)
        .sum()
}

/// This calendar month through today against the same days of last month,
/// by reviewed group and category. Dates are local to `now`'s time zone.
pub fn monthly_spending(
    transactions: &[Tx],
    mappings: &[GroupMapping],
    now: &Zoned,
) -> MonthlySpending {
    let zone = now.time_zone().clone();
    let today = now.date();
    let start = today.first_of_month();
    let previous_start = shift_month(start, -1);
    let day = today.day().min(previous_start.days_in_month());
    let comparison_through = today.with().day(day).build().unwrap_or(today);
    let previous_through = previous_start
        .with()
        .day(day)
        .build()
        .unwrap_or(previous_start);
    let reviewed: HashMap<&str, &GroupMapping> =
        mappings.iter().map(|m| (m.category.as_str(), m)).collect();
    let local = |t: &Tx| t.occurred_at.with_time_zone(zone.clone()).date();

    let rows: Vec<&Tx> = transactions
        .iter()
        .filter(|t| t.occurred_at.timestamp() <= now.timestamp())
        .collect();
    #[derive(Default)]
    struct Buckets<'a> {
        current: Vec<&'a Tx>,
        previous: Vec<&'a Tx>,
        transfers: u64,
    }
    let mut by_currency: BTreeMap<Currency, Buckets<'_>> = BTreeMap::new();
    for &t in &rows {
        let on = local(t);
        let current = start <= on && on <= today;
        if current && t.kind == Kind::Transfer {
            by_currency.entry(t.currency.clone()).or_default().transfers += 1;
        }
        if !matches!(t.kind, Kind::Outflow | Kind::Refund) {
            continue;
        }
        if current {
            by_currency
                .entry(t.currency.clone())
                .or_default()
                .current
                .push(t);
        } else if previous_start <= on && on <= previous_through {
            by_currency
                .entry(t.currency.clone())
                .or_default()
                .previous
                .push(t);
        }
    }

    let currencies = by_currency
        .into_iter()
        .map(|(currency, b)| {
            let total_gross = gross(&b.current);
            let mut stores: BTreeMap<Option<&str>, (Vec<&Tx>, Vec<&Tx>)> = BTreeMap::new();
            for &t in &b.current {
                stores.entry(t.category.as_deref()).or_default().0.push(t);
            }
            for &t in &b.previous {
                stores.entry(t.category.as_deref()).or_default().1.push(t);
            }
            let has_prior = !b.previous.is_empty();
            let mut groups: BTreeMap<Option<String>, Vec<StoreSpending>> = BTreeMap::new();
            for (category, (items, prior)) in stores {
                let store_gross = gross(&items);
                let spending = net(&items);
                let group = category
                    .and_then(|c| reviewed.get(c))
                    .and_then(|m| m.group.clone());
                groups.entry(group).or_default().push(StoreSpending {
                    category: category.map(str::to_owned),
                    spending,
                    gross: store_gross,
                    refunds: store_gross - spending,
                    transaction_count: items.len() as u64,
                    previous_spending: has_prior.then(|| net(&prior)),
                });
            }
            let mut group_rows: Vec<GroupSpending> = groups
                .into_iter()
                .map(|(group, mut stores)| {
                    stores.sort_by(|a, b| {
                        b.gross.cmp(&a.gross).then_with(|| {
                            a.category
                                .as_deref()
                                .unwrap_or("")
                                .cmp(b.category.as_deref().unwrap_or(""))
                        })
                    });
                    let group_gross: Decimal = stores.iter().map(|s| s.gross).sum();
                    GroupSpending {
                        group,
                        spending: stores.iter().map(|s| s.spending).sum(),
                        share_percent: if total_gross.is_zero() {
                            Decimal::ZERO
                        } else {
                            py_div(group_gross * Decimal::from(100), total_gross)
                        },
                        stores,
                    }
                })
                .collect();
            group_rows.sort_by(|a, b| {
                b.share_percent.cmp(&a.share_percent).then_with(|| {
                    a.group
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.group.as_deref().unwrap_or(""))
                })
            });
            let spending = net(&b.current);
            let compared: Vec<&Tx> = b
                .current
                .iter()
                .copied()
                .filter(|t| local(t) <= comparison_through)
                .collect();
            CurrencySpending {
                currency,
                spending,
                gross: total_gross,
                refunds: total_gross - spending,
                comparison_spending: net(&compared),
                previous_spending: has_prior.then(|| net(&b.previous)),
                transfer_count: b.transfers,
                groups: group_rows,
            }
        })
        .collect();

    let owned: Vec<Tx> = rows.iter().map(|t| (*t).clone()).collect();
    let mut categories: Vec<String> = expense_categories(&owned).into_iter().collect();
    categories.sort_by(|a, b| casefold(a).cmp(&casefold(b)).then_with(|| a.cmp(b)));
    MonthlySpending {
        start: start.to_string(),
        through: today.to_string(),
        comparison_through: comparison_through.to_string(),
        previous_start: previous_start.to_string(),
        previous_through: previous_through.to_string(),
        currencies,
        mappings: categories
            .into_iter()
            .map(|category| {
                reviewed
                    .get(category.as_str())
                    .map(|m| (*m).clone())
                    .unwrap_or(GroupMapping {
                        category,
                        group: None,
                        revision: 0,
                    })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: &str, kind: Kind, amount: &str, category: Option<&str>, at: &str) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 0,
            account_id: "firefly:1".into(),
            counterparty_account_id: (kind == Kind::Transfer).then(|| "firefly:2".into()),
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: at.parse().unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: category.map(str::to_owned),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn mapping(category: &str, group: Option<&str>) -> GroupMapping {
        GroupMapping {
            category: category.into(),
            group: group.map(str::to_owned),
            revision: 1,
        }
    }

    #[test]
    fn groups_stores_and_compares_same_days() {
        let rows = vec![
            tx(
                "1",
                Kind::Outflow,
                "40",
                Some("Albert Heijn"),
                "2026-10-03T12:00:00+02:00[Europe/Amsterdam]",
            ),
            tx(
                "2",
                Kind::Outflow,
                "10",
                Some("Jumbo"),
                "2026-10-04T12:00:00+02:00[Europe/Amsterdam]",
            ),
            tx(
                "3",
                Kind::Refund,
                "5",
                Some("Jumbo"),
                "2026-10-05T12:00:00+02:00[Europe/Amsterdam]",
            ),
            tx(
                "4",
                Kind::Outflow,
                "40",
                Some("Rent"),
                "2026-10-01T09:00:00+02:00[Europe/Amsterdam]",
            ),
            tx(
                "5",
                Kind::Outflow,
                "30",
                Some("Albert Heijn"),
                "2026-09-03T12:00:00+02:00[Europe/Amsterdam]",
            ),
            // After the comparison window of last month.
            tx(
                "6",
                Kind::Outflow,
                "99",
                Some("Albert Heijn"),
                "2026-09-20T12:00:00+02:00[Europe/Amsterdam]",
            ),
            tx(
                "7",
                Kind::Transfer,
                "100",
                None,
                "2026-10-02T12:00:00+02:00[Europe/Amsterdam]",
            ),
            // In the future: ignored.
            tx(
                "8",
                Kind::Outflow,
                "70",
                Some("Rent"),
                "2026-10-11T12:00:00+02:00[Europe/Amsterdam]",
            ),
        ];
        let mappings = vec![
            mapping("Albert Heijn", Some("Groceries")),
            mapping("Jumbo", Some("Groceries")),
        ];
        let now: Zoned = "2026-10-10T12:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        let report = monthly_spending(&rows, &mappings, &now);
        assert_eq!(report.previous_through, "2026-09-10");
        let eur = &report.currencies[0];
        assert_eq!(eur.spending, Decimal::from(85));
        assert_eq!(eur.gross, Decimal::from(90));
        assert_eq!(eur.refunds, Decimal::from(5));
        assert_eq!(eur.previous_spending, Some(Decimal::from(30)));
        assert_eq!(eur.transfer_count, 1);
        let groceries = &eur.groups[0];
        assert_eq!(groceries.group.as_deref(), Some("Groceries"));
        assert_eq!(
            groceries.share_percent.to_string(),
            "55.55555555555555555555555556"
        );
        assert_eq!(
            groceries.stores[0].category.as_deref(),
            Some("Albert Heijn")
        );
        assert_eq!(groceries.stores[1].previous_spending, Some(Decimal::ZERO));
        assert_eq!(eur.groups[1].group, None);
        let names: Vec<_> = report
            .mappings
            .iter()
            .map(|m| (m.category.as_str(), m.revision))
            .collect();
        assert_eq!(names, vec![("Albert Heijn", 1), ("Jumbo", 1), ("Rent", 0)]);
    }

    #[test]
    fn fixed_spending() {
        let bill = Bill {
            id: "1".into(),
            name: " Rent ".into(),
            currency: Currency::eur(),
            amount_min: Decimal::ONE,
            amount_max: Decimal::ONE,
            start: jiff::civil::date(2026, 1, 1),
            end: None,
            repeat_freq: "monthly".into(),
            skip: 0,
            active: false,
        };
        let fixed = fixed_category_names(
            &[bill],
            &[
                mapping("Netflix", Some(SUBSCRIPTIONS)),
                mapping("Jumbo", Some("Groceries")),
            ],
            &[],
        );
        assert!(fixed.contains("rent") && fixed.contains("netflix") && !fixed.contains("jumbo"));
        let at = "2026-10-01T09:00:00+02:00[Europe/Amsterdam]";
        let mut debit = tx("d", Kind::Outflow, "5", Some("Gym"), at);
        debit.description = Some("SEPA Incasso algemeen doorlopend Gym".into());
        let rows = vec![
            tx("r", Kind::Outflow, "5", Some("RENT"), at),
            tx("u", Kind::Outflow, "5", None, at),
            tx("g", Kind::Outflow, "5", Some("Jumbo"), at),
            debit,
        ];
        let budget = fixed_transaction_ids(&rows, &fixed);
        assert_eq!(budget, HashSet::from(["r".to_owned()]));
        let other = non_variable_ids(&rows, &fixed);
        assert_eq!(other.len(), 3);
        assert!(!other.contains("g"));
    }
}
