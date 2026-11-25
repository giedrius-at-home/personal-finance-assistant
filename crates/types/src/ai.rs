//! AI suggestions: the owner's settings (Manage → Settings), the runner's
//! status, spending-group suggestions and public merchant research
//! (Python `categorisation_runtime`, `spending_group_suggestions`,
//! `domain/research`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Which model proposes categories.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classifier {
    /// No suggestions are made.
    #[default]
    Off,
    /// Mistral AI (EU-hosted); the API key lives in the secret store.
    /// Settings saved when this was the local Ollama read as Mistral.
    #[serde(alias = "ollama")]
    Mistral,
    /// TypeSafe System One ("Jev"); the API key lives in the secret store.
    #[serde(rename = "typesafe")]
    TypeSafe,
}

pub const DEFAULT_MISTRAL_MODEL: &str = "mistral-small-latest";
pub const MISTRAL_EMBED_MODEL: &str = "mistral-embed";
pub const DEFAULT_TYPESAFE_MODEL: &str = "jev-latest";
/// `PFA_CATEGORY_SUGGESTION_BATCH_SIZE`.
pub const DEFAULT_BATCH_SIZE: u32 = 20;

/// `ai_settings.payload_json`. Never holds a secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub classifier: Classifier,
    /// The Mistral chat model that proposes categories and writes Gisel's
    /// lines.
    #[serde(default = "AiSettings::default_model")]
    pub mistral_model: String,
    /// Find similar past transactions with `mistral-embed`; off: text
    /// matches only.
    #[serde(default)]
    pub embeddings: bool,
    #[serde(default = "AiSettings::default_typesafe_model")]
    pub typesafe_model: String,
    /// Look up public merchant names on Wikipedia (and SEC EDGAR).
    #[serde(default)]
    pub research: bool,
    /// Save Jev suggestions at the daily review threshold without asking.
    #[serde(default)]
    pub auto_apply: bool,
    #[serde(default = "AiSettings::default_batch")]
    pub batch_size: u32,
}

impl AiSettings {
    fn default_model() -> String {
        DEFAULT_MISTRAL_MODEL.into()
    }
    fn default_typesafe_model() -> String {
        DEFAULT_TYPESAFE_MODEL.into()
    }
    fn default_batch() -> u32 {
        DEFAULT_BATCH_SIZE
    }

    /// The model whose name proposals carry (`classifier_model`).
    pub fn classifier_model(&self) -> &str {
        match self.classifier {
            Classifier::TypeSafe => &self.typesafe_model,
            _ => &self.mistral_model,
        }
    }

    /// The embedding model, when embeddings are on.
    pub fn embedding(&self) -> Option<&str> {
        self.embeddings.then_some(MISTRAL_EMBED_MODEL)
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults are valid")
    }
}

/// The outcome of the last suggestion pass.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiRun {
    /// RFC 3339.
    pub finished_at: String,
    /// Transactions sent to the classifier.
    pub suggested: usize,
    /// Categories saved by automatic approval.
    pub applied: usize,
    pub millis: u64,
    /// A short, data-free reason when the pass stopped.
    #[serde(default)]
    pub error: Option<String>,
}

/// The "AI suggestions" settings card.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiStatus {
    pub settings: AiSettings,
    /// Whether a Mistral API key is stored.
    #[serde(default)]
    pub mistral_key_set: bool,
    pub typesafe_key_set: bool,
    pub running: bool,
    #[serde(default)]
    pub last_run: Option<AiRun>,
}

/// A manual "Suggest categories" pass.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionPass {
    pub suggested: usize,
    pub applied: usize,
}

// ---------------------------------------------------------------------------
// Spending groups (`spending_group_suggestions.GroupSuggestion`)

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedGroupOption {
    pub group: String,
    pub probability: Decimal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSuggestion {
    pub group: Option<String>,
    #[serde(default)]
    pub options: Vec<RankedGroupOption>,
    #[serde(default)]
    pub confidence: Option<Decimal>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub public_research: Option<MerchantResearchResult>,
}

// ---------------------------------------------------------------------------
// Public research (`domain/research.py`)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPurpose {
    MerchantIdentity,
    MarketCompanyProfile,
    MarketRegulatoryFilings,
    MarketRiskDisclosures,
}

impl ResearchPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MerchantIdentity => "merchant_identity",
            Self::MarketCompanyProfile => "market_company_profile",
            Self::MarketRegulatoryFilings => "market_regulatory_filings",
            Self::MarketRiskDisclosures => "market_risk_disclosures",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStatus {
    Live,
    FreshCache,
    StaleCache,
    Disabled,
    Unavailable,
    Abstained,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCitation {
    pub title: String,
    pub publisher: String,
    pub url: String,
    pub retrieved_at: String,
    pub expires_at: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntrustedPublicDocument {
    pub trust_label: String,
    pub citation: PublicCitation,
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchResult {
    pub schema_version: String,
    pub policy_version: String,
    pub query_key: String,
    pub purpose: ResearchPurpose,
    pub status: ResearchStatus,
    pub documents: Vec<UntrustedPublicDocument>,
    #[serde(default)]
    pub error_code: Option<String>,
    pub generated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerchantResearchResult {
    pub schema_version: String,
    pub public_name: String,
    pub country_code: Option<String>,
    pub research: ResearchResult,
    #[serde(default)]
    pub likely_business_type: Option<String>,
    pub interpretation: String,
    pub category_change_allowed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketIdentifierKind {
    Ticker,
    Cik,
    Isin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketQuestion {
    CompanyProfile,
    RegulatoryFilings,
    RiskDisclosures,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketResearchResult {
    pub schema_version: String,
    pub identifier: String,
    pub identifier_kind: MarketIdentifierKind,
    pub question: MarketQuestion,
    pub research: ResearchResult,
    pub personalized_advice: bool,
    pub trade_action_allowed: bool,
}
