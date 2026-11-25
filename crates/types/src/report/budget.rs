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
    /// This week's share of what is left (running month's plan groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week: Option<WeekLimit>,
}

/// What a budget line may spend this week: what was left when the week
/// began, spread over the month's remaining days by how much each usually
/// takes (evenly without history).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekLimit {
    /// Monday (or the month's first day), `YYYY-MM-DD`.
    pub starts: String,
    /// Sunday (or the month's last day).
    pub ends: String,
    pub limit: Decimal,
    pub spent: Decimal,
    /// `limit - spent`; negative when the week is over.
    pub left: Decimal,
    /// The share follows when this line usually spends (weekends, the
    /// start of the month) rather than spreading evenly.
    #[serde(default)]
    pub follows_pattern: bool,
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
    /// Money back the owner linked to this purchase (positive), so its own
    /// cost is `amount - paid_back`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_back: Option<Decimal>,
    /// On a linked money-back: the purchase it pays back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repays: Option<String>,
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
    /// Budget lines spent over in each of the last three or more finished
    /// months: a plan that is never met, with one based on recent spending.
    #[serde(default)]
    pub overruns: Vec<BudgetOverrun>,
    /// One-off payments in most recent months: they are a monthly cost.
    #[serde(default)]
    pub recurring_extras: Option<RecurringExtras>,
}

/// One-off payments (the extras row) that came in month after month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringExtras {
    /// `(YYYY-MM-01, spent)` for the months looked at, oldest first.
    pub months: Vec<(String, Decimal)>,
    /// How many of them had one-offs worth noticing.
    pub months_with_extras: usize,
    /// A monthly amount to set aside: their average, rounded up to ten.
    pub monthly: Decimal,
}

/// A budget line that has gone over its plan month after month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetOverrun {
    pub group_id: String,
    pub name: String,
    /// Finished months in a row over plan, the latest last month.
    pub months_over: usize,
    /// The first month of that run, `YYYY-MM-01`.
    pub since: String,
    /// This month's plan for the line.
    pub planned: Decimal,
    /// Average spending over the run.
    pub average: Decimal,
    /// A plan recent spending can meet: the last three months' average,
    /// rounded up to ten.
    pub suggested: Decimal,
}
