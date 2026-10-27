//! Report section types for `finance_core::reporting` (weekly report, net
//! worth, taxonomy hints) and `finance_core::groups` (monthly spending).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;
use crate::gateway::GroupMapping;

/// Ordered human-facing alert level; serialized names sort the way the
/// Python report listed them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    None,
    HeadsUp,
    Warning,
    Critical,
}

impl AlertSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HeadsUp => "heads_up",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "heads_up" => Self::HeadsUp,
            "warning" => Self::Warning,
            "critical" => Self::Critical,
            _ => return None,
        })
    }
}

/// Progress of one savings goal against a linear schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalEvaluation {
    pub goal_id: String,
    pub currency: Currency,
    pub completed: bool,
    pub overdue: bool,
    pub on_track: bool,
    pub remaining: Decimal,
    pub expected_saved: Decimal,
    pub schedule_variance: Decimal,
    pub required_weekly_contribution: Decimal,
    pub days_remaining: i64,
    pub projected_completion_date: Option<String>,
}

/// Spending in one category during the week (refunds subtract).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekCategory {
    pub category: String,
    pub amount: Decimal,
    pub transaction_count: u64,
}

/// Cash flow of one currency; never mixed with other currencies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyWeek {
    pub currency: Currency,
    pub income: Decimal,
    pub spending: Decimal,
    pub net_cash_flow: Decimal,
    pub transfer_count: u64,
    pub categories: Vec<WeekCategory>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyAmount {
    pub currency: Currency,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertCount {
    pub severity: AlertSeverity,
    pub count: u64,
}

/// One local ISO week, Monday through today at most.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyReport {
    pub report_version: String,
    pub basis: String,
    pub timezone: String,
    pub week_start: String,
    pub through_date: String,
    pub complete_week: bool,
    pub currencies: Vec<CurrencyWeek>,
    pub safe_to_spend: Vec<CurrencyAmount>,
    pub active_alerts: Vec<AlertCount>,
    pub goals: Vec<GoalEvaluation>,
    pub source_transaction_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetWorthComponent {
    pub account_id: String,
    pub label: String,
    /// The system that owns the balance (always `firefly_iii` for ledger accounts).
    pub owner: String,
    pub currency: Currency,
    pub amount: Decimal,
    /// When the balance was last refreshed from the source (RFC 3339).
    pub updated_at: Option<String>,
    pub stale: bool,
    pub excluded_mirror_count: u64,
}

/// Every included account once; a combined total only when it is honest
/// (one currency, nothing stale).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetWorthReport {
    pub report_version: String,
    pub as_of: String,
    pub components: Vec<NetWorthComponent>,
    pub currency_totals: Vec<CurrencyAmount>,
    pub total: Option<Decimal>,
    pub total_currency: Option<Currency>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxonomySignal {
    Sparse,
    DuplicateLike,
    Overloaded,
}

/// A review hint about the category vocabulary; never applied automatically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxonomyRecommendation {
    pub signal: TaxonomySignal,
    pub categories: Vec<String>,
    pub reason: String,
}

/// One category ("store") inside a spending group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreSpending {
    pub category: Option<String>,
    pub spending: Decimal,
    pub gross: Decimal,
    pub refunds: Decimal,
    pub transaction_count: u64,
    /// Same days of the previous month; `None` when that month had no spending.
    pub previous_spending: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSpending {
    pub group: Option<String>,
    pub spending: Decimal,
    pub share_percent: Decimal,
    pub stores: Vec<StoreSpending>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencySpending {
    pub currency: Currency,
    pub spending: Decimal,
    pub gross: Decimal,
    pub refunds: Decimal,
    pub comparison_spending: Decimal,
    pub previous_spending: Option<Decimal>,
    pub transfer_count: u64,
    pub groups: Vec<GroupSpending>,
}

/// This calendar month's spending by reviewed group, against the same days
/// of last month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlySpending {
    pub start: String,
    pub through: String,
    pub comparison_through: String,
    pub previous_start: String,
    pub previous_through: String,
    pub currencies: Vec<CurrencySpending>,
    /// Every spending category with its reviewed group (or none yet).
    pub mappings: Vec<GroupMapping>,
}
