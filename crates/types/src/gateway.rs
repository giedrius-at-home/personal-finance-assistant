//! Records the old Python service stored as JSON payloads in the gateway
//! tables. Field names match the stored JSON exactly, so rows written by
//! either implementation stay readable by the other.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::report::{Currency, UnitMoney};

fn default_true() -> bool {
    true
}

/// `owner_settings.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerSettings {
    #[serde(default = "OwnerSettings::policy")]
    pub policy_version: String,
    #[serde(default = "OwnerSettings::zone")]
    pub timezone: String,
    #[serde(default = "OwnerSettings::hour")]
    pub evaluation_hour: u8,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(default = "default_true")]
    pub savings_goal_contributions_enabled: bool,
    #[serde(default)]
    pub payday_day: Option<u8>,
    #[serde(default)]
    pub monthly_salary_eur: Option<Decimal>,
    #[serde(default)]
    pub salary_account_id: Option<String>,
    #[serde(default = "OwnerSettings::buffer")]
    pub safety_buffer_eur: Decimal,
    #[serde(default = "OwnerSettings::spent_ratio")]
    pub heads_up_spent_ratio: Decimal,
    #[serde(default = "OwnerSettings::pace_ratio")]
    pub heads_up_pace_ratio: Decimal,
    #[serde(default = "OwnerSettings::heads_up_hours")]
    pub heads_up_cooldown_hours: u16,
    #[serde(default = "OwnerSettings::warning_hours")]
    pub warning_cooldown_hours: u16,
    #[serde(default = "OwnerSettings::critical_hours")]
    pub critical_cooldown_hours: u16,
    #[serde(default)]
    pub forecast_excluded_categories: Vec<String>,
}

impl OwnerSettings {
    fn policy() -> String {
        "2026-09-v1".into()
    }
    fn zone() -> String {
        "Europe/Amsterdam".into()
    }
    fn hour() -> u8 {
        18
    }
    fn buffer() -> Decimal {
        Decimal::from(300)
    }
    fn spent_ratio() -> Decimal {
        Decimal::new(65, 2)
    }
    fn pace_ratio() -> Decimal {
        Decimal::new(125, 2)
    }
    fn heads_up_hours() -> u16 {
        168
    }
    fn warning_hours() -> u16 {
        72
    }
    fn critical_hours() -> u16 {
        24
    }
}

impl Default for OwnerSettings {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults are valid")
    }
}

/// One group in a month's budget plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetGroup {
    pub id: String,
    pub name: String,
    pub amount: Decimal,
    #[serde(default)]
    pub source_groups: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub transaction_ids: Vec<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

/// `monthly_budget_plans.payload_json`; `month` is the first day (`YYYY-MM-01`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetPlan {
    pub month: String,
    pub currency: Currency,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub groups: Vec<BudgetGroup>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Outflow,
    Inflow,
}

/// `planned_payments.payload_json`: a one-off payment the owner expects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPayment {
    pub plan_id: String,
    pub name: String,
    pub match_category: String,
    #[serde(default)]
    pub direction: Direction,
    pub amount: UnitMoney,
    pub due_on: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reimbursement_for: Option<String>,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub match_description: Option<String>,
}

/// `spending_group_history`: the reviewed group of one category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMapping {
    pub category: String,
    pub group: Option<String>,
    pub revision: u32,
}

/// `spending_group_limits.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingLimit {
    pub group: String,
    pub amount: UnitMoney,
    #[serde(default)]
    pub revision: u32,
    pub updated_at: String,
}

/// `subscription_category_rules.payload_json`: categories that count as a bill's payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionCategoryRule {
    pub bill_id: String,
    #[serde(default)]
    pub categories: Vec<String>,
    pub revision: u32,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerTag {
    pub label: String,
    #[serde(default)]
    pub parent: String,
}

/// `transaction_details.payload_json`: the owner's notes on one transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerDetails {
    #[serde(default)]
    pub tags: Vec<OwnerTag>,
    #[serde(default)]
    pub merchant_name: String,
    #[serde(default)]
    pub merchant_location: String,
    #[serde(default)]
    pub payment_method: String,
    #[serde(default)]
    pub from_country: String,
    #[serde(default)]
    pub to_country: String,
    #[serde(default)]
    pub notes: String,
    /// "", "component" or "settlement".
    #[serde(default)]
    pub bulk_payment_role: String,
    #[serde(default)]
    pub bulk_payment_name: String,
    /// "", "return", "payment" or "ignore".
    #[serde(default)]
    pub personal_expense_role: String,
    #[serde(default)]
    pub personal_expense_category: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDetails {
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub values: OwnerDetails,
}

/// `daily_review_policy.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyReviewPolicy {
    pub activated_at: String,
    pub timezone: String,
    pub auto_threshold: Decimal,
    pub missed_after_days: u32,
}

/// `daily_review_entries.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyReviewEntry {
    pub transaction_id: String,
    pub batch_date: String,
    pub first_seen: String,
    pub due_at: String,
    #[serde(default)]
    pub resolved_at: Option<String>,
    /// "manual", "automatic" or "source".
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub missed_at: Option<String>,
    #[serde(default)]
    pub auto_error: Option<String>,
    #[serde(default)]
    pub auto_proposal_id: Option<String>,
    #[serde(default)]
    pub auto_category: Option<String>,
    #[serde(default)]
    pub audited_at: Option<String>,
}

/// `account_roles.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRole {
    pub account_id: String,
    /// "spending", "savings" or "ignored".
    pub role: String,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_account_id: Option<String>,
    pub confirmed_at: String,
}

/// `pots.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotRecord {
    pub pot_id: String,
    pub name: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
    pub created_at: String,
}

/// `pot_entries.payload_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotEntryRecord {
    pub entry_id: String,
    pub pot_id: String,
    pub month: String,
    pub amount: Decimal,
    pub currency: Currency,
    #[serde(default)]
    pub line_id: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub created_at: String,
}

/// `canonical_goals.payload_json` (a former Firefly piggy bank).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsGoalRecord {
    pub goal: SavingsGoal,
    pub priority: u32,
    pub protected_amount: UnitMoney,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsGoal {
    pub goal_id: String,
    pub name: String,
    pub target: UnitMoney,
    pub current: UnitMoney,
    pub started_on: String,
    pub target_date: String,
}

/// A new one-off planned payment or expected income, as the site's
/// `/api/finance/plans` "create" payload carried it (Python
/// `/planned-payments/create`). `amount` stays text so its format can be
/// checked exactly; empty optional strings count as absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPaymentRequest {
    pub name: String,
    pub match_category: String,
    #[serde(default)]
    pub direction: Direction,
    /// A positive decimal like `12.50`.
    pub amount: String,
    /// ISO currency code (`EUR`).
    pub currency: String,
    /// `YYYY-MM-DD`.
    pub due_on: String,
    pub account_id: String,
    /// The purchase (`firefly:<id>`) this income reimburses.
    #[serde(default)]
    pub reimbursement_for: Option<String>,
    /// Text the matching bank description must contain.
    #[serde(default)]
    pub match_description: Option<String>,
}
