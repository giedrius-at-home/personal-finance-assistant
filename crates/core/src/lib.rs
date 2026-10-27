//! Finance logic: the canonical model built from the ledger, and the reports
//! and decisions every screen shows. A Rust port of the old Python service;
//! each module names the Python module it replaces.

pub mod allocation;
pub mod bank_sync;
pub mod budget;
pub mod cash;
pub mod catalog;
pub mod closure;
pub mod context;
pub mod daily;
pub mod dashboard;
pub mod forecast;
pub mod goals;
pub mod groups;
pub mod limits;
pub mod load;
pub mod manage;
pub mod model;
pub mod outlook;
pub mod period;
pub mod personal;
pub mod plans;
pub mod reporting;
pub mod review;
pub mod service;
pub mod subscriptions;
pub mod text;
pub mod transfers;
pub mod trends;
pub mod writes;

pub use context::Ctx;
pub use load::Snapshot;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] finance_storage::Error),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored value is not valid: {0}")]
    Invalid(String),
    /// A request the owner can fix; the message is shown as is.
    #[error("{0}")]
    Rejected(String),
}

pub type Result<T> = std::result::Result<T, Error>;
