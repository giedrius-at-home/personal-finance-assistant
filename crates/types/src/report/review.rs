//! Report section types for `finance_core::review`: the category review
//! inbox, the daily review and the transaction search.
//!
//! The proposal parts double as the stored `category_proposals.payload_json`
//! shape, so their field names and defaults follow the Python models.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::gateway::{DailyReviewEntry, DailyReviewPolicy, SavedDetails};
use crate::report::{Currency, UnitMoney};

/// Where a country claim about one side of a payment comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountryEvidence {
    pub code: String,
    /// "account_iban", "description_country", "description_iban" or "description_bic".
    pub basis: String,
}

/// A model-suggested detail, with the memo excerpts that support it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailSuggestion {
    pub value: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSuggestion {
    pub label: String,
    #[serde(default)]
    pub parent: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraSuggestions {
    #[serde(default)]
    pub tags: Vec<TagSuggestion>,
    #[serde(default)]
    pub merchant_name: Option<DetailSuggestion>,
    #[serde(default)]
    pub merchant_location: Option<DetailSuggestion>,
    #[serde(default)]
    pub payment_method: Option<DetailSuggestion>,
}

/// An unconfirmed sender or recipient country; never replaces bank fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountrySuggestion {
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountrySuggestions {
    #[serde(default)]
    pub from_country: Option<CountrySuggestion>,
    #[serde(default)]
    pub to_country: Option<CountrySuggestion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    #[default]
    Unchecked,
    Supported,
    Review,
    Unavailable,
}

/// The local plausibility second opinion, separate from category confidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCheck {
    #[serde(default)]
    pub status: CheckStatus,
    #[serde(default = "CategoryCheck::default_reason")]
    pub reason: String,
    #[serde(default = "CategoryCheck::default_version")]
    pub version: String,
    #[serde(default)]
    pub extras: ExtraSuggestions,
    #[serde(default)]
    pub countries: CountrySuggestions,
}

impl CategoryCheck {
    fn default_reason() -> String {
        "Not checked yet; manual review required.".into()
    }
    fn default_version() -> String {
        "category-check-v1".into()
    }
}

impl Default for CategoryCheck {
    fn default() -> Self {
        Self {
            status: CheckStatus::Unchecked,
            reason: Self::default_reason(),
            version: Self::default_version(),
            extras: ExtraSuggestions::default(),
            countries: CountrySuggestions::default(),
        }
    }
}

/// One category the model considered, with its probability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryAlternative {
    pub category: String,
    pub probability: Decimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorisationMethod {
    Baseline,
    Model,
    Abstain,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    #[default]
    Pending,
    Approved,
    Corrected,
    Rejected,
    Reversed,
    Superseded,
}

/// The ledger record a proposal was made for, with the revision it saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalSource {
    pub system: String,
    pub entity: String,
    pub external_id: String,
    #[serde(default)]
    pub source_revision: Option<String>,
    #[serde(default)]
    pub source_updated_at: Option<String>,
}

/// `category_proposals.payload_json`: one reviewable category suggestion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryProposal {
    pub proposal_id: String,
    pub source: ProposalSource,
    #[serde(default)]
    pub original_category: Option<String>,
    #[serde(default)]
    pub proposed_category: Option<String>,
    pub method: CategorisationMethod,
    #[serde(default)]
    pub confidence: Option<Decimal>,
    #[serde(default)]
    pub runner_up: Option<String>,
    #[serde(default)]
    pub runner_up_confidence: Option<Decimal>,
    #[serde(default)]
    pub category_options: Vec<CategoryAlternative>,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub prompt_version: Option<String>,
    pub policy_version: String,
    pub vocabulary_version: String,
    pub feature_hash: String,
    pub normalized_merchant: String,
    /// The table's `status` column wins over the payload's copy.
    #[serde(default)]
    pub status: ProposalStatus,
    pub created_at: String,
    #[serde(default)]
    pub sanity_check: CategoryCheck,
    #[serde(default)]
    pub extras: ExtraSuggestions,
    #[serde(default)]
    pub country_suggestions: CountrySuggestions,
}

/// One proposal in the review inbox, with the transaction it is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub deferred: bool,
    pub bank_name: Option<String>,
    pub extras: ExtraSuggestions,
    pub owner_details: SavedDetails,
    pub country_suggestions: CountrySuggestions,
    pub sanity_check: CategoryCheck,
    pub proposal_id: String,
    pub merchant: String,
    pub original_category: Option<String>,
    pub proposed_category: Option<String>,
    pub method: CategorisationMethod,
    pub confidence: Option<Decimal>,
    pub runner_up: Option<String>,
    pub runner_up_confidence: Option<Decimal>,
    pub category_options: Vec<CategoryAlternative>,
    pub reasons: Vec<String>,
    pub model_version: Option<String>,
    pub prompt_version: Option<String>,
    pub policy_version: String,
    pub vocabulary_version: String,
    pub status: ProposalStatus,
    pub source_updated_at: Option<String>,
    pub transaction_description: Option<String>,
    /// Negative for spending.
    pub transaction_amount: Option<Decimal>,
    pub transaction_currency: Option<Currency>,
    /// RFC 3339 with the owner's offset.
    pub transaction_occurred_at: Option<String>,
    pub source_country: Option<CountryEvidence>,
    pub destination_country: Option<CountryEvidence>,
    pub description_country: Option<CountryEvidence>,
}

/// A daily review entry with its transaction and latest proposal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyInboxItem {
    #[serde(flatten)]
    pub entry: DailyReviewEntry,
    pub description: Option<String>,
    /// Negative for spending.
    pub amount: Option<Decimal>,
    pub currency: Option<Currency>,
    pub current_category: Option<String>,
    /// "done", "missed" or "pending".
    pub status: String,
    pub proposal: Option<ReviewItem>,
}

/// How many daily reviews of one month (`YYYY-MM`) passed their deadline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedReviewMonth {
    pub month: String,
    pub missed: u32,
    pub still_open: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyInbox {
    pub automation_active: bool,
    pub suggestions_active: bool,
    pub policy: DailyReviewPolicy,
    pub today: String,
    pub items: Vec<DailyInboxItem>,
    /// Every month since activation, zero months included.
    pub missed_months: Vec<MissedReviewMonth>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchDirection {
    In,
    Out,
    Transfer,
}

/// Filters for the Transactions tab; empty fields do not filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionSearchQuery {
    /// Words that must all appear (accent- and case-insensitive).
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub account: Option<String>,
    /// First local booking date, `YYYY-MM-DD`.
    #[serde(default)]
    pub from: Option<String>,
    /// Last local booking date, `YYYY-MM-DD`.
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub direction: Option<SearchDirection>,
    #[serde(default)]
    pub min: Option<Decimal>,
    #[serde(default)]
    pub max: Option<Decimal>,
    #[serde(default = "TransactionSearchQuery::first_page")]
    pub page: u32,
    #[serde(default = "TransactionSearchQuery::default_page_size")]
    pub page_size: u32,
}

impl TransactionSearchQuery {
    fn first_page() -> u32 {
        1
    }
    fn default_page_size() -> u32 {
        50
    }
}

impl Default for TransactionSearchQuery {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults are valid")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRow {
    /// The ledger id.
    pub transaction_id: String,
    pub occurred_on: String,
    pub direction: SearchDirection,
    pub amount: UnitMoney,
    /// The other party's name, or the other own account of a transfer.
    pub counterparty: Option<String>,
    pub description: Option<String>,
    pub account_id: String,
    pub account_name: Option<String>,
    pub category: Option<String>,
    pub firefly_group_id: Option<String>,
}

/// Money in and out of one currency across all matches; transfers excluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTotal {
    pub currency: Currency,
    #[serde(rename = "in")]
    pub inflow: Decimal,
    #[serde(rename = "out")]
    pub outflow: Decimal,
    pub net: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAccount {
    pub account_id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionSearchResult {
    /// The requested page, newest first.
    pub items: Vec<SearchRow>,
    pub page: u32,
    pub page_size: u32,
    pub total_count: usize,
    pub totals: Vec<SearchTotal>,
    /// Accounts with transactions, for the account filter.
    pub accounts: Vec<SearchAccount>,
}

// ---------------------------------------------------------------------------
// Transactions tab writes (Python `web.py` review and transaction routes).

/// One payment in "Shared spending & money back" (`/transactions/personal`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalTransaction {
    /// The ledger id.
    pub transaction_id: String,
    pub description: String,
    pub occurred_on: String,
    /// Always positive.
    pub amount: UnitMoney,
    pub incoming: bool,
    pub category: Option<String>,
    /// "", "return", "payment" or "ignore".
    pub role: String,
    pub assigned_category: String,
    pub revision: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalTransactions {
    /// Unreviewed first, newest first within each part; at most 100.
    pub items: Vec<PersonalTransaction>,
    pub total: usize,
    /// The first day marks count from, `YYYY-MM-DD`.
    pub since: String,
}

/// Save or clear the owner's shared-spending mark on one payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalChoice {
    pub transaction_id: String,
    /// The `transaction_details` revision the owner saw.
    pub revision: u32,
    /// "return", "payment", "ignore" or "clear".
    pub role: String,
    pub category: String,
    /// The bank amount the owner saw (positive).
    pub expected_amount: String,
    pub expected_currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSaved {
    pub saved: bool,
    pub revision: u32,
}

/// Set one transaction's category (`/transactions/category`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryChange {
    /// The ledger id or the canonical `firefly:<id>`.
    pub transaction_id: String,
    /// The category the owner saw; `None` when it was uncategorised.
    pub expected_category: Option<String>,
    pub category: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategorySaved {
    pub saved: bool,
    pub category: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Approve,
    Correct,
    Reject,
    /// "No idea": set aside until another decision resumes it.
    Defer,
}

/// One category decision on a pending proposal (a `/review/stream` item).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDecision {
    pub proposal_id: String,
    pub decision: ReviewAction,
    /// Required for approve and correct, absent otherwise.
    #[serde(default)]
    pub category: Option<String>,
    pub expected_proposed_category: Option<String>,
    pub expected_original_category: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Saved,
    /// The proposal or transaction changed; the decision was dropped.
    Changed,
    /// Not saved; the owner can try again.
    Retry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionResult {
    pub proposal_id: String,
    pub status: DecisionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionsSaved {
    pub saved: usize,
    pub results: Vec<DecisionResult>,
}

/// One row of "Save all" in the current daily batch (`/review/batch`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchDecision {
    pub proposal_id: String,
    pub category: String,
    pub expected_proposed_category: String,
    pub expected_original_category: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSaved {
    pub saved: usize,
}

/// Save the owner's details & tags on a reviewed transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailsSave {
    pub proposal_id: String,
    /// The revision the owner edited.
    pub revision: u32,
    pub values: crate::gateway::OwnerDetails,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audited {
    pub audited: bool,
}

/// Move every transaction of one category to another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consolidation {
    pub source: String,
    pub target: String,
    /// From the preview; required to apply.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationPreview {
    pub source: String,
    pub target: String,
    pub count: usize,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationApplied {
    pub applied: usize,
    pub total: usize,
    pub complete: bool,
    pub refreshed: bool,
    /// Why it stopped early.
    #[serde(default)]
    pub error: Option<String>,
}
