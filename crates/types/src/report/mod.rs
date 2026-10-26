//! The dashboard report every screen reads: the Rust port of the Python
//! service's `DashboardView`. Field names match its JSON so the two can be
//! compared field by field.

pub mod allocation;
pub mod budget;
pub mod cash;
pub mod catalog;
pub mod plans;
pub mod reporting;
pub mod review;
pub mod subscriptions;
pub mod trends;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A currency and its number of minor digits (EUR → 2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Currency {
    pub code: String,
    pub minor_unit: u8,
}

impl Currency {
    pub fn new(code: &str) -> Self {
        let minor_unit = match code {
            "JPY" | "KRW" | "ISK" | "HUF" | "CLP" | "VND" => 0,
            "BHD" | "KWD" | "OMR" | "JOD" | "TND" => 3,
            _ => 2,
        };
        Self {
            code: code.to_owned(),
            minor_unit,
        }
    }

    pub fn eur() -> Self {
        Self::new("EUR")
    }
}

/// An exact amount with its currency unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitMoney {
    pub amount: Decimal,
    pub currency: Currency,
}

/// Everything the Overview, Spending and Plan screens read, built in one go
/// so every figure agrees (the Python `DashboardView`, without the review
/// inbox, which is [`ReviewReport`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    /// The build moment, RFC 3339 with the owner's offset.
    pub generated_at: String,
    pub settings: crate::gateway::OwnerSettings,
    pub weekly: reporting::WeeklyReport,
    pub net_worth: reporting::NetWorthReport,
    pub cash_overview: cash::CashOverview,
    /// Last successful bank refresh (RFC 3339).
    pub last_firefly_refresh: Option<String>,
    pub firefly_refresh_failed: bool,
    pub subscriptions: subscriptions::SubscriptionReport,
    /// Next month's bills, once this month's salary arrived.
    pub next_month_subscriptions: Option<subscriptions::SubscriptionReport>,
    pub planned_payments: Vec<plans::PlannedPaymentView>,
    pub monthly_spending: reporting::MonthlySpending,
    pub category_catalog: catalog::CategoryCatalog,
    pub taxonomy: Vec<reporting::TaxonomyRecommendation>,
    pub bulk_payments: Vec<catalog::BulkPaymentSummary>,
    pub historical_uncategorized_count: u64,
    pub custom_spending_groups: Vec<String>,
    pub time_trends: trends::TimeTrends,
    /// Trends without bills and other fixed spending.
    pub variable_time_trends: trends::TimeTrends,
    pub spending_limits: Vec<crate::gateway::SpendingLimit>,
    pub spending_nudges: Vec<trends::SpendingNudge>,
    pub spending_forecasts: Vec<trends::SpendingForecast>,
}

/// The category review inbox (Transactions screen).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewReport {
    pub review_items: Vec<review::ReviewItem>,
    pub daily_review: Option<review::DailyInbox>,
}
