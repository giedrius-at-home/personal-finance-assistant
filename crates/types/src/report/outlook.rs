//! Report section types for `finance_core::outlook` (the three-month savings
//! outlook) and `finance_core::daily` (the daily spending forecast).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;
use crate::gateway::Direction;

/// The cash balance at one month end (and at the horizon end).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastMonth {
    pub through: String,
    pub planned_balance: Decimal,
    pub expected_balance: Decimal,
    pub lower_balance: Decimal,
    pub upper_balance: Decimal,
}

/// Spending outside every budget group in one calendar month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnplannedMonth {
    pub month: String,
    pub net_spending: Decimal,
    /// The same, through today's day of the month.
    pub same_elapsed_spending: Decimal,
    pub share: Decimal,
    pub purchase_count: usize,
    /// No plan was saved for the month; the current plan stood in.
    pub reconstructed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorDeduction {
    pub reason: String,
    pub points: u32,
}

/// The last three complete days against the three before, the plan and history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreeDaySignal {
    pub start: String,
    pub end: String,
    pub previous_start: String,
    pub previous_end: String,
    pub spending: Decimal,
    pub previous_spending: Decimal,
    pub unplanned: Decimal,
    pub previous_unplanned: Decimal,
    pub allowance: Decimal,
    pub historical_upper: Decimal,
    pub excessive_speed: bool,
    pub accelerating: bool,
    pub unusual_unplanned: bool,
    pub alert: bool,
}

/// Where the cash balance is heading over three calendar months, on plan and
/// with current behavior.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsOutlook {
    pub currency: Currency,
    pub as_of: String,
    pub through: String,
    pub starting_balance: Decimal,
    pub planned_savings: Decimal,
    pub expected_savings: Decimal,
    pub planned_final: Decimal,
    pub expected_final: Decimal,
    pub lower_final: Decimal,
    pub upper_final: Decimal,
    pub expected_unplanned: Decimal,
    pub expected_income: Decimal,
    pub received_refunds: Decimal,
    pub on_track_probability: Option<Decimal>,
    pub behavior_score: Option<u32>,
    pub deductions: Vec<BehaviorDeduction>,
    pub signal: ThreeDaySignal,
    pub months: Vec<ForecastMonth>,
    pub unplanned_history: Vec<UnplannedMonth>,
    pub method: String,
    pub training_days: usize,
    pub training_seconds: Decimal,
    pub validation_windows: usize,
    pub horizon_validation_windows: usize,
    pub baseline_error: Option<Decimal>,
    pub model_error: Option<Decimal>,
    pub interval_coverage: Option<Decimal>,
    pub assumptions: Vec<String>,
    pub data_fresh: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowSource {
    PlannedPayment,
    Bill,
}

/// A known future payment or deposit: a planned payment or an unpaid bill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFlow {
    pub day: String,
    pub name: String,
    pub amount: Decimal,
    pub direction: Direction,
    pub source: FlowSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyPoint {
    pub day: String,
    pub habit_expected: Decimal,
    pub habit_lower: Decimal,
    pub habit_upper: Decimal,
    pub planned: Decimal,
    pub expected_income: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekdayHabit {
    pub weekday: String,
    pub average: Decimal,
    pub spend_day_share: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMonthForecast {
    pub group_id: String,
    pub expected: Decimal,
    /// The forecast part of `expected`: from today to the month's end.
    #[serde(default)]
    pub remaining: Decimal,
}

/// Habit spending for a whole budget month: spent so far plus the forecast
/// rest, and the part each budget group of the month's plan is expected to take.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthHabitForecast {
    pub month: String,
    pub habit: Decimal,
    #[serde(default)]
    pub groups: Vec<GroupMonthForecast>,
    /// The forecast part of `habit`: from today to the month's end.
    #[serde(default)]
    pub remaining: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedCategory {
    pub category: String,
    pub count: usize,
    pub amount: Decimal,
}

/// Outflows the everyday forecast left out of its recent history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastExclusions {
    pub since: String,
    pub limit: Decimal,
    /// Single purchases over `limit`, outside the excluded categories.
    #[serde(default)]
    pub large_count: usize,
    #[serde(default)]
    pub large_amount: Decimal,
    #[serde(default)]
    pub categories: Vec<ExcludedCategory>,
}

/// Habit spending forecast day by day through the budget month's end, with
/// planned payments and unpaid bills as known flows on top.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailySpendingForecast {
    pub currency: Currency,
    pub as_of: String,
    /// `weekday_baseline` here; the Python service could also pick TimesFM.
    pub method: String,
    pub model: String,
    pub habit_spent_this_month: Decimal,
    pub planned_paid_this_month: Decimal,
    pub days: Vec<DailyPoint>,
    pub next_week_habit: Decimal,
    pub next_week_lower: Decimal,
    pub next_week_upper: Decimal,
    pub next_week_planned: Decimal,
    pub month_end_habit: Decimal,
    pub month_end_lower: Decimal,
    pub month_end_upper: Decimal,
    pub month_remaining_planned: Decimal,
    pub month_remaining_income: Decimal,
    pub upcoming: Vec<PlannedFlow>,
    pub habits: Vec<WeekdayHabit>,
    pub training_days: usize,
    pub backtest_windows: usize,
    pub model_error: Option<Decimal>,
    pub planned_model_error: Option<Decimal>,
    pub baseline_error: Option<Decimal>,
    /// The running budget month and the next one, for month-closure planning.
    #[serde(default)]
    pub months: Vec<MonthHabitForecast>,
    #[serde(default)]
    pub excluded: Option<ForecastExclusions>,
}
