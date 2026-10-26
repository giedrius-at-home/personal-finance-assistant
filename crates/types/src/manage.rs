//! The Manage section: bank CSV import, owner settings, status and banks
//! (personal-site `owner-operations.tsx`, `owner-banks.tsx`,
//! `lib/finance/operations-contract.ts`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::AccountSummary;
use crate::gateway::OwnerSettings;

/// An own account the CSV import and the salary setting can target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedAccount {
    /// The ledger id, as the import form submits it.
    pub id: String,
    /// `firefly:<ledger id>`, as the settings store it.
    pub account_id: String,
    pub name: String,
    pub currency: String,
}

/// One applied settings change (`settings_history`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsRevision {
    pub revision_id: i64,
    pub actor: String,
    pub applied_at: String,
    pub active: bool,
    pub rolled_back_at: Option<String>,
    pub rolled_back_by: Option<String>,
}

/// One row of `import_runs`: a ledger change and when it finished.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportRun {
    pub source: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub finished_at: String,
    pub stats: serde_json::Value,
}

/// What the Import, Status and Settings screens read (the site's
/// `/api/finance/workspace`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManageWorkspace {
    pub csv_enabled: bool,
    pub notifications_available: bool,
    pub settings: OwnerSettings,
    /// Newest first, at most 50.
    pub revisions: Vec<SettingsRevision>,
    pub accounts: Vec<ManagedAccount>,
    /// The latest ledger change of any kind.
    pub last_import: Option<ImportRun>,
    /// The latest bank CSV import.
    pub last_csv_import: Option<ImportRun>,
    /// When the database file last changed (UTC, RFC 3339).
    pub last_write: Option<String>,
    pub transactions: u64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
}

/// The settings form as typed; validated into [`OwnerSettings`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsForm {
    pub timezone: String,
    pub evaluation_hour: String,
    pub notifications_enabled: bool,
    pub payday_day: String,
    pub monthly_salary_eur: String,
    pub salary_account_id: String,
    pub safety_buffer_eur: String,
    pub heads_up_spent_ratio: String,
    pub heads_up_pace_ratio: String,
    pub heads_up_cooldown_hours: String,
    pub warning_cooldown_hours: String,
    pub critical_cooldown_hours: String,
    pub forecast_excluded_categories: Vec<String>,
}

/// Before and after of a settings change; `token` binds both.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPreview {
    pub token: String,
    pub before: OwnerSettings,
    pub after: OwnerSettings,
}

/// Apply exactly the previewed settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsApply {
    pub token: String,
    pub settings: OwnerSettings,
}

/// A bank export to preview: the decoded text of the chosen file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvUpload {
    /// A ledger account id, or `auto` to detect it from the file name.
    pub account_id: String,
    pub filename: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvRowStatus {
    New,
    Duplicate,
    PossibleDuplicate,
}

impl CsvRowStatus {
    /// `possible duplicate`, as the site shows it.
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Duplicate => "duplicate",
            Self::PossibleDuplicate => "possible duplicate",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvPreviewRow {
    pub line_number: u32,
    pub booked_on: String,
    pub amount: Decimal,
    pub currency: String,
    pub description: String,
    pub status: CsvRowStatus,
}

/// A row that could not be read; never carries the row's values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvRejectedRow {
    pub line_number: u32,
    pub reason: String,
}

/// The review summary of a staged import; `preview_id` commits exactly it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvPreview {
    pub preview_id: String,
    pub bank_label: String,
    pub account_name: String,
    pub row_count: u32,
    pub new_count: u32,
    pub duplicate_count: u32,
    pub possible_duplicate_count: u32,
    pub rejected_count: u32,
    pub committable: bool,
    /// The first 100 rows.
    pub rows: Vec<CsvPreviewRow>,
    pub rejected_rows: Vec<CsvRejectedRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvCommit {
    pub preview_id: String,
    pub skip_flagged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvCommitResult {
    pub imported: u32,
    pub refreshed: bool,
}

/// One own account on the Banks screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankAccount {
    pub account: AccountSummary,
    pub active: bool,
    /// The bank the account belongs to (`ABN AMRO`, `Revolut`, `Swedbank`),
    /// or `None` for other accounts.
    pub bank: Option<String>,
    /// The latest CSV import into this account (UTC).
    pub last_csv_import: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanksReport {
    pub accounts: Vec<BankAccount>,
    /// The latest ledger change of any kind.
    pub last_import: Option<ImportRun>,
}
