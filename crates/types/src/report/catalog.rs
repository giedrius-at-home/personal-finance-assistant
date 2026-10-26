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

/// The categories in use, and likely duplicates to consolidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCatalog {
    pub entries: Vec<CategoryEntry>,
    pub similar: Vec<CategoryPair>,
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
