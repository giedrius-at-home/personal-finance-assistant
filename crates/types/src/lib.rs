//! Domain types shared by every crate and by the Leptos UI.
//!
//! This crate is the single definition of what crosses the Tauri boundary, so
//! the UI and the backend can never disagree about a shape. Money is always an
//! exact [`Decimal`] serialized as a string; dates are ISO `YYYY-MM-DD` strings.

pub mod gateway;
pub mod manage;
pub mod report;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// An exact amount in one currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub amount: Decimal,
    pub currency: String,
}

/// A reference to a named record (account, category, …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Asset,
    Liability,
    Expense,
    Revenue,
    Cash,
    Opening,
    Reconciliation,
    Other,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Expense => "expense",
            Self::Revenue => "revenue",
            Self::Cash => "cash",
            Self::Opening => "opening",
            Self::Reconciliation => "reconciliation",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "asset" => Self::Asset,
            "liability" => Self::Liability,
            "expense" => Self::Expense,
            "revenue" => Self::Revenue,
            "cash" => Self::Cash,
            "opening" => Self::Opening,
            "reconciliation" => Self::Reconciliation,
            _ => Self::Other,
        }
    }

    /// Accounts the owner holds money in (shown on Overview, count to net worth).
    pub fn is_own(self) -> bool {
        matches!(self, Self::Asset | Self::Liability | Self::Cash)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxKind {
    Withdrawal,
    Deposit,
    Transfer,
    Opening,
    Reconciliation,
    Other,
}

impl TxKind {
    pub const FILTERABLE: [Self; 3] = [Self::Withdrawal, Self::Deposit, Self::Transfer];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Withdrawal => "withdrawal",
            Self::Deposit => "deposit",
            Self::Transfer => "transfer",
            Self::Opening => "opening",
            Self::Reconciliation => "reconciliation",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "withdrawal" => Self::Withdrawal,
            "deposit" => Self::Deposit,
            "transfer" => Self::Transfer,
            "opening" => Self::Opening,
            "reconciliation" => Self::Reconciliation,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Withdrawal => "Spending",
            Self::Deposit => "Income",
            Self::Transfer => "Transfer",
            Self::Opening => "Opening balance",
            Self::Reconciliation => "Reconciliation",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: i64,
    pub name: String,
    pub kind: AccountKind,
    pub role: Option<String>,
    /// Last four characters of the IBAN, if any.
    pub iban_tail: Option<String>,
    pub balances: Vec<Money>,
    pub transaction_count: u64,
    pub last_activity: Option<String>,
}

/// Money in and out of the owner's accounts for one month and currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthFlow {
    pub month: String,
    pub currency: String,
    pub income: Decimal,
    pub spending: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryTotal {
    pub category: Option<Named>,
    pub currency: String,
    pub amount: Decimal,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataStatus {
    pub transactions: u64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub imported_at: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overview {
    pub net_worth: Vec<Money>,
    pub accounts: Vec<AccountSummary>,
    pub months: Vec<MonthFlow>,
    /// Spending by category for `category_month`.
    pub categories: Vec<CategoryTotal>,
    pub category_month: Option<String>,
    pub status: DataStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxQuery {
    pub text: Option<String>,
    pub account_id: Option<i64>,
    pub category_id: Option<i64>,
    /// Only rows without a category (ignored when `category_id` is set).
    pub uncategorised: bool,
    pub kind: Option<TxKind>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRow {
    pub id: i64,
    pub date: String,
    pub description: String,
    pub kind: TxKind,
    /// Signed from the owner's point of view: spending negative, income positive,
    /// transfers positive.
    pub amount: Money,
    pub foreign: Option<Money>,
    pub source: Named,
    pub destination: Named,
    pub category: Option<Named>,
    pub bill: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub reconciled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxPage {
    pub rows: Vec<TxRow>,
    pub total: u64,
    /// Net total of all matching rows, per currency.
    pub sums: Vec<Money>,
}

/// Values the transaction filters can choose from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facets {
    pub accounts: Vec<Named>,
    pub categories: Vec<Named>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
}

/// Per-account totals used to prove a migration copied everything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCheck {
    pub legacy_id: i64,
    pub postings: u64,
    pub sums: Vec<Money>,
}

/// A newer release the app can install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// What the app knows about updates right now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub current_version: String,
    /// A release token is stored in the Keychain.
    pub token_set: bool,
    pub checking: bool,
    pub installing: bool,
    /// Local time of the last finished check, `YYYY-MM-DD HH:MM`.
    pub last_checked: Option<String>,
    pub available: Option<AvailableUpdate>,
    /// Short, data-free description of the last failure.
    pub error: Option<String>,
}
