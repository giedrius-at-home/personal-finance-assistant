//! Report section types for `finance_core::{trends, limits, forecast}`:
//! spending over time, saved group limits and month-end group forecasts.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;

/// Confirmed spending over the last six calendar months, never predictions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeTrends {
    pub start: String,
    pub through: String,
    pub timezone: String,
    pub currencies: Vec<CurrencyTrends>,
    /// Ledger bookings carry no reliable time of day.
    pub time_of_day_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyTrends {
    pub currency: Currency,
    /// Income booked this calendar month through today.
    pub income: Decimal,
    pub income_transaction_count: usize,
    pub months: Vec<MonthSpending>,
    pub groups: Vec<GroupTrend>,
    pub categories: Vec<CategoryTrend>,
    pub daily: Vec<DailySpending>,
    pub weekdays: Vec<WeekdaySpending>,
    /// Weekdays and weekend, in that order.
    pub day_types: Vec<WeekdaySpending>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthSpending {
    pub start: String,
    pub through: String,
    pub partial_month: bool,
    /// `None` when the month has no spending records at all.
    pub spending: Option<Decimal>,
}

/// One reviewed group's net spending per month (oldest first).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupTrend {
    pub group: Option<String>,
    pub values: Vec<Option<Decimal>>,
    pub total: Decimal,
    /// This month's spending extrapolated over the whole calendar month.
    pub projected_month_end: Decimal,
    /// Median of earlier months with spending, once there are two.
    pub historical_median: Option<Decimal>,
    pub history_month_count: usize,
    /// Purchases this month.
    pub purchase_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryTrend {
    pub category: Option<String>,
    pub values: Vec<Option<Decimal>>,
    pub total: Decimal,
    pub projected_month_end: Decimal,
    pub historical_median: Option<Decimal>,
    pub history_month_count: usize,
    pub purchase_count: usize,
}

/// Cumulative spending by day of month, this month against the last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailySpending {
    pub day: u8,
    pub current_on: String,
    pub previous_on: Option<String>,
    pub current: Option<Decimal>,
    pub previous: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekdaySpending {
    pub label: String,
    pub spending: Option<Decimal>,
    pub gross: Decimal,
    pub refunds: Decimal,
    pub purchase_count: usize,
    pub purchase_days: usize,
    pub average_purchase_day: Option<Decimal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendingRisk {
    OverLimit,
    LikelyOver,
    Watch,
    OnTrack,
}

/// How a saved group limit is doing this month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingNudge {
    pub group: String,
    pub currency: Currency,
    pub risk: SpendingRisk,
    pub limit: Decimal,
    pub spent: Decimal,
    pub projected_month_end: Decimal,
    pub remaining: Decimal,
    pub historical_median: Option<Decimal>,
    pub history_month_count: usize,
    pub purchase_count: usize,
    pub month_progress: Decimal,
    pub limit_progress: Decimal,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastMethod {
    /// TimesFM's daily forecast of the rest of the period.
    Timesfm,
    /// This month's pace averaged with the group's historical median.
    HistoricalFallback,
}

/// A variable spending group's expected month-end total with a range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingForecast {
    pub group: String,
    pub currency: Currency,
    pub spent: Decimal,
    pub lower: Decimal,
    pub expected: Decimal,
    pub upper: Decimal,
    pub confidence: Decimal,
    pub method: ForecastMethod,
    /// Complete earlier months with group spending in this currency.
    pub training_months: usize,
}
