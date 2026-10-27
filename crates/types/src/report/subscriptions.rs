//! Report section types for `finance_core::subscriptions`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{Currency, UnitMoney};

/// Bills due in one window: what they cost, which are paid, and which
/// unlinked transactions look like their payments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionReport {
    /// When the bills were last read from the bank import (RFC 3339).
    pub fetched_at: Option<String>,
    pub through_date: String,
    pub stale: bool,
    pub rows: Vec<SubscriptionRow>,
    /// Every expected payment in the window at the bill's maximum amount.
    pub expected_costs: Vec<UnitMoney>,
    /// The expected payments not yet paid.
    #[serde(default)]
    pub remaining_costs: Vec<UnitMoney>,
    #[serde(default)]
    pub scheduled_payments: Vec<ScheduledBillPayment>,
    pub unmatched_count: usize,
    pub evidence_start: String,
    pub category_matched_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionRow {
    pub bill_id: String,
    pub name: String,
    pub currency: Currency,
    pub amount_min: Decimal,
    pub amount_max: Decimal,
    pub frequency: String,
    pub expected_dates: Vec<String>,
    /// The newest payment linked to the bill in the last 31 days.
    pub last_matched_on: Option<String>,
    pub native_payment_count: usize,
    pub category_payment_count: usize,
    pub recorded_spending: Decimal,
    pub last_recorded_on: Option<String>,
    pub category_name_match: bool,
    #[serde(default)]
    pub category_match_categories: Vec<String>,
    #[serde(default)]
    pub category_match_configured: bool,
    #[serde(default)]
    pub category_match_revision: u32,
    #[serde(default)]
    pub link_candidates: Vec<BillLinkCandidate>,
}

/// An unlinked transaction that names the bill (or uses its category).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillLinkCandidate {
    /// The ledger id, without the `firefly:` prefix.
    pub transaction_id: String,
    pub occurred_on: String,
    pub description: String,
    pub amount: Decimal,
    pub evidence: String,
    #[serde(default)]
    pub category: Option<String>,
}

/// One expected bill payment and the day it was paid, if it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledBillPayment {
    pub bill_id: String,
    pub name: String,
    pub due_on: String,
    pub paid_on: Option<String>,
    pub amount: UnitMoney,
    #[serde(default)]
    pub candidate_ids: Vec<String>,
}
