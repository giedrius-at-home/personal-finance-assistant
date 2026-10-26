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
    transactions
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
            }
            t
        })
        .collect()
}
