//! One-off migration from the `laptopthree` finance stack.
//!
//! Copies the gateway SQLite database as-is and the Firefly III ledger as JSON
//! lines over SSH, writes them into a fresh local database, proves that every
//! account's postings and sums match MariaDB, and only then swaps the new file
//! into place. Nothing is written on the remote host.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use finance_storage::{
    Ledger, LedgerAccount, LedgerBill, LedgerCategory, LedgerPosting, LedgerTransaction, Store,
};
use rust_decimal::Decimal;
use serde::Deserialize;

const EXPORT_SQL: &str = include_str!("../sql/export.sql");
const VERIFY_SQL: &str = include_str!("../sql/verify.sql");

const GATEWAY: &str = "personal-finance-assistant-gateway-1";
const FIREFLY_DB: &str = "personal-finance-assistant-firefly-db-1";
const GATEWAY_DB: &str = "/var/lib/pfa/assistant.sqlite3";

#[derive(Debug, Parser)]
#[command(about = "Copy the finance data from laptopthree into the local app database")]
struct Args {
    /// SSH destination of the old finance host.
    #[arg(long, default_value = "rafael@laptopthree.home")]
    host: String,
    /// Known-hosts name the host key is stored under.
    #[arg(long, default_value = "100.72.43.52")]
    host_key_alias: String,
    #[arg(long, default_value = "~/.ssh/id_ed25519_home")]
    identity: String,
    /// Target database (default: ~/Library/Application Support/Finance/finance.sqlite3).
    #[arg(long)]
    db: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Row {
    Account(LedgerAccount),
    Category(LedgerCategory),
    Bill(LedgerBill),
    Transaction(LedgerTransaction),
    Posting(LedgerPosting),
}

#[derive(Debug, Deserialize)]
struct RemoteCheck {
    legacy_id: i64,
    currency: String,
    postings: u64,
    sum: Decimal,
}

struct Remote<'a>(&'a Args);

impl Remote<'_> {
    fn command(&self, remote: &str) -> Command {
        let identity = self
            .0
            .identity
            .replacen('~', &std::env::var("HOME").unwrap_or_default(), 1);
        let mut cmd = Command::new("ssh");
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "IdentitiesOnly=yes",
        ])
        .args(["-o", &format!("HostKeyAlias={}", self.0.host_key_alias)])
        .args(["-i", &identity, &self.0.host, remote]);
        cmd
    }

    /// Run `remote`, feeding `input` on stdin, and return stdout.
    fn run(&self, remote: &str, input: Option<&str>) -> Result<Vec<u8>> {
        let mut child = self
            .command(remote)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start ssh")?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .context("ssh stdin")?
                .write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            // Remote stderr can carry server details but never data values.
            bail!(
                "remote command failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }

    fn mariadb(&self, sql: &str) -> Result<Vec<u8>> {
        let remote = format!(
            "docker exec -i {FIREFLY_DB} sh -c 'MYSQL_PWD=\"$MYSQL_PASSWORD\" exec mariadb -u\"$MYSQL_USER\" \
             --batch --raw --skip-column-names --default-character-set=utf8mb4 \"$MYSQL_DATABASE\"'"
        );
        self.run(&remote, Some(sql))
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let remote = Remote(&args);
    let target = match &args.db {
        Some(path) => path.clone(),
        None => finance_storage::default_db_path()?,
    };
    let dir = target.parent().context("database path has no folder")?;
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let staging = target.with_extension("sqlite3.migrating");
    remove_db_files(&staging)?;

    eprintln!("1/4 copying gateway database from {}", args.host);
    let bytes = remote.run(
        &format!(
            "docker exec {GATEWAY} python3 -c 'import sqlite3, sys; \
             c = sqlite3.connect(\"file:{GATEWAY_DB}?mode=ro\", uri=True); \
             sys.stdout.buffer.write(c.serialize())'"
        ),
        None,
    )?;
    ensure!(
        bytes.starts_with(b"SQLite format 3\0"),
        "gateway copy is not a SQLite database"
    );
    write_private(&staging, &bytes)?;
    eprintln!("    {:.1} MB", bytes.len() as f64 / 1e6);

    eprintln!("2/4 exporting Firefly ledger");
    let ledger = parse_ledger(&remote.mariadb(EXPORT_SQL)?)?;
    eprintln!(
        "    {} accounts, {} categories, {} bills, {} transactions, {} postings",
        ledger.accounts.len(),
        ledger.categories.len(),
        ledger.bills.len(),
        ledger.transactions.len(),
        ledger.postings.len()
    );

    eprintln!("3/4 writing local database");
    let mut store = Store::open(&staging)?;
    ensure!(
        store.is_intact()?,
        "copied gateway database failed integrity_check"
    );
    store.replace_ledger(&ledger, &format!("firefly@{}", args.host))?;

    eprintln!("4/4 verifying against MariaDB");
    verify(&store, &remote.mariadb(VERIFY_SQL)?)?;
    drop(store);

    if target.exists() {
        let backup = target.with_extension("sqlite3.before-migrate");
        remove_db_files(&backup)?;
        fs::rename(&target, &backup)?;
        eprintln!("    previous database kept at {}", backup.display());
    }
    remove_db_files_except_main(&target)?;
    fs::rename(&staging, &target)?;
    remove_db_files(&staging)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
    eprintln!("done: {}", target.display());
    Ok(())
}

fn parse_ledger(output: &[u8]) -> Result<Ledger> {
    let mut ledger = Ledger::default();
    for (index, line) in BufReader::new(output).lines().enumerate() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        // Report only the line number: lines hold financial data.
        let row: Row =
            serde_json::from_str(&line).with_context(|| format!("export line {}", index + 1))?;
        match row {
            Row::Account(a) => ledger.accounts.push(a),
            Row::Category(c) => ledger.categories.push(c),
            Row::Bill(b) => ledger.bills.push(b),
            Row::Transaction(t) => ledger.transactions.push(t),
            Row::Posting(p) => ledger.postings.push(p),
        }
    }
    ensure!(
        !ledger.transactions.is_empty(),
        "Firefly export returned no transactions"
    );
    Ok(ledger)
}

fn verify(store: &Store, output: &[u8]) -> Result<()> {
    let mut expected: BTreeMap<i64, (u64, BTreeMap<String, Decimal>)> = BTreeMap::new();
    for line in BufReader::new(output).lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let check: RemoteCheck = serde_json::from_str(&line).context("verify row")?;
        let entry = expected.entry(check.legacy_id).or_default();
        entry.0 += check.postings;
        *entry.1.entry(check.currency).or_default() += check.sum;
    }
    let actual: BTreeMap<i64, (u64, BTreeMap<String, Decimal>)> = store
        .account_checks()?
        .into_iter()
        .map(|c| {
            (
                c.legacy_id,
                (
                    c.postings,
                    c.sums.into_iter().map(|m| (m.currency, m.amount)).collect(),
                ),
            )
        })
        .collect();

    let mut mismatches = 0;
    for (id, (count, sums)) in &expected {
        let ok = actual.get(id).is_some_and(|(n, s)| {
            n == count
                && sums.len() == s.len()
                && sums
                    .iter()
                    .all(|(cur, sum)| s.get(cur).is_some_and(|a| a.normalize() == sum.normalize()))
        });
        if !ok {
            mismatches += 1;
            eprintln!("    account {id}: postings or sums differ");
        }
    }
    let extra = actual
        .keys()
        .filter(|id| !expected.contains_key(id))
        .count();
    ensure!(
        mismatches == 0 && extra == 0,
        "{mismatches} accounts differ, {extra} unexpected accounts"
    );
    eprintln!("    {} accounts match exactly", expected.len());
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sidecars(path: &Path) -> [PathBuf; 2] {
    let with = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    [with("-wal"), with("-shm")]
}

fn remove_db_files_except_main(path: &Path) -> Result<()> {
    for file in sidecars(path) {
        if file.exists() {
            fs::remove_file(file)?;
        }
    }
    Ok(())
}

fn remove_db_files(path: &Path) -> Result<()> {
    remove_db_files_except_main(path)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
