//! Owner-marked shared spending: a payment for someone else, or money back
//! from them, counts in the chosen category instead of as income.

use std::collections::HashMap;

use finance_types::gateway::SavedDetails;
use jiff::civil::{Date, date};

use crate::model::{Kind, Tx};

/// Marks before this date belong to the old bookkeeping and are ignored.
pub const PERSONAL_REVIEW_START: Date = date(2026, 9, 1);

pub fn apply_personal_spending(
    transactions: Vec<Tx>,
    details: &HashMap<String, SavedDetails>,
) -> Vec<Tx> {
    let mut marked: Vec<Tx> = transactions
        .into_iter()
        .map(|mut t| {
            let Some(saved) = details.get(&t.source_key()) else {
                return t;
            };
            let role = saved.values.personal_expense_role.as_str();
            let allowed = (role == "return" && matches!(t.kind, Kind::Inflow | Kind::Refund))
                || (role == "payment" && t.kind == Kind::Outflow);
            if allowed && t.booked_on() >= PERSONAL_REVIEW_START {
                t.kind = if role == "return" {
                    Kind::Refund
                } else {
                    Kind::Outflow
                };
                let category = saved.values.personal_expense_category.clone();
                t.category = Some(category).filter(|c| !c.is_empty());
                t.subscription_id = None;
                let link = saved.values.personal_expense_link.trim();
                t.repays =
                    (role == "return" && !link.is_empty()).then(|| format!("firefly:{link}"));
            }
            t
        })
        .collect();
    link_returns(&mut marked);
    marked
}

/// A linked money-back counts where its purchase counts: the purchase's
/// category and subscription. A link to a purchase that is not an outflow
/// of the same currency in `transactions` is dropped.
fn link_returns(transactions: &mut [Tx]) {
    let purchases: HashMap<String, (Option<String>, Option<String>, String)> = transactions
        .iter()
        .filter(|t| t.kind == Kind::Outflow)
        .map(|t| {
            (
                t.transaction_id.clone(),
                (
                    t.category.clone(),
                    t.subscription_id.clone(),
                    t.currency.code.clone(),
                ),
            )
        })
        .collect();
    for t in transactions.iter_mut().filter(|t| t.repays.is_some()) {
        match purchases.get(t.repays.as_deref().unwrap_or("")) {
            Some((category, subscription, currency)) if *currency == t.currency.code => {
                if category.is_some() {
                    t.category.clone_from(category);
                }
                t.subscription_id.clone_from(subscription);
            }
            _ => t.repays = None,
        }
    }
}

/// Money paid back per purchase (`firefly:<id>` → total), from the linked
/// money-back in `transactions`.
pub fn paid_back(transactions: &[Tx]) -> HashMap<&str, rust_decimal::Decimal> {
    let mut out: HashMap<&str, rust_decimal::Decimal> = HashMap::new();
    for t in transactions.iter().filter(|t| t.kind == Kind::Refund) {
        if let Some(origin) = t.repays.as_deref() {
            *out.entry(origin).or_default() += t.amount;
        }
    }
    out
}

/// Categories a split bill usually lands in.
const SHARED_CATEGORIES: [&str; 8] = [
    "eating out",
    "restaurant",
    "restaurants",
    "dining",
    "drinks",
    "takeaway",
    "delivery",
    "groceries",
];
/// How far back a money-back looks for its purchase.
pub const LINK_LOOKBACK_DAYS: i64 = 60;

fn words(text: &str) -> std::collections::HashSet<String> {
    crate::text::casefold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !["tikkie", "van", "from", "the", "and", "for"].contains(w))
        .map(str::to_owned)
        .collect()
}

/// True when `part` is an even share of `whole` (half, a third, … an eighth).
fn even_share(part: rust_decimal::Decimal, whole: rust_decimal::Decimal) -> bool {
    use rust_decimal::Decimal;
    (2..=8).any(|n| {
        let share = whole / Decimal::from(n);
        (share - part).abs() <= Decimal::new(5, 2).max(share * Decimal::new(1, 2))
    })
}

/// Purchases a money-back may pay back, best first, at most `limit`: outflows
/// of its currency in the [`LINK_LOOKBACK_DAYS`] before it (or two days
/// after), with at least its amount still unpaid. A shared word with the
/// Tikkie memo, an even share of the bill and a shared-bill category rank a
/// purchase higher; nearer days break ties.
pub fn link_candidates<'t>(
    refund: &Tx,
    transactions: &'t [Tx],
    paid_back: &HashMap<&str, rust_decimal::Decimal>,
    limit: usize,
) -> Vec<&'t Tx> {
    let memo = refund.description.as_deref().unwrap_or("");
    let purpose = crate::ai::text::parse_description(memo).purpose;
    let cue = words(purpose.as_deref().unwrap_or(memo));
    let day = refund.booked_on();
    let mut scored: Vec<(i64, i64, &Tx)> = transactions
        .iter()
        .filter(|t| t.kind == Kind::Outflow && t.currency == refund.currency)
        .filter(|t| t.transaction_id != refund.transaction_id)
        .filter_map(|t| {
            let days = crate::outlook::days_between(t.booked_on(), day);
            if !(-2..=LINK_LOOKBACK_DAYS).contains(&days) {
                return None;
            }
            let repaid = paid_back
                .get(t.transaction_id.as_str())
                .copied()
                .unwrap_or_default();
            // Its own earlier link does not use up room for itself.
            let own = if refund.repays.as_deref() == Some(t.transaction_id.as_str()) {
                refund.amount
            } else {
                rust_decimal::Decimal::ZERO
            };
            if t.amount - repaid + own < refund.amount {
                return None;
            }
            let text = format!(
                "{} {}",
                t.description.as_deref().unwrap_or(""),
                t.category.as_deref().unwrap_or("")
            );
            let shared = i64::try_from(words(&text).intersection(&cue).count()).unwrap_or(0);
            let category = t
                .category
                .as_deref()
                .is_some_and(|c| SHARED_CATEGORIES.contains(&crate::text::casefold(c).as_str()));
            let score = shared * 3
                + if even_share(refund.amount, t.amount) {
                    2
                } else {
                    0
                }
                + i64::from(category);
            Some((score, days.abs(), t))
        })
        .collect();
    scored.sort_by_key(|(score, days, _)| (std::cmp::Reverse(*score), *days));
    scored.into_iter().take(limit).map(|(_, _, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::gateway::OwnerDetails;
    use finance_types::report::Currency;
    use rust_decimal::Decimal;

    fn tx(id: i64, kind: Kind, amount: &str, on: &str, description: &str, category: &str) -> Tx {
        Tx {
            transaction_id: format!("firefly:{id}"),
            ledger_id: id,
            account_id: "firefly:1".into(),
            counterparty_account_id: None,
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: format!("{on}T19:00:00+01:00[Europe/Amsterdam]")
                .parse()
                .unwrap(),
            purchased_on: None,
            description: Some(description.into()),
            counterparty_name: None,
            group_id: None,
            category: Some(category.to_owned()).filter(|c| !c.is_empty()),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
            repays: None,
        }
    }

    fn marked(role: &str, category: &str, link: &str) -> SavedDetails {
        SavedDetails {
            revision: 1,
            values: OwnerDetails {
                personal_expense_role: role.into(),
                personal_expense_category: category.into(),
                personal_expense_link: link.into(),
                ..OwnerDetails::default()
            },
        }
    }

    fn dinner_and_tikkie() -> Vec<Tx> {
        vec![
            tx(
                1,
                Kind::Outflow,
                "30.90",
                "2026-11-14",
                "BEA, Betaalpas Restaurant Fuji",
                "Restaurant",
            ),
            tx(
                2,
                Kind::Outflow,
                "61.20",
                "2026-11-13",
                "AH Amsterdam",
                "Groceries",
            ),
            tx(
                3,
                Kind::Inflow,
                "15.45",
                "2026-11-16",
                "Tikkie ID 0123456, Fuji dinner, Van J Doe",
                "",
            ),
        ]
    }

    #[test]
    fn a_linked_return_counts_in_its_purchase_category() {
        let details = HashMap::from([(source_key(3), marked("return", "Eating out", "1"))]);
        let out = apply_personal_spending(dinner_and_tikkie(), &details);
        let back = &out[2];
        assert_eq!(back.kind, Kind::Refund);
        assert_eq!(back.category.as_deref(), Some("Restaurant"));
        assert_eq!(back.repays.as_deref(), Some("firefly:1"));
        let paid = paid_back(&out);
        assert_eq!(paid.get("firefly:1"), Some(&Decimal::new(1545, 2)));
    }

    #[test]
    fn a_link_to_a_missing_purchase_keeps_the_chosen_category() {
        let details = HashMap::from([(source_key(3), marked("return", "Eating out", "99"))]);
        let out = apply_personal_spending(dinner_and_tikkie(), &details);
        assert_eq!(out[2].category.as_deref(), Some("Eating out"));
        assert_eq!(out[2].repays, None);
    }

    #[test]
    fn the_dinner_ranks_first_for_its_tikkie() {
        let all = dinner_and_tikkie();
        let found = link_candidates(&all[2], &all, &HashMap::new(), 3);
        let ids: Vec<i64> = found.iter().map(|t| t.ledger_id).collect();
        assert_eq!(ids, vec![1, 2]);
        // Once paid back in full, it has no room for another.
        let paid = HashMap::from([("firefly:1", Decimal::new(2000, 2))]);
        let found = link_candidates(&all[2], &all, &paid, 3);
        assert_eq!(
            found.iter().map(|t| t.ledger_id).collect::<Vec<_>>(),
            vec![2]
        );
    }

    fn source_key(id: i64) -> String {
        crate::model::source_key(id)
    }
}
