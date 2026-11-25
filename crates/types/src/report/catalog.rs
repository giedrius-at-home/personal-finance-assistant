//! Report section types for `finance_core::catalog`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryEntry {
    pub name: String,
    pub count: u64,
}

/// A less-used spelling (`source`) that looks like a more-used one (`target`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryPair {
    pub source: String,
    pub target: String,
    pub count: u64,
    pub reason: String,
}

/// A name that sits in two places of the spending groups or the budget, so
/// only one of them ever counts it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupConflict {
    pub name: String,
    /// The groups (or budget lines) that each claim it.
    pub groups: Vec<String>,
    pub note: String,
}

/// Bills for the same kind of thing (two cinema passes, two music apps).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionOverlap {
    /// `cinema`, `video streaming`, …
    pub kind: String,
    pub names: Vec<String>,
}

/// The categories in use, and likely duplicates to consolidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCatalog {
    pub entries: Vec<CategoryEntry>,
    pub similar: Vec<CategoryPair>,
    /// Different names for the same kind of spending (Dining, Restaurant,
    /// Eating out): less-used into most-used.
    #[serde(default)]
    pub overlaps: Vec<CategoryPair>,
    #[serde(default)]
    pub group_conflicts: Vec<GroupConflict>,
    #[serde(default)]
    pub subscription_overlaps: Vec<SubscriptionOverlap>,
}

/// One owner-named bulk payment: the purchases it covers against the
/// payments that settled it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkPaymentSummary {
    pub name: String,
    pub currency: Currency,
    pub component_total: Decimal,
    pub settlement_total: Decimal,
    pub outstanding_amount: Decimal,
    pub component_count: u64,
    pub settlement_count: u64,
}
