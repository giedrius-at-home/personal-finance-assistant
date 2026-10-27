//! Bank sync through Enable Banking, the port of the old importer
//! (`bank_imports.py`, `bank_import_worker.php` and the Firefly data
//! importer's Enable Banking routine). The HTTP client lives in the app; this
//! module checks the consent, pages through the transactions, writes them
//! into the ledger and keeps the daily cadence.

mod connection;
mod entry;
mod import;
pub mod link;

use std::collections::{BTreeSet, HashMap};

use finance_storage::{NewTransaction, counterparty_account, insert_transaction};
use finance_types::bank::{
    AccountBalance, BANKS, BankRun, BankState, BankSyncReport, BankSyncResult, SyncStatus,
};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};
use rusqlite::{Connection as Sql, OptionalExtension};
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use connection::{Connection, Identity};
pub use entry::Entry;
pub use import::{Imported, OwnAccount, import, own_accounts};

use crate::service::Service;
use crate::writes::{Writer, utc_stamp};

/// Days of overlap each sync downloads again (the importer's 30-day range).
const OVERLAP_DAYS: i64 = 30;
/// The importer gave up after this many pages of one account.
const MAX_PAGES: usize = 50;

/// Why a request to Enable Banking failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// HTTP 429 or the bank's own rate limit.
    RateLimited,
    /// Any other failure: network, non-200 answer, unreadable body.
    Unavailable,
}

impl From<ApiError> for SyncStatus {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::RateLimited => Self::RateLimited,
            ApiError::Unavailable => Self::UpstreamUnavailable,
        }
    }
}

/// An authenticated `GET` against `https://api.enablebanking.com/<path>`.
pub trait Api {
    fn get(&mut self, path: &str) -> Result<Value, ApiError>;
    /// An authenticated `POST` of a JSON body.
    fn post(&mut self, path: &str, body: &Value) -> Result<Value, ApiError>;
    /// What the bank answered to the last failed request, for the owner
    /// (request kind, HTTP status, error code and message).
    fn last_failure(&self) -> Option<String> {
        None
    }
}

pub(crate) fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Upper case without spaces, as the worker compared IBANs.
pub fn normalize_iban(value: &str) -> String {
    value.replace(' ', "").to_uppercase()
}

/// PHP `rawurlencode`.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The days one sync covers (both inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: Date,
    pub to: Date,
}

/// The last 30 days up to `today` (UTC), never before the cut-over day.
/// `None` on the cut-over day itself: there is no completed day yet.
pub fn window(today: Date, not_before: Option<Date>) -> Result<Option<Window>, SyncStatus> {
    let mut from = today
        .checked_sub(OVERLAP_DAYS.days())
        .map_err(|_| SyncStatus::InvalidConfiguration)?;
    if let Some(cutoff) = not_before {
        if cutoff > today {
            return Err(SyncStatus::InvalidConfiguration);
        }
        from = from.max(cutoff);
    }
    Ok((from != today).then_some(Window { from, to: today }))
}

/// The bank account uids the consent's sessions grant, and when the
/// earliest of them expires.
fn granted(
    api: &mut dyn Api,
    connection: &Connection,
) -> Result<(BTreeSet<String>, Option<String>), SyncStatus> {
    let mut sources = BTreeSet::new();
    let mut expires: Option<Timestamp> = None;
    for session in &connection.sessions {
        let data = api.get(&format!("sessions/{}", encode(session)))?;
        if data.get("status").and_then(Value::as_str) != Some("AUTHORIZED") {
            return Err(SyncStatus::ReauthorizationRequired);
        }
        if let Some(until) = data
            .get("access")
            .and_then(|a| a.get("valid_until"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Timestamp>().ok())
        {
            expires = Some(expires.map_or(until, |e| e.min(until)));
        }
        for account in data
            .get("accounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let uid = account
                .as_str()
                .or_else(|| account.get("uid").and_then(Value::as_str))
                .unwrap_or("");
            sources.insert(uid.to_owned());
        }
    }
    Ok((sources, expires.map(utc_stamp)))
}

fn details_iban(api: &mut dyn Api, uid: &str) -> Result<String, SyncStatus> {
    let details = api.get(&format!("accounts/{}/details", encode(uid)))?;
    Ok(normalize_iban(
        details
            .get("account_id")
            .and_then(|a| a.get("iban"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    ))
}

/// The consent still holds and every bank account still fills the same
/// ledger account: the worker's `bankImportPreflight`, or with `full` its
/// `bankPreflight` (always reads details and pages through the window), used
/// before a renewed consent is stored.
pub fn preflight(
    api: &mut dyn Api,
    connection: &Connection,
    own: &HashMap<i64, OwnAccount>,
    full: Option<Window>,
) -> Result<Option<String>, SyncStatus> {
    let (sources, expires) = granted(api, connection)?;
    for (uid, &target) in &connection.accounts {
        if !sources.contains(uid) {
            return Err(SyncStatus::MappingMismatch);
        }
        let ledger = own
            .get(&target)
            .filter(|a| a.kind == "asset" && a.active)
            .ok_or(SyncStatus::MappingMismatch)?;
        if full.is_none() && connection.hash_bound(uid, target, &ledger.iban) {
            continue;
        }
        let iban = details_iban(api, uid)?;
        let bound = iban.is_empty()
            && (connection.hash_bound(uid, target, &ledger.iban)
                || connection.no_iban_bound(uid, target, &ledger.iban));
        if !bound && (iban.is_empty() || iban != ledger.iban) {
            return Err(SyncStatus::MappingMismatch);
        }
        if let Some(window) = full {
            download(api, uid, window)?;
        }
    }
    Ok(expires)
}

/// Every entry of `uid` in `window`, page by page.
pub fn download(api: &mut dyn Api, uid: &str, window: Window) -> Result<Vec<Entry>, SyncStatus> {
    let base = format!(
        "accounts/{}/transactions?date_from={}&date_to={}",
        encode(uid),
        window.from,
        window.to
    );
    let mut entries = Vec::new();
    let mut key = String::new();
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let path = if key.is_empty() {
            base.clone()
        } else {
            format!("{base}&continuation_key={}", encode(&key))
        };
        let page = api.get(&path)?;
        entries.extend(Entry::page(&page).ok_or(SyncStatus::UpstreamUnavailable)?);
        key = page
            .get("continuation_key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if key.is_empty() {
            return Ok(entries);
        }
        if !seen.insert(key.clone()) {
            return Err(SyncStatus::ProviderPaginationError);
        }
    }
    Err(SyncStatus::ProviderPaginationError)
}

/// When the scheduler may sync a bank again: a day after success, six hours
/// after a rate limit, an hour after anything else, and the next UTC day
/// while waiting for a completed day (`bank_control.py` `status`).
pub fn next_eligible(run: &BankRun) -> Option<Timestamp> {
    let last: Timestamp = run.last_attempt.as_deref()?.parse().ok()?;
    match run.status {
        Some(SyncStatus::WaitingForCompletedDay) => last
            .to_zoned(TimeZone::UTC)
            .date()
            .tomorrow()
            .ok()?
            .to_zoned(TimeZone::UTC)
            .ok()
            .map(|z| z.timestamp()),
        Some(SyncStatus::Success) => last.checked_add(SignedDuration::from_hours(24)).ok(),
        Some(SyncStatus::RateLimited) => last.checked_add(SignedDuration::from_hours(6)).ok(),
        _ => last.checked_add(SignedDuration::from_hours(1)).ok(),
    }
}

/// `bank_imports.py` `is_due`.
pub fn is_due(run: &BankRun, now: Timestamp) -> bool {
    next_eligible(run).is_none_or(|at| now >= at)
}

/// A rate limit that still holds manual syncs back.
pub fn retry_after(run: &BankRun, now: Timestamp) -> Option<Timestamp> {
    (run.status == Some(SyncStatus::RateLimited))
        .then(|| next_eligible(run))
        .flatten()
        .filter(|at| *at > now)
}

/// The run after an attempt at `now` that ended in `status`.
pub fn record(
    previous: &BankRun,
    status: SyncStatus,
    imported: Option<Imported>,
    now: Timestamp,
) -> BankRun {
    let stamp = utc_stamp(now);
    let success = status == SyncStatus::Success;
    BankRun {
        status: Some(status),
        last_attempt: Some(stamp.clone()),
        last_success: if success {
            Some(stamp)
        } else {
            previous.last_success.clone()
        },
        created: if success {
            imported.map(|i| i.created)
        } else {
            previous.created
        },
        consent_valid_until: previous.consent_valid_until.clone(),
        checked: if success {
            imported.map(|i| i.checked)
        } else {
            previous.checked
        },
        detail: None,
        balances: previous.balances.clone(),
    }
}

fn has_runs_table(conn: &Sql) -> crate::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'bank_sync_runs'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The stored run of every bank that has one.
pub fn runs(conn: &Sql) -> crate::Result<HashMap<String, BankRun>> {
    if !has_runs_table(conn)? {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare("SELECT bank, payload_json FROM bank_sync_runs")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut runs = HashMap::new();
    for row in rows {
        let (bank, json) = row?;
        if let Ok(run) = serde_json::from_str(&json) {
            runs.insert(bank, run);
        }
    }
    Ok(runs)
}

pub fn save_run(w: &Writer<'_>, bank: &str, run: &BankRun) -> crate::Result<()> {
    w.put_json(
        "INSERT OR REPLACE INTO bank_sync_runs (bank, payload_json) VALUES (?2, ?1)",
        run,
        &[&bank],
    )
}

/// Update the data-refresh health the dashboard reads (Python's
/// `firefly_sync_health`, written by its sync loop): every bank sync is an
/// attempt; the data is fresh only while no connected bank's last sync
/// failed. Banks without consent or key do not count.
pub fn save_health(w: &Writer<'_>) -> crate::Result<()> {
    let exists =
        w.tx.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'firefly_sync_health'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(());
    }
    let failed = runs(&w.tx)?.values().any(|r| {
        r.status.is_some_and(|s| {
            !s.is_ok()
                && !matches!(
                    s,
                    SyncStatus::MissingConfig | SyncStatus::MissingCredentials
                )
        })
    });
    let stamp = w.stamp();
    w.tx.execute(
        "INSERT INTO firefly_sync_health (singleton, last_success, last_attempt, failed) \
         VALUES (1, CASE WHEN ?2 THEN NULL ELSE ?1 END, ?1, ?2) \
         ON CONFLICT(singleton) DO UPDATE SET last_attempt = ?1, failed = ?2, \
         last_success = CASE WHEN ?2 THEN last_success ELSE ?1 END",
        rusqlite::params![stamp, failed],
    )?;
    Ok(())
}

/// The old importer's `status.json`, as runs. Statuses this app does not
/// know (Firefly-only ones) read as a failed attempt.
pub fn legacy_runs(raw: &str) -> HashMap<String, BankRun> {
    let Ok(Value::Object(banks)) = serde_json::from_str::<Value>(raw) else {
        return HashMap::new();
    };
    let stamp = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Timestamp>().ok())
            .map(utc_stamp)
    };
    BANKS
        .iter()
        .filter_map(|(id, _)| {
            let item = banks.get(*id)?;
            let status = item.get("status").map(|s| {
                serde_json::from_value::<SyncStatus>(s.clone()).unwrap_or(SyncStatus::Failed)
            });
            Some((
                (*id).to_owned(),
                BankRun {
                    status,
                    last_attempt: stamp(item, "last_attempt"),
                    last_success: stamp(item, "last_success"),
                    created: None,
                    checked: item
                        .get("converted")
                        .and_then(Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok()),
                    consent_valid_until: None,
                    detail: None,
                    balances: Vec::new(),
                },
            ))
        })
        .collect()
}

/// What the app layer knows that the database does not.
#[derive(Debug, Default)]
pub struct Context {
    /// Bank id → the ledger accounts its stored consent fills.
    pub connected: HashMap<String, Vec<i64>>,
    pub credentials: bool,
    pub running: Option<String>,
    pub legacy_consents: bool,
    pub last_check: Option<Timestamp>,
}

impl Service {
    /// The Banks screen.
    pub fn bank_sync_report(&self, context: Context) -> Result<BankSyncReport, String> {
        let now = Timestamp::now();
        let (runs, own) = self.read_store(|conn| Ok((runs(conn)?, own_accounts(conn)?)))?;
        let banks = BANKS
            .iter()
            .map(|(id, name)| {
                let run = runs.get(*id).cloned().unwrap_or_default();
                let targets = context.connected.get(*id);
                BankState {
                    id: (*id).to_owned(),
                    name: (*name).to_owned(),
                    next_eligible: targets.and(next_eligible(&run)).map(utc_stamp),
                    retry_after: retry_after(&run, now).map(utc_stamp),
                    connected: targets.is_some(),
                    accounts: targets
                        .into_iter()
                        .flatten()
                        .map(|t| {
                            own.get(t)
                                .map_or_else(|| format!("Account {t}"), |a| a.name.clone())
                        })
                        .collect(),
                    run,
                }
            })
            .collect();
        Ok(BankSyncReport {
            banks,
            credentials: context.credentials,
            running: context.running,
            legacy_consents: context.legacy_consents,
            last_check: context.last_check.map(utc_stamp),
        })
    }

    /// The ledger's own accounts, for mapping bank accounts onto them.
    pub fn bank_own_accounts(&self) -> Result<HashMap<i64, OwnAccount>, String> {
        self.read_store(own_accounts)
    }

    /// The stored run of every bank.
    pub fn bank_runs(&self) -> Result<HashMap<String, BankRun>, String> {
        self.read_store(runs)
    }

    /// Store a run as is (taking over the old importer's history).
    pub fn save_bank_run(&self, bank: &str, run: &BankRun) -> Result<(), String> {
        self.write(|w, _| save_run(w, bank, run))
    }

    /// Check a consent against the bank before it is stored.
    pub fn verify_bank(
        &self,
        connection: &Connection,
        api: &mut dyn Api,
        now: Timestamp,
    ) -> Result<(), SyncStatus> {
        let today = now.to_zoned(TimeZone::UTC).date();
        let window = window(today, connection.import_not_before)?.unwrap_or(Window {
            from: today,
            to: today,
        });
        let own = self
            .read_store(own_accounts)
            .map_err(|_| SyncStatus::Failed)?;
        preflight(api, connection, &own, Some(window)).map(|_| ())
    }

    /// Sync one bank: check the consent, download every account's window,
    /// write what is new, run the closed-month checks and record the run.
    pub fn sync_bank(
        &self,
        bank: &str,
        connection: &Connection,
        api: &mut dyn Api,
        now: Timestamp,
    ) -> Result<BankSyncResult, String> {
        let previous = self.bank_runs()?.remove(bank).unwrap_or_default();
        let Attempt {
            status,
            imported,
            expires,
            balances,
        } = self.attempt(bank, connection, api, now);
        let mut run = record(&previous, status, imported, now);
        if expires.is_some() {
            run.consent_valid_until = expires;
        }
        if !balances.is_empty() {
            run.balances = balances;
        }
        if !status.is_ok() {
            run.detail = api.last_failure();
        }
        self.save_bank_run(bank, &run)?;
        self.write(|w, _| save_health(w))?;
        Ok(BankSyncResult {
            bank: bank.to_owned(),
            status,
            created: imported.map(|i| i.created),
            checked: imported.map(|i| i.checked),
        })
    }

    fn attempt(
        &self,
        bank: &str,
        connection: &Connection,
        api: &mut dyn Api,
        now: Timestamp,
    ) -> Attempt {
        let failed = |status| Attempt {
            status,
            ..Attempt::default()
        };
        let today = now.to_zoned(TimeZone::UTC).date();
        let window = match window(today, connection.import_not_before) {
            Ok(Some(window)) => window,
            Ok(None) => return failed(SyncStatus::WaitingForCompletedDay),
            Err(status) => return failed(status),
        };
        let Ok(own) = self.read_store(own_accounts) else {
            return failed(SyncStatus::Failed);
        };
        let expires = match preflight(api, connection, &own, None) {
            Ok(expires) => expires,
            Err(status) => return failed(status),
        };
        let mut downloads = Vec::new();
        let mut failure = None;
        for uid in connection.accounts.keys() {
            match download(api, uid, window) {
                Ok(entries) => downloads.push((uid.clone(), entries)),
                Err(status) => {
                    failure.get_or_insert(status);
                }
            }
        }
        let written = self.write(|w, _| {
            let own = own_accounts(&w.tx)?;
            let imported = import(w, bank, connection, &downloads, window, &own)?;
            w.record(
                &format!("enable_banking:{bank}"),
                &serde_json::json!({
                    "bank": bank,
                    "created": imported.created,
                    "checked": imported.checked,
                }),
            )?;
            Ok(imported)
        });
        let Ok(imported) = written else {
            return Attempt {
                status: SyncStatus::ImportFailed,
                expires,
                ..Attempt::default()
            };
        };
        // Late transactions may reopen closed months (Python ran this after
        // every bank sync); it needs a snapshot with the new rows.
        if imported.created > 0
            && self
                .write(crate::writes::closure::refresh_after_sync)
                .is_err()
        {
            return Attempt {
                status: SyncStatus::ImportFailed,
                imported: Some(imported),
                expires,
                balances: Vec::new(),
            };
        }
        // New transactions may change the alerts (Python evaluated them after
        // every sync). A failure here never fails the sync.
        if imported.created > 0 {
            let _ = self.evaluate_alerts_now();
        }
        // The bank's own balances, to show where the ledger drifted. A
        // failure here never fails the sync.
        let balances = self
            .read_store(|conn| Ok(balances(api, connection, conn)))
            .unwrap_or_default();
        let status = match failure {
            None => SyncStatus::Success,
            Some(SyncStatus::RateLimited) => SyncStatus::RateLimited,
            Some(_) => SyncStatus::ImportFailed,
        };
        Attempt {
            status,
            imported: Some(imported),
            expires,
            balances,
        }
    }

    /// Bring `account_id`'s ledger balance to what `bank` last reported for
    /// it, with a reconciliation entry (neither spending nor income), as the
    /// owner's earlier manual reconciliations did.
    pub fn reconcile_bank_account(&self, bank: &str, account_id: i64) -> Result<(), String> {
        let mut run = self.bank_runs()?.remove(bank).unwrap_or_default();
        let entry = run
            .balances
            .iter_mut()
            .find(|b| b.account_id == account_id)
            .ok_or("Sync the bank first: there is no bank balance for this account.")?;
        let bank_amount: Decimal = entry.bank.parse().map_err(|_| "Invalid bank balance")?;
        let reported = entry.reference_date.clone();
        let currency = entry.currency.clone();
        let account = entry.account.clone();
        self.write(|w, _| {
            let ledger = ledger_balance(&w.tx, account_id, &currency)?;
            let difference = bank_amount - ledger;
            if difference.is_zero() {
                return Err(crate::writes::invalid("The ledger already matches the bank."));
            }
            let other = counterparty_account(
                &w.tx,
                &format!("{account} reconciliation ({currency})"),
                "reconciliation",
            )?;
            let (source, destination) = if difference.is_sign_positive() {
                (other, account_id)
            } else {
                (account_id, other)
            };
            let day = w.now.date().to_string();
            let description = format!(
                "Balance reconciliation — bank reported {bank_amount} {currency}{}",
                reported.map(|d| format!(" on {d}")).unwrap_or_default()
            );
            insert_transaction(
                &w.tx,
                &NewTransaction {
                    kind: "reconciliation",
                    date: &day,
                    description: &description,
                    source_account_id: source,
                    destination_account_id: destination,
                    amount: difference.abs(),
                    currency: &currency,
                    external_id: None,
                    internal_reference: None,
                    notes: None,
                    tags: &[],
                },
            )?;
            w.record(
                "reconciliation",
                &serde_json::json!({"account_id": account_id, "difference": difference.to_string()}),
            )?;
            Ok(())
        })?;
        entry.ledger = entry.bank.clone();
        self.save_bank_run(bank, &run)
    }
}

/// What one sync attempt produced.
#[derive(Debug)]
struct Attempt {
    status: SyncStatus,
    imported: Option<Imported>,
    expires: Option<String>,
    balances: Vec<AccountBalance>,
}

impl Default for Attempt {
    fn default() -> Self {
        Self {
            status: SyncStatus::Failed,
            imported: None,
            expires: None,
            balances: Vec::new(),
        }
    }
}

/// The ledger balance of `account` in `currency` (sum of its postings).
fn ledger_balance(conn: &Sql, account: i64, currency: &str) -> crate::Result<Decimal> {
    let mut stmt =
        conn.prepare_cached("SELECT amount FROM postings WHERE account_id = ?1 AND currency = ?2")?;
    let rows = stmt.query_map(rusqlite::params![account, currency], |r| {
        r.get::<_, String>(0)
    })?;
    let mut total = Decimal::ZERO;
    for row in rows {
        total += row?.parse::<Decimal>().unwrap_or_default();
    }
    Ok(total.normalize())
}

/// The balance type that best matches a ledger of booked transactions.
const BALANCE_TYPES: [&str; 6] = ["CLBD", "ITBD", "XPCD", "OPBD", "ITAV", "CLAV"];

/// Each synced account's bank balance next to its ledger balance
/// (`GET /accounts/{uid}/balances`). Accounts the bank gives no balance for
/// are left out.
pub fn balances(api: &mut dyn Api, connection: &Connection, conn: &Sql) -> Vec<AccountBalance> {
    let names: HashMap<i64, String> = own_accounts(conn)
        .map(|own| own.into_iter().map(|(id, a)| (id, a.name)).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (uid, &target) in &connection.accounts {
        let Ok(answer) = api.get(&format!("accounts/{}/balances", encode(uid))) else {
            continue;
        };
        let list = answer
            .get("balances")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let rank = |b: &Value| {
            let kind = b.get("balance_type").and_then(Value::as_str).unwrap_or("");
            BALANCE_TYPES
                .iter()
                .position(|t| *t == kind)
                .unwrap_or(BALANCE_TYPES.len())
        };
        let Some(best) = list.iter().min_by_key(|b| rank(b)) else {
            continue;
        };
        let amount = best
            .get("balance_amount")
            .and_then(|a| a.get("amount"))
            .and_then(|a| match a {
                Value::String(s) => s.parse::<Decimal>().ok(),
                Value::Number(n) => n.to_string().parse::<Decimal>().ok(),
                _ => None,
            });
        let currency = best
            .get("balance_amount")
            .and_then(|a| a.get("currency"))
            .and_then(Value::as_str);
        let (Some(amount), Some(currency)) = (amount, currency) else {
            continue;
        };
        let Ok(ledger) = ledger_balance(conn, target, currency) else {
            continue;
        };
        out.push(AccountBalance {
            account_id: target,
            account: names
                .get(&target)
                .cloned()
                .unwrap_or_else(|| format!("Account {target}")),
            currency: currency.to_owned(),
            bank: amount.normalize().to_string(),
            ledger: ledger.to_string(),
            balance_type: best
                .get("balance_type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            reference_date: best
                .get("reference_date")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};

    use serde_json::json;

    use super::*;

    /// Answers requests from a script and records the paths asked for.
    struct Script {
        answers: HashMap<String, VecDeque<Result<Value, ApiError>>>,
        asked: Vec<String>,
    }

    impl Script {
        fn new(answers: Vec<(&str, Result<Value, ApiError>)>) -> Self {
            let mut map: HashMap<String, VecDeque<_>> = HashMap::new();
            for (path, answer) in answers {
                map.entry(path.to_owned()).or_default().push_back(answer);
            }
            Self {
                answers: map,
                asked: Vec::new(),
            }
        }
    }

    impl Api for Script {
        fn get(&mut self, path: &str) -> Result<Value, ApiError> {
            self.asked.push(path.to_owned());
            self.answers
                .get_mut(path)
                .and_then(VecDeque::pop_front)
                .unwrap_or(Err(ApiError::Unavailable))
        }

        fn post(&mut self, path: &str, _body: &Value) -> Result<Value, ApiError> {
            self.get(&format!("POST {path}"))
        }
    }

    fn own() -> HashMap<i64, OwnAccount> {
        HashMap::from([(
            5,
            OwnAccount {
                id: 5,
                name: "ABN AMRO".into(),
                kind: "asset".into(),
                active: true,
                iban: "NL01BANK0000000001".into(),
            },
        )])
    }

    fn connection() -> Connection {
        Connection {
            sessions: vec!["s1".into()],
            accounts: BTreeMap::from([("uid-1".into(), 5)]),
            verified_identity: BTreeMap::new(),
            import_not_before: None,
            names: BTreeMap::new(),
        }
    }

    #[test]
    fn window_is_thirty_days_after_the_cutover() {
        let today = jiff::civil::date(2026, 10, 27);
        assert_eq!(
            window(today, None).unwrap(),
            Some(Window {
                from: jiff::civil::date(2026, 9, 27),
                to: today
            })
        );
        assert_eq!(
            window(today, Some(jiff::civil::date(2026, 10, 20)))
                .unwrap()
                .unwrap()
                .from,
            jiff::civil::date(2026, 10, 20)
        );
        assert_eq!(window(today, Some(today)).unwrap(), None);
        assert_eq!(
            window(today, Some(jiff::civil::date(2026, 11, 1))).unwrap_err(),
            SyncStatus::InvalidConfiguration
        );
    }

    #[test]
    fn preflight_checks_session_and_iban() {
        let session = json!({"status": "AUTHORIZED", "accounts": [{"uid": "uid-1"}]});
        let details = json!({"account_id": {"iban": "nl01 bank 0000 0000 01"}});
        let mut ok = Script::new(vec![
            ("sessions/s1", Ok(session.clone())),
            ("accounts/uid-1/details", Ok(details)),
        ]);
        assert_eq!(preflight(&mut ok, &connection(), &own(), None), Ok(None));

        let mut expired = Script::new(vec![("sessions/s1", Ok(json!({"status": "EXPIRED"})))]);
        assert_eq!(
            preflight(&mut expired, &connection(), &own(), None),
            Err(SyncStatus::ReauthorizationRequired)
        );

        let mut other = Script::new(vec![
            ("sessions/s1", Ok(session)),
            (
                "accounts/uid-1/details",
                Ok(json!({"account_id": {"iban": "NL99"}})),
            ),
        ]);
        assert_eq!(
            preflight(&mut other, &connection(), &own(), None),
            Err(SyncStatus::MappingMismatch)
        );

        let mut limited = Script::new(vec![("sessions/s1", Err(ApiError::RateLimited))]);
        assert_eq!(
            preflight(&mut limited, &connection(), &own(), None),
            Err(SyncStatus::RateLimited)
        );
    }

    #[test]
    fn verified_iban_hash_skips_the_details_request() {
        let mut c = connection();
        c.verified_identity.insert(
            "uid-1".into(),
            Identity {
                target: 5,
                sessions: vec!["s1".into()],
                iban_sha256: Some(sha256_hex("NL01BANK0000000001")),
                no_iban: false,
            },
        );
        let mut api = Script::new(vec![(
            "sessions/s1",
            Ok(json!({"status": "AUTHORIZED", "accounts": ["uid-1"]})),
        )]);
        assert_eq!(preflight(&mut api, &c, &own(), None), Ok(None));
        assert_eq!(api.asked, ["sessions/s1"]);
    }

    #[test]
    fn download_follows_pages_and_stops_on_loops() {
        let window = Window {
            from: jiff::civil::date(2026, 10, 1),
            to: jiff::civil::date(2026, 10, 27),
        };
        let base = "accounts/uid-1/transactions?date_from=2026-10-01&date_to=2026-10-27";
        let mut api = Script::new(vec![
            (
                base,
                Ok(json!({"transactions": [{"entry_reference": "a"}], "continuation_key": "k 1"})),
            ),
            (
                &format!("{base}&continuation_key=k%201"),
                Ok(json!({"transactions": [{"entry_reference": "b"}], "continuation_key": null})),
            ),
        ]);
        let ids: Vec<String> = download(&mut api, "uid-1", window)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);

        let mut looping = Script::new(vec![
            (
                base,
                Ok(json!({"transactions": [], "continuation_key": "k"})),
            ),
            (
                &format!("{base}&continuation_key=k"),
                Ok(json!({"transactions": [], "continuation_key": "k"})),
            ),
        ]);
        assert_eq!(
            download(&mut looping, "uid-1", window).unwrap_err(),
            SyncStatus::ProviderPaginationError
        );
    }

    #[test]
    fn cadence_follows_the_last_outcome() {
        let at = |s: &str| s.parse::<Timestamp>().unwrap();
        let run = |status| BankRun {
            status: Some(status),
            last_attempt: Some("2026-10-26T20:15:00Z".into()),
            ..BankRun::default()
        };
        assert_eq!(
            next_eligible(&run(SyncStatus::Success)),
            Some(at("2026-10-27T20:15:00Z"))
        );
        assert_eq!(
            next_eligible(&run(SyncStatus::RateLimited)),
            Some(at("2026-10-27T02:15:00Z"))
        );
        assert_eq!(
            next_eligible(&run(SyncStatus::Failed)),
            Some(at("2026-10-26T21:15:00Z"))
        );
        assert_eq!(
            next_eligible(&run(SyncStatus::WaitingForCompletedDay)),
            Some(at("2026-10-27T00:00:00Z"))
        );
        assert!(is_due(&BankRun::default(), at("2026-10-26T00:00:00Z")));
        assert!(!is_due(
            &run(SyncStatus::Success),
            at("2026-10-27T10:00:00Z")
        ));
        assert_eq!(
            retry_after(&run(SyncStatus::RateLimited), at("2026-10-26T21:00:00Z")),
            Some(at("2026-10-27T02:15:00Z"))
        );
    }

    #[test]
    fn records_keep_the_last_success() {
        let now = "2026-10-27T08:00:00Z".parse::<Timestamp>().unwrap();
        let ok = record(
            &BankRun::default(),
            SyncStatus::Success,
            Some(Imported {
                created: 3,
                checked: 40,
            }),
            now,
        );
        assert_eq!(ok.created, Some(3));
        let failed = record(&ok, SyncStatus::RateLimited, None, now);
        assert_eq!(failed.last_success, ok.last_success);
        assert_eq!(failed.created, Some(3));
        assert_eq!(failed.status, Some(SyncStatus::RateLimited));
    }

    #[test]
    fn health_follows_the_connected_banks() {
        let mut db = crate::writes::fixture::db();
        db.store
            .connection()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS firefly_sync_health (singleton INTEGER PRIMARY KEY,
                 last_success TEXT, last_attempt TEXT NOT NULL, failed INTEGER NOT NULL,
                 last_full_success TEXT);
                 INSERT OR REPLACE INTO firefly_sync_health VALUES (1, 'old', 'old', 0, 'full');",
            )
            .unwrap();
        let run = |status| BankRun {
            status: Some(status),
            ..BankRun::default()
        };
        db.write(|w, _| {
            save_run(w, "abnamro", &run(SyncStatus::Success))?;
            save_run(w, "swedbank", &run(SyncStatus::MissingConfig))?;
            save_health(w)
        })
        .unwrap();
        let fresh: String = db.one("SELECT last_success || '|' || failed || '|' || last_full_success FROM firefly_sync_health");
        assert_eq!(fresh, "2026-10-26T20:00:00.000000Z|0|full");
        db.write(|w, _| {
            save_run(w, "revolut", &run(SyncStatus::ReauthorizationRequired))?;
            save_health(w)
        })
        .unwrap();
        let failed: String =
            db.one("SELECT last_success || '|' || failed FROM firefly_sync_health");
        assert_eq!(failed, "2026-10-26T20:00:00.000000Z|1");
    }

    #[test]
    fn reconcile_books_the_difference_to_the_bank() {
        let dir = std::env::temp_dir().join(format!("finance-reconcile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("finance.sqlite3");
        let _ = std::fs::remove_file(&path);
        {
            let store = finance_storage::Store::open(&path).unwrap();
            store
                .connection()
                .execute_batch(include_str!("../writes/test_fixture.sql"))
                .unwrap();
        }
        let service = Service::new(path.clone());
        let ledger = |service: &Service| {
            service
                .read_store(|conn| ledger_balance(conn, 1, "EUR"))
                .unwrap()
        };
        let before = ledger(&service);
        let bank = before + Decimal::new(1250, 2);
        let run = BankRun {
            balances: vec![AccountBalance {
                account_id: 1,
                account: "Checking".into(),
                currency: "EUR".into(),
                bank: bank.to_string(),
                ledger: before.to_string(),
                balance_type: "CLBD".into(),
                reference_date: Some("2026-10-27".into()),
            }],
            ..BankRun::default()
        };
        service.save_bank_run("abnamro", &run).unwrap();
        service.reconcile_bank_account("abnamro", 1).unwrap();
        assert_eq!(ledger(&service), bank);
        // The entry is a reconciliation, not spending or income.
        let kind: String = service
            .read_store(|conn| {
                Ok(conn.query_row(
                    "SELECT kind FROM transactions ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(kind, "reconciliation");
        assert!(service.reconcile_bank_account("abnamro", 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_old_status_file() {
        let runs = legacy_runs(
            r#"{"abnamro":{"status":"success","last_attempt":"2026-10-26T19:15:00.123+00:00",
                 "last_success":"2026-10-26T19:15:01+00:00","converted":12},
                "revolut":{"status":"runtime_unavailable","last_attempt":"2026-10-26T20:00:00+00:00"},
                "swedbank":{"status":"event_review_required"}}"#,
        );
        assert_eq!(runs["abnamro"].status, Some(SyncStatus::Success));
        assert_eq!(runs["abnamro"].checked, Some(12));
        assert_eq!(
            runs["abnamro"].last_attempt.as_deref(),
            Some("2026-10-26T19:15:00.123000Z")
        );
        assert_eq!(runs["revolut"].status, Some(SyncStatus::RuntimeUnavailable));
        assert_eq!(runs["swedbank"].status, Some(SyncStatus::Failed));
    }
}
