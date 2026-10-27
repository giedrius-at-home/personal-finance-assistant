//! Bank sync through Enable Banking: the per-bank state the Banks screen and
//! the Close month sync step show, and the owner's requests.

use serde::{Deserialize, Serialize};

/// The banks the app syncs, in sync order: `(id, name)`.
pub const BANKS: [(&str, &str); 3] = [
    ("abnamro", "ABN AMRO"),
    ("revolut", "Revolut"),
    ("swedbank", "Swedbank"),
];

/// The display name of bank `id`.
pub fn bank_name(id: &str) -> &str {
    BANKS
        .iter()
        .find(|(bank, _)| *bank == id)
        .map_or(id, |(_, name)| name)
}

/// Outcome of the last sync or check of one bank (the statuses of the old
/// importer's `status.json`, plus `missing_credentials`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Verified,
    WaitingForCompletedDay,
    Success,
    Failed,
    ImportFailed,
    ReauthorizationRequired,
    MappingMismatch,
    InvalidConfiguration,
    InvalidMapping,
    RateLimited,
    UpstreamUnavailable,
    RuntimeUnavailable,
    MissingConfig,
    MissingCredentials,
    ProviderPaginationError,
}

impl SyncStatus {
    /// The short reason the Close month step shows for a failed bank.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Verified | Self::Success | Self::WaitingForCompletedDay => "Synced",
            Self::ReauthorizationRequired
            | Self::MappingMismatch
            | Self::InvalidConfiguration
            | Self::InvalidMapping
            | Self::MissingConfig => "Reconnect required",
            Self::MissingCredentials => "Enable Banking key missing",
            Self::RateLimited => "Rate limited",
            Self::UpstreamUnavailable | Self::RuntimeUnavailable => "Bank unavailable",
            Self::ProviderPaginationError => "Bank returned inconsistent pages",
            Self::ImportFailed | Self::Failed => "Sync failed",
        }
    }

    /// A finished sync the owner needs to do nothing about.
    pub fn is_ok(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Verified | Self::WaitingForCompletedDay
        )
    }
}

/// What the app stores per bank after each attempt (UTC RFC 3339 times).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankRun {
    pub status: Option<SyncStatus>,
    pub last_attempt: Option<String>,
    pub last_success: Option<String>,
    /// New ledger transactions of the last successful sync.
    pub created: Option<u32>,
    /// Bank transactions the last successful sync looked at.
    pub checked: Option<u32>,
    /// When the bank consent ends (UTC), as the bank last reported it.
    #[serde(default)]
    pub consent_valid_until: Option<String>,
    /// What the bank answered when the last attempt failed (request kind,
    /// HTTP status, the bank's error code and message; never account data).
    #[serde(default)]
    pub detail: Option<String>,
    /// Each synced account's balance at the bank next to the ledger's, as of
    /// the last successful sync.
    #[serde(default)]
    pub balances: Vec<AccountBalance>,
}

/// One synced account's balance at the bank and in the ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBalance {
    pub account_id: i64,
    pub account: String,
    pub currency: String,
    /// Decimal strings.
    pub bank: String,
    pub ledger: String,
    /// The bank's balance type (`CLBD` closing booked, `ITAV` available, ...).
    pub balance_type: String,
    pub reference_date: Option<String>,
}

/// One bank on the Banks screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankState {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub run: BankRun,
    /// When the scheduler syncs this bank next (UTC).
    pub next_eligible: Option<String>,
    /// Set while a rate limit holds manual syncs back (UTC).
    pub retry_after: Option<String>,
    /// A consent is stored for this bank.
    pub connected: bool,
    /// The ledger accounts the consent fills.
    pub accounts: Vec<String>,
}

/// The Banks screen: every bank plus what the app can do right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankSyncReport {
    pub banks: Vec<BankState>,
    /// The Enable Banking application id and private key are stored.
    pub credentials: bool,
    /// The bank being synced right now.
    pub running: Option<String>,
    /// The old importer's saved consents can be taken over.
    pub legacy_consents: bool,
    /// When the in-app scheduler last looked for due banks (UTC).
    pub last_check: Option<String>,
}

/// The owner's Enable Banking application: id and PEM private key.
#[derive(Clone, Serialize, Deserialize)]
pub struct BankCredentials {
    pub app_id: String,
    pub private_key: String,
}

impl std::fmt::Debug for BankCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BankCredentials(<redacted>)")
    }
}

/// A renewed consent: the importer configuration exported after the owner
/// approved access at the bank.
#[derive(Clone, Serialize, Deserialize)]
pub struct BankConsent {
    pub bank: String,
    pub configuration: String,
}

impl std::fmt::Debug for BankConsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BankConsent")
            .field("bank", &self.bank)
            .finish_non_exhaustive()
    }
}

/// The result of syncing one bank.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankSyncResult {
    pub bank: String,
    pub status: SyncStatus,
    pub created: Option<u32>,
    pub checked: Option<u32>,
}

/// The Enable Banking ASPSP (bank name and country) behind bank `id`.
pub fn aspsp(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "abnamro" => Some(("ABN AMRO", "NL")),
        "revolut" => Some(("Revolut", "NL")),
        "swedbank" => Some(("Swedbank", "LT")),
        _ => None,
    }
}

/// The redirect the old importer registered with Enable Banking.
pub const DEFAULT_REDIRECT: &str = "https://localhost:8444/eb-callback";

/// Start connecting `bank`: the owner approves at `url` in the browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLinkStart {
    pub bank: String,
    pub redirect_url: String,
}

/// One bank account the approved session grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLinkAccount {
    pub uid: String,
    pub name: String,
    pub iban_tail: Option<String>,
    pub currency: Option<String>,
    /// The ledger account it most likely fills.
    pub suggested: Option<i64>,
}

/// A ledger account a bank account may fill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerTarget {
    pub id: i64,
    pub name: String,
    pub iban_tail: Option<String>,
}

/// The approved session's accounts, for the owner to map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLinkProposal {
    pub bank: String,
    pub accounts: Vec<BankLinkAccount>,
    pub targets: Vec<LedgerTarget>,
    /// Ledger accounts the previous consent filled; each must stay mapped.
    pub required: Vec<i64>,
    pub valid_until: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLinkChoice {
    pub uid: String,
    /// `None`: do not import this bank account.
    pub target: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLinkConfirm {
    pub bank: String,
    pub choices: Vec<BankLinkChoice>,
}
