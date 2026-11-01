//! Local SQLite store.
//!
//! One database file holds both the gateway tables copied from the old Python
//! service and the ledger tables that replace Firefly III. This crate owns the
//! schema, the ledger import and every read query the app makes.

mod ledger;
mod queries;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

pub use ledger::{
    Ledger, LedgerAccount, LedgerBill, LedgerCategory, LedgerPosting, LedgerTransaction,
    NewTransaction, counterparty_account, delete_transaction, insert_transaction, search_text,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored value is not valid: {0}")]
    Corrupt(String),
    #[error("no application data folder on this system")]
    NoDataDir,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One schema migration: a SQL script, or a step that needs to look at the
/// database first.
enum Migration {
    Sql(&'static str),
    Run(fn(&Connection) -> Result<()>),
}

/// Ordered schema migrations; the index + 1 is the version.
const MIGRATIONS: &[Migration] = &[
    Migration::Sql(include_str!("schema.sql")),
    Migration::Sql(include_str!("bank_sync.sql")),
    Migration::Sql(include_str!("notify.sql")),
    Migration::Sql(include_str!("ai.sql")),
    Migration::Sql(include_str!("wishes.sql")),
    Migration::Run(repair_pocket_transfers),
];

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |r| r.get(0),
    )?)
}

/// Migration 6: pocket transfers the bank sync booked as a purchase and a
/// gift become one transfer, and "To <savings pocket>" withdrawals with no
/// known counterparty become transfers to the savings account routed via
/// that account (`pockets.sql`), for rows from 1 November 2026 on. Idempotent;
/// safe to run again.
///
/// The savings routes come from `account_roles`, a gateway table a fresh
/// store does not have, so they go into a temporary table here and the SQL
/// stays free of that dependency.
pub fn repair_pocket_transfers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS temp.pocket_routes;
         CREATE TEMP TABLE pocket_routes (via INTEGER NOT NULL, target INTEGER NOT NULL);",
    )?;
    if has_table(conn, "account_roles")? {
        conn.execute_batch(
            "INSERT INTO pocket_routes (via, target)
             SELECT CAST(substr(json_extract(payload_json, '$.via_account_id'), 9) AS INTEGER),
                    CAST(substr(json_extract(payload_json, '$.account_id'), 9) AS INTEGER)
             FROM account_roles
             JOIN accounts a
               ON a.id = CAST(substr(json_extract(payload_json, '$.account_id'), 9) AS INTEGER)
              AND a.kind = 'asset' AND a.active = 1
             WHERE json_extract(payload_json, '$.role') = 'savings'
               AND json_extract(payload_json, '$.via_account_id') LIKE 'firefly:%'
               AND json_extract(payload_json, '$.account_id') LIKE 'firefly:%';",
        )?;
    }
    conn.execute_batch(include_str!("pockets.sql"))?;
    conn.execute_batch("DROP TABLE temp.pocket_routes;")?;
    Ok(())
}

/// `~/Library/Application Support/Finance`.
pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir().ok_or(Error::NoDataDir)?.join("Finance"))
}

pub fn default_db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("finance.sqlite3"))
}

/// Reports group and sort large gateway tables (59k category proposals):
/// keep temporary b-trees in memory and give the page cache room.
fn tune(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -65536)?;
    conn.pragma_update(None, "mmap_size", 268_435_456)?;
    Ok(())
}

#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open for reading only; the UI never writes in this phase. The file is
    /// opened read-write because a WAL database needs its shared-memory file,
    /// and `query_only` then refuses every write.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        tune(&conn)?;
        Ok(Self { conn })
    }

    /// Open for writing and bring the schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "secure_delete", true)?;
        tune(&conn)?;
        let store = Self { conn };
        store.migrate(Some(path))?;
        Ok(store)
    }

    /// Before a migration that rewrites ledger rows (`Migration::Run`), a
    /// copy of the file: `<db>.before-migration-<n>`. Nothing for an
    /// in-memory store or when the copy already exists.
    fn backup_before(&self, path: Option<&Path>, version: i64) -> Result<()> {
        let Some(path) = path.filter(|p| p.is_file()) else {
            return Ok(());
        };
        let copy = path.with_file_name(format!(
            "{}.before-migration-{version}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("finance.sqlite3")
        ));
        if copy.exists() {
            return Ok(());
        }
        let Some(target) = copy.to_str() else {
            return Ok(());
        };
        self.conn.execute("VACUUM INTO ?1", [target])?;
        Ok(())
    }

    fn migrate(&self, path: Option<&Path>) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            )",
        )?;
        let current: i64 = self.conn.query_row(
            "SELECT coalesce(max(version), 0) FROM app_migrations",
            [],
            |r| r.get(0),
        )?;
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            let version = index as i64 + 1;
            if version <= current {
                continue;
            }
            if matches!(migration, Migration::Run(_)) {
                self.backup_before(path, version)?;
            }
            let tx = self.conn.unchecked_transaction()?;
            match migration {
                Migration::Sql(sql) => tx.execute_batch(sql)?,
                Migration::Run(step) => step(&tx)?,
            }
            tx.execute(
                "INSERT INTO app_migrations (version) VALUES (?1)",
                [version],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    /// The connection, for the computations in `finance-core` that read the
    /// gateway tables directly.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// The connection, for write transactions in `finance-core`.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// `PRAGMA integrity_check` returned "ok".
    pub fn is_intact(&self) -> Result<bool> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok(result == "ok")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three November shapes: a pocket top-up seen from both sides, a
    /// transfer to the unconnected savings pocket, and a real purchase.
    fn november() -> Store {
        let store = Store::open(Path::new(":memory:")).expect("store");
        store
            .connection()
            .execute_batch(
                "CREATE TABLE account_roles (account_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
                 INSERT INTO accounts (id, name, kind) VALUES
                    (76, 'Revolut', 'asset'), (415, 'Revolut Groceries', 'asset'),
                    (413, 'Revolut Savings', 'asset'), (10, '(unknown destination)', 'expense'),
                    (63, '(unknown source)', 'revenue'), (20, 'Bakery', 'expense');
                 INSERT INTO categories (id, name) VALUES (1, 'Groceries');
                 INSERT INTO transactions (id, kind, date, booked_at, description, source_account_id,
                    destination_account_id, category_id, external_id, tags) VALUES
                    (1, 'withdrawal', '2026-11-01', '2026-11-01T00:00:00', 'To EUR MB:a4ba', 76, 10, 1, 'X', '[\"booked\"]'),
                    (2, 'deposit', '2026-11-01', '2026-11-01T00:00:00', 'To EUR MB:a4ba', 63, 415, NULL, 'X', '[\"booked\"]'),
                    (3, 'withdrawal', '2026-11-01', '2026-11-01T00:00:00', 'To Instant Access Savings', 76, 10, 1, 'Y', '[]'),
                    (4, 'withdrawal', '2026-11-01', '2026-11-01T00:00:00', 'Bakery', 76, 20, 1, 'Z', '[]');
                 INSERT INTO postings (transaction_id, account_id, amount, currency) VALUES
                    (1, 76, '-300.00', 'EUR'), (1, 10, '300.00', 'EUR'),
                    (2, 63, '-300.00', 'EUR'), (2, 415, '300.00', 'EUR'),
                    (3, 76, '-913.82', 'EUR'), (3, 10, '913.82', 'EUR'),
                    (4, 76, '-4.50', 'EUR'), (4, 20, '4.50', 'EUR');
                 INSERT INTO transactions_search (rowid, text) VALUES
                    (1, 'To EUR MB:a4ba • Revolut • (unknown destination) • Groceries'),
                    (2, 'To EUR MB:a4ba • (unknown source) • Revolut Groceries'),
                    (3, 'To Instant Access Savings • Revolut • (unknown destination)'),
                    (4, 'Bakery • Revolut • Bakery');
                 INSERT INTO account_roles VALUES ('firefly:413',
                    '{\"account_id\":\"firefly:413\",\"role\":\"savings\",\"goal_id\":null,\"via_account_id\":\"firefly:76\",\"confirmed_at\":\"2026-10-01T00:00:00Z\"}');",
            )
            .expect("fixture");
        store
    }

    fn one<T: rusqlite::types::FromSql>(store: &Store, sql: &str) -> T {
        store
            .connection()
            .query_row(sql, [], |r| r.get(0))
            .expect("one row")
    }

    #[test]
    fn pocket_repair_pairs_legs_and_routes_savings() {
        let store = november();
        repair_pocket_transfers(store.connection()).unwrap();
        // Twice: the repair finds nothing left to do.
        repair_pocket_transfers(store.connection()).unwrap();

        let pair: String = one(
            &store,
            "SELECT kind || ':' || source_account_id || '>' || destination_account_id              || ':' || coalesce(category_id, 'none') FROM transactions WHERE id = 1",
        );
        assert_eq!(pair, "transfer:76>415:none");
        let gone: i64 = one(
            &store,
            "SELECT count(*) FROM transactions WHERE id = 2              UNION ALL SELECT count(*) FROM postings WHERE transaction_id = 2              UNION ALL SELECT count(*) FROM transactions_search WHERE rowid = 2",
        );
        assert_eq!(gone, 0);
        let postings: String = one(
            &store,
            "SELECT group_concat(account_id || '=' || amount, ',') FROM              (SELECT account_id, amount FROM postings WHERE transaction_id = 1 ORDER BY id)",
        );
        assert_eq!(postings, "76=-300.00,415=300.00");

        let savings: String = one(
            &store,
            "SELECT kind || ':' || source_account_id || '>' || destination_account_id              || ':' || coalesce(category_id, 'none') FROM transactions WHERE id = 3",
        );
        assert_eq!(savings, "transfer:76>413:none");
        let routed: i64 = one(
            &store,
            "SELECT count(*) FROM postings WHERE transaction_id = 3 AND account_id = 413 AND amount = '913.82'",
        );
        assert_eq!(routed, 1);

        // The purchase is untouched.
        let bakery: String = one(
            &store,
            "SELECT kind || ':' || destination_account_id || ':' || category_id FROM transactions WHERE id = 4",
        );
        assert_eq!(bakery, "withdrawal:20:1");

        let found: i64 = one(
            &store,
            "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH '\"Revolut Groceries\"' AND rowid = 1",
        );
        assert_eq!(found, 1);
        let text_has_both: i64 = one(
            &store,
            "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH '\"Revolut Savings\"' AND rowid = 3",
        );
        assert_eq!(text_has_both, 1);
        let stale: i64 = one(
            &store,
            "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH '\"unknown destination\"' AND rowid IN (1, 3)",
        );
        assert_eq!(stale, 0);
        let version: i64 = one(&store, "SELECT max(version) FROM app_migrations");
        assert_eq!(version, 6);
    }

    #[test]
    fn pocket_repair_leaves_two_candidate_savings_alone() {
        let store = november();
        store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (414, 'Revolut Vault', 'asset');
                 INSERT INTO account_roles VALUES ('firefly:414',
                    '{\"account_id\":\"firefly:414\",\"role\":\"savings\",\"goal_id\":null,\"via_account_id\":\"firefly:76\",\"confirmed_at\":null}');",
            )
            .unwrap();
        repair_pocket_transfers(store.connection()).unwrap();
        let savings: String = one(
            &store,
            "SELECT kind || ':' || destination_account_id FROM transactions WHERE id = 3",
        );
        assert_eq!(savings, "withdrawal:10");
        // Rule 2 still pairs the pocket legs.
        let pair: String = one(
            &store,
            "SELECT kind || ':' || destination_account_id FROM transactions WHERE id = 1",
        );
        assert_eq!(pair, "transfer:415");
    }
}
