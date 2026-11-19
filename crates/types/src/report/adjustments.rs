//! Report section types for `finance_core::adjustments`: budget rows (real
//! and shadow) with how often the owner goes where.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;
use super::budget::BudgetRow;

/// One currency's spending groups of the running budget month, each against
/// the owner's own history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjustments {
    pub currency: Currency,
    /// The budget month (`YYYY-MM-01`).
    pub month: String,
    pub rows: Vec<AdjustmentRow>,
    /// This month's spending without a category or a group, or in "Other".
    pub unsorted: Decimal,
}

/// A budget group, or a spending group without a budget (a shadow row).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustmentRow {
    pub row: BudgetRow,
    /// `false` for a shadow row, whose `row.planned` is zero.
    pub budgeted: bool,
    /// The reviewed group of a shadow row.
    pub group: Option<String>,
    /// The row as a whole.
    pub habit: Habit,
    /// The row's stores (categories), the most visited lately first.
    pub stores: Vec<StoreHabit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreHabit {
    pub category: String,
    pub habit: Habit,
}

/// Visits (payments; refunds are not visits) and net spending.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Habit {
    /// This budget month.
    pub visits: usize,
    pub spent: Decimal,
    /// Medians over the earlier months with a visit, once there are two.
    pub typical_visits: Option<Decimal>,
    pub typical_spent: Option<Decimal>,
    pub history_month_count: usize,
    /// Monday through today.
    pub visits_this_week: usize,
    /// Median of the twelve full weeks before; a week without a visit is zero.
    pub typical_week_visits: Decimal,
    /// The last four full weeks and this one.
    pub recent_visits: usize,
    /// Monday first, over the twelve full weeks.
    pub weekday_visits: [usize; 7],
}
