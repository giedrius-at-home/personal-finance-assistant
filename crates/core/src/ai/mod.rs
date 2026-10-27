//! AI suggestions: category proposals, the second-opinion check, daily
//! automatic approval, spending-group suggestions and public merchant
//! research (Python `categorisation_runtime.py`, `categorisation_service.py`,
//! `category_check.py`, `daily_review.py`, `spending_group_suggestions.py`,
//! `research.py`, `integrations/typesafe.py`).
//!
//! Everything here is prompts, parsing, ranking and storage. The model and
//! web calls go through the backend traits below, which `finance-ai`
//! implements with blocking HTTP clients, so this crate stays free of HTTP.

pub mod app;
pub mod categorise;
pub mod check;
pub mod daily;
pub mod difflib;
pub mod extract;
pub mod groups;
pub mod history;
pub mod pyjson;
pub mod research;
pub mod retrieval;
pub mod runner;
pub mod settings;
pub mod text;
pub mod typesafe;

pub use categorise::{Categoriser, ModelResult};
pub use check::Checker;
pub use groups::GroupRanker;
pub use research::Fetch;
pub use typesafe::{ChoiceRequest, ChoiceResult};

/// A model call that failed. Never carries prompt or response text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    #[error("the model could not be reached")]
    Unavailable,
    #[error("the model returned HTTP {0}")]
    Status(u16),
    #[error("the model response violated the contract")]
    Protocol,
    #[error("{0}")]
    Invalid(&'static str),
}

/// A local text model (Ollama `api/generate` with JSON output).
pub trait Generate {
    fn generate(&self, prompt: &str, system: Option<&str>) -> Result<String, BackendError>;
}

/// A local embedding model (Ollama `api/embed`).
pub trait Embed {
    /// One vector per input, all of the same length.
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>, BackendError>;
}

/// TypeSafe System One: one closed Choice question.
pub trait Choose {
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceResult, BackendError>;
}
