//! Report section types for `finance_core::cash`.

use serde::{Deserialize, Serialize};

use super::UnitMoney;
use super::reporting::CurrencyWeek;

/// What the ledger holds for one account, without claiming completeness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountHealth {
    pub account_id: String,
    pub name: String,
    /// When the account was last refreshed from the source (RFC 3339).
    pub source_modified_at: Option<String>,
    pub first_transaction_on: Option<String>,
    pub last_transaction_on: Option<String>,
    pub transaction_count: u64,
    pub outflow_count: u64,
    pub uncategorised_outflows: u64,
}

/// Whether this cycle's salary arrived.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaydayStatus {
    pub due_on: String,
    pub next_expected_on: String,
    pub received_on: Option<String>,
    pub received_amounts: Vec<UnitMoney>,
    pub transaction_count: u64,
}

/// Cash flow since the last payday and per-account record health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashOverview {
    pub through_date: String,
    pub period_start: Option<String>,
    pub next_payday: Option<String>,
    pub days_until_payday: Option<i64>,
    pub payday_status: Option<PaydayStatus>,
    pub currencies: Vec<CurrencyWeek>,
    pub accounts: Vec<AccountHealth>,
}
