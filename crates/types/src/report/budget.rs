//! Report section types for `finance_core::budget`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::gateway::BudgetPlan;

/// Spending in one category of a budget row (refunds subtract).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetCategory {
    pub category: Option<String>,
    pub source_group: Option<String>,
    pub spent: Decimal,
    pub count: usize,
}

/// One envelope of a month: planned, spent and what is still set aside.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetRow {
    pub id: String,
    pub name: String,
    pub planned: Decimal,
    pub spent: Decimal,
    pub remaining: Decimal,
    /// Still free after spending and this month's scheduled plans.
    pub reserved: Decimal,
    /// Planned payments still due that this envelope covers.
    pub scheduled: Decimal,
    pub categories: Vec<BudgetCategory>,
    /// Spending in each of the six months ending with this one.
    #[serde(default)]
    pub history: Vec<Decimal>,
    /// Spending per day of the month so far.
    #[serde(default)]
    pub daily_spending: Vec<Decimal>,
    #[serde(default)]
    pub elapsed_days: usize,
    /// `(label, amount)` estimates inside `planned` that are still to be
    /// spent: predicted bills, or the unplanned spending expected for the
    /// rest of the month.
    #[serde(default)]
    pub expected: Vec<(String, Decimal)>,
    /// `(event, amount)`: the usual spending of an event's days, handed to
    /// the event and no longer reserved here.
    #[serde(default)]
    pub to_events: Vec<(String, Decimal)>,
}

/// A non-fixed purchase of the month and the envelope it counts in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetPurchase {
    pub id: String,
    pub description: String,
    pub category: Option<String>,
    /// Negative for a refund.
    pub amount: Decimal,
    pub occurred_on: String,
    /// `None` for unplanned spending.
    pub budget_group_id: Option<String>,
}

/// One month's budget in one currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyBudget {
    pub plan: BudgetPlan,
    pub fixed: BudgetRow,
    pub groups: Vec<BudgetRow>,
    pub unplanned: BudgetRow,
    pub extras: BudgetRow,
    /// Newest first.
    /// One row per event with a share or spending in the month: id
    /// `event:<wish id>`, `planned` the month's share, nothing reserved.
    #[serde(default)]
    pub events: Vec<BudgetRow>,
    pub purchases: Vec<BudgetPurchase>,
    /// The six months `history` covers, oldest first.
    pub history_months: Vec<String>,
    pub reserved: Decimal,
    pub planned: Decimal,
    pub spent: Decimal,
}
