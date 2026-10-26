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

/// Ordered schema migrations; the index + 1 is the version.
const MIGRATIONS: &[&str] = &[include_str!("schema.sql")];

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
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
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
        for (index, sql) in MIGRATIONS.iter().enumerate() {
            let version = index as i64 + 1;
            if version <= current {
                continue;
            }
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
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
