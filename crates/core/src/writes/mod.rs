//! Writes the screens make: one module per area, each a port of the Python
//! route that did the same (`web.py`, `closure_routes.py`). Every write runs
//! inside one SQLite transaction through [`Writer`]; the service backs up the
//! database before the first write of each app session.

use jiff::{Timestamp, Zoned};
use rusqlite::{Transaction, params};
use serde::Serialize;

use crate::{Error, Result};

/// One write transaction plus the moment it happens.
#[derive(Debug)]
pub struct Writer<'c> {
    pub tx: Transaction<'c>,
    pub now: Zoned,
}

impl Writer<'_> {
    /// `now` as Python's `datetime.now(UTC).isoformat()` wrote it.
    pub fn stamp(&self) -> String {
        utc_stamp(self.now.timestamp())
    }

    /// Insert or replace one JSON payload row: `sql` takes the payload as `?1`
    /// followed by `keys`.
    pub fn put_json<T: Serialize>(
        &self,
        sql: &str,
        payload: &T,
        keys: &[&dyn rusqlite::ToSql],
    ) -> Result<()> {
        let json = serde_json::to_string(payload).map_err(|e| Error::Invalid(e.to_string()))?;
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&json];
        values.extend_from_slice(keys);
        self.tx.execute(sql, values.as_slice())?;
        Ok(())
    }

    /// Record that the ledger changed, so status screens show when.
    pub fn record(&self, source: &str, stats: &serde_json::Value) -> Result<()> {
        self.tx.execute(
            "INSERT INTO import_runs (source, stats) VALUES (?1, ?2)",
            params![source, stats.to_string()],
        )?;
        Ok(())
    }
}

/// A payment id as the ledger keeps it: the transaction search names a
/// payment by its bare ledger id, the ledger as `firefly:<id>`.
pub(crate) fn ledger_id(transactions: &[crate::model::Tx], id: &str) -> String {
    let ledger = format!("firefly:{id}");
    if transactions.iter().any(|t| t.transaction_id == ledger) {
        ledger
    } else {
        id.to_owned()
    }
}

/// `2026-10-26T20:00:00.123456Z`, the format the gateway tables hold.
pub fn utc_stamp(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// A short, data-free error the UI shows as is.
pub fn invalid(message: impl Into<String>) -> Error {
    Error::Rejected(message.into())
}

pub mod bills;
pub mod budget;
pub mod closure;
pub mod csv_import;
#[cfg(test)]
pub(crate) mod fixture;
pub mod groups;
pub mod plans;
pub mod review;
pub mod settings;
pub mod transactions;
pub mod wishes;
