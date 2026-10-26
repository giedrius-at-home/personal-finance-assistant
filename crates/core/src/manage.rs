//! Reads behind the Manage screens: the workspace the Import, Status and
//! Settings screens share (Python `/v1/workspace`), and the Banks list.

use std::collections::{HashMap, HashSet};

use finance_storage::Store;
use finance_types::manage::{BankAccount, BanksReport, ImportRun, ManageWorkspace, ManagedAccount};
use rusqlite::{Connection, OptionalExtension};

use crate::writes::csv_import::{self, Target};
use crate::writes::settings;
use crate::{Result, Snapshot};

/// The CSV source every import records in `import_runs`.
pub const CSV_SOURCE: &str = "csv-import";

/// Ledger id → currency of every own account.
fn currencies(snap: &Snapshot) -> HashMap<i64, String> {
    snap.accounts
        .iter()
        .map(|a| (a.ledger_id, a.currency().code.clone()))
        .collect()
}

/// The asset accounts a CSV import can target.
pub fn import_targets(conn: &Connection, snap: &Snapshot) -> Result<Vec<Target>> {
    csv_import::targets(conn, &currencies(snap))
}

/// Canonical ids of the accounts a salary can be paid into.
pub fn salary_accounts(snap: &Snapshot) -> HashSet<String> {
    snap.accounts.iter().map(|a| a.account_id.clone()).collect()
}

fn import_run(conn: &Connection, filter: &str) -> Result<Option<ImportRun>> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            &format!(
                "SELECT source, finished_at, stats FROM import_runs {filter} ORDER BY id DESC LIMIT 1"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(source, finished_at, stats)| ImportRun {
        source,
        finished_at,
        stats: serde_json::from_str(&stats).unwrap_or(serde_json::Value::Null),
    }))
}

pub fn workspace(
    store: &Store,
    snap: &Snapshot,
    last_write: Option<String>,
) -> Result<ManageWorkspace> {
    let conn = store.connection();
    let status = store.status()?;
    Ok(ManageWorkspace {
        csv_enabled: true,
        notifications_available: settings::NOTIFICATIONS_AVAILABLE,
        settings: settings::current(conn)?,
        revisions: settings::revisions(conn)?,
        accounts: snap
            .accounts
            .iter()
            .map(|a| ManagedAccount {
                id: a.ledger_id.to_string(),
                account_id: a.account_id.clone(),
                name: a.name.clone(),
                currency: a.currency().code.clone(),
            })
            .collect(),
        last_import: import_run(conn, "")?,
        last_csv_import: import_run(conn, &format!("WHERE source = '{CSV_SOURCE}'"))?,
        last_write,
        transactions: status.transactions,
        first_date: status.first_date,
        last_date: status.last_date,
    })
}

pub fn banks(store: &Store) -> Result<BanksReport> {
    let conn = store.connection();
    let active: HashMap<i64, bool> = conn
        .prepare("SELECT id, active FROM accounts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut last_csv: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT json_extract(stats, '$.account_id'), max(finished_at) FROM import_runs \
         WHERE source = '{CSV_SOURCE}' GROUP BY 1"
    ))?;
    for row in stmt.query_map([], |r| {
        Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, String>(1)?))
    })? {
        if let (Some(id), at) = row? {
            last_csv.insert(id, at);
        }
    }
    let accounts = store
        .accounts()?
        .into_iter()
        .map(|account| BankAccount {
            active: active.get(&account.id).copied().unwrap_or(true),
            bank: csv_import::bank_of(&account.name).map(str::to_owned),
            last_csv_import: last_csv.get(&account.id).cloned(),
            account,
        })
        .collect();
    Ok(BanksReport {
        accounts,
        last_import: import_run(conn, "")?,
    })
}
