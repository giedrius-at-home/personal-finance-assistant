//! An in-memory database with the gateway tables and a tiny ledger, for the
//! write tests.

use std::path::Path;

use finance_storage::Store;
use jiff::Zoned;
use jiff::tz::TimeZone;
use rusqlite::params;

use super::Writer;
use crate::{Result, Snapshot};

pub struct Db {
    pub store: Store,
}

pub fn db() -> Db {
    let store = Store::open(Path::new(":memory:")).expect("in-memory store");
    store
        .connection()
        .execute_batch(include_str!("test_fixture.sql"))
        .expect("fixture");
    Db { store }
}

pub fn now() -> Zoned {
    "2026-10-26T20:00:00Z"
        .parse::<jiff::Timestamp>()
        .expect("instant")
        .to_zoned(TimeZone::get("Europe/Amsterdam").expect("zone"))
}

impl Db {
    pub fn snap(&self) -> Snapshot {
        Snapshot::load(&self.store).expect("snapshot")
    }

    /// Run `f` in one write transaction, committed only when it succeeds.
    pub fn write<T>(&mut self, f: impl FnOnce(&Writer<'_>, &Snapshot) -> Result<T>) -> Result<T> {
        let snap = self.snap();
        let writer = Writer {
            tx: self.store.connection_mut().transaction()?,
            now: now(),
        };
        let value = f(&writer, &snap)?;
        writer.tx.commit()?;
        Ok(value)
    }

    pub fn one<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.store
            .connection()
            .query_row(sql, [], |r| r.get(0))
            .expect("one row")
    }

    /// A pending model proposal for ledger transaction `ledger`.
    pub fn proposal(&self, id: &str, ledger: i64, proposed: Option<&str>) {
        let payload = serde_json::json!({
            "proposal_id": id,
            "source": {"system": "firefly_iii", "entity": "transaction", "external_id": ledger.to_string()},
            "original_category": null,
            "proposed_category": proposed,
            "method": if proposed.is_some() { "model" } else { "abstain" },
            "confidence": "0.95",
            "category_options": [{"category": "Groceries", "probability": "0.95"}, {"category": "Snacks", "probability": "0.05"}],
            "reasons": [],
            "model_version": "jev-latest",
            "policy_version": "p",
            "vocabulary_version": "v",
            "feature_hash": "f".repeat(64),
            "normalized_merchant": "lidl utrecht",
            "status": "pending",
            "created_at": "2026-10-21T10:00:00Z",
        });
        self.store
            .connection()
            .execute(
                "INSERT INTO category_proposals VALUES (?1, 'firefly_iii', 'transaction', ?2, ?3, \
                 'pending', NULL, ?4, '2026-10-21T10:00:00+00:00', '2026-10-21T10:00:00+00:00')",
                params![id, ledger.to_string(), "f".repeat(64), payload.to_string()],
            )
            .expect("proposal");
    }

    /// An open daily review entry for ledger transaction `ledger`.
    pub fn entry(&self, ledger: i64, extra: serde_json::Value) {
        let mut entry = serde_json::json!({
            "transaction_id": format!("firefly:{ledger}"),
            "batch_date": "2026-10-21",
            "first_seen": "2026-10-21T10:00:00Z",
            "due_at": "2026-11-20T10:00:00Z",
        });
        if let (Some(map), Some(more)) = (entry.as_object_mut(), extra.as_object()) {
            map.extend(more.clone());
        }
        self.store
            .connection()
            .execute(
                "INSERT INTO daily_review_entries VALUES (?1, ?2)",
                params![format!("firefly:{ledger}"), entry.to_string()],
            )
            .expect("entry");
    }
}
