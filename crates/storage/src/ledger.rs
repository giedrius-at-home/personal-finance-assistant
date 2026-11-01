//! Ledger import: replaces all ledger rows in one transaction.

use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{Error, Result, Store};

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerAccount {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub iban: Option<String>,
    pub active: bool,
    pub role: Option<String>,
    pub include_net_worth: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerCategory {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerBill {
    pub id: i64,
    pub name: String,
    pub currency: String,
    pub amount_min: Decimal,
    pub amount_max: Decimal,
    pub start_date: String,
    pub end_date: Option<String>,
    pub repeat_freq: String,
    pub skip: i64,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerTransaction {
    pub id: i64,
    pub group_id: Option<i64>,
    pub kind: String,
    pub booked_at: String,
    pub description: String,
    pub category_id: Option<i64>,
    pub bill_id: Option<i64>,
    pub external_id: Option<String>,
    pub internal_reference: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerPosting {
    pub id: i64,
    pub transaction_id: i64,
    pub account_id: i64,
    pub amount: Decimal,
    pub currency: String,
    pub foreign_amount: Option<Decimal>,
    pub foreign_currency: Option<String>,
    pub reconciled: bool,
}

/// A complete ledger, as exported from the source system.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    pub accounts: Vec<LedgerAccount>,
    pub categories: Vec<LedgerCategory>,
    pub bills: Vec<LedgerBill>,
    pub transactions: Vec<LedgerTransaction>,
    pub postings: Vec<LedgerPosting>,
}

impl Store {
    /// Replace the whole ledger with `ledger`. Either everything is written or
    /// nothing is.
    pub fn replace_ledger(&mut self, ledger: &Ledger, source: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM postings; DELETE FROM transactions; DELETE FROM bills;
             DELETE FROM categories; DELETE FROM accounts;
             INSERT INTO transactions_search(transactions_search) VALUES ('delete-all');",
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO accounts (id, legacy_firefly_id, name, kind, iban, active, role, include_net_worth)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for a in &ledger.accounts {
                insert.execute(params![
                    a.id,
                    a.name,
                    a.kind,
                    a.iban,
                    a.active,
                    a.role,
                    a.include_net_worth
                ])?;
            }
            let mut insert = tx.prepare(
                "INSERT INTO categories (id, legacy_firefly_id, name) VALUES (?1, ?1, ?2)",
            )?;
            for c in &ledger.categories {
                insert.execute(params![c.id, c.name])?;
            }
            let mut insert = tx.prepare(
                "INSERT INTO bills (id, legacy_firefly_id, name, currency, amount_min, amount_max,
                                    start_date, end_date, repeat_freq, skip, active)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for b in &ledger.bills {
                insert.execute(params![
                    b.id,
                    b.name,
                    b.currency,
                    b.amount_min.to_string(),
                    b.amount_max.to_string(),
                    b.start_date,
                    b.end_date,
                    b.repeat_freq,
                    b.skip,
                    b.active
                ])?;
            }
        }

        let account_names: std::collections::HashMap<i64, &str> = ledger
            .accounts
            .iter()
            .map(|a| (a.id, a.name.as_str()))
            .collect();
        let category_names: std::collections::HashMap<i64, &str> = ledger
            .categories
            .iter()
            .map(|c| (c.id, c.name.as_str()))
            .collect();
        let mut postings_by_tx: std::collections::HashMap<i64, Vec<&LedgerPosting>> =
            std::collections::HashMap::new();
        for p in &ledger.postings {
            postings_by_tx.entry(p.transaction_id).or_default().push(p);
        }

        {
            let mut insert_tx = tx.prepare(
                "INSERT INTO transactions (id, legacy_firefly_id, group_id, kind, date, booked_at, description,
                    source_account_id, destination_account_id, category_id, bill_id, external_id,
                    internal_reference, notes, tags, reconciled)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            let mut insert_posting = tx.prepare(
                "INSERT INTO postings (id, legacy_firefly_id, transaction_id, account_id, amount, currency,
                                       foreign_amount, foreign_currency)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut insert_search =
                tx.prepare("INSERT INTO transactions_search (rowid, text) VALUES (?1, ?2)")?;

            for t in &ledger.transactions {
                let postings = postings_by_tx
                    .get(&t.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let [first, second] = postings else {
                    return Err(Error::Corrupt(format!(
                        "transaction {} has {} postings",
                        t.id,
                        postings.len()
                    )));
                };
                // The source side is the negative posting; for zero amounts the
                // source is written first (lower id).
                let (source, destination) = if second.amount < first.amount
                    || (second.amount == first.amount && second.id < first.id)
                {
                    (second, first)
                } else {
                    (first, second)
                };
                let date = t
                    .booked_at
                    .get(..10)
                    .ok_or_else(|| Error::Corrupt(format!("date of {}", t.id)))?;
                let tags =
                    serde_json::to_string(&t.tags).map_err(|e| Error::Corrupt(e.to_string()))?;
                insert_tx.execute(params![
                    t.id,
                    t.group_id,
                    t.kind,
                    date,
                    t.booked_at,
                    t.description,
                    source.account_id,
                    destination.account_id,
                    t.category_id,
                    t.bill_id,
                    t.external_id,
                    t.internal_reference,
                    t.notes,
                    tags,
                    postings.iter().all(|p| p.reconciled),
                ])?;
                for p in postings {
                    insert_posting.execute(params![
                        p.id,
                        t.id,
                        p.account_id,
                        p.amount.to_string(),
                        p.currency,
                        p.foreign_amount.map(|a| a.to_string()),
                        p.foreign_currency,
                    ])?;
                }
                let text = search_text(
                    &t.description,
                    account_names.get(&source.account_id).copied(),
                    account_names.get(&destination.account_id).copied(),
                    t.category_id.and_then(|c| category_names.get(&c)).copied(),
                    t.notes.as_deref(),
                    &t.tags,
                );
                insert_search.execute(params![t.id, text])?;
            }
        }
        let stats = serde_json::json!({
            "accounts": ledger.accounts.len(),
            "categories": ledger.categories.len(),
            "bills": ledger.bills.len(),
            "transactions": ledger.transactions.len(),
            "postings": ledger.postings.len(),
        });
        tx.execute(
            "INSERT INTO import_runs (source, stats) VALUES (?1, ?2)",
            params![source, stats.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// The text `transactions_search` indexes for one transaction: description,
/// both account names, category, notes and tags.
pub fn search_text(
    description: &str,
    source: Option<&str>,
    destination: Option<&str>,
    category: Option<&str>,
    notes: Option<&str>,
    tags: &[String],
) -> String {
    let mut text = vec![description];
    text.extend(source);
    text.extend(destination);
    text.extend(category);
    text.extend(notes);
    text.extend(tags.iter().map(String::as_str));
    text.join(" \u{2022} ")
}

/// A transaction to add to the ledger: two postings of `amount` (positive)
/// from `source_account_id` to `destination_account_id`.
#[derive(Clone, Debug)]
pub struct NewTransaction<'a> {
    /// `withdrawal`, `deposit` or `transfer`.
    pub kind: &'a str,
    /// `YYYY-MM-DD`.
    pub date: &'a str,
    pub description: &'a str,
    pub source_account_id: i64,
    pub destination_account_id: i64,
    pub amount: Decimal,
    pub currency: &'a str,
    pub external_id: Option<&'a str>,
    /// The bank's account id for synced rows (as the Firefly importer kept it).
    pub internal_reference: Option<&'a str>,
    pub notes: Option<&'a str>,
    pub tags: &'a [String],
}

/// Add `t` with its two postings and its search row; returns the new
/// transaction id. Runs on the caller's connection, so it joins the caller's
/// transaction.
pub fn insert_transaction(conn: &Connection, t: &NewTransaction<'_>) -> Result<i64> {
    if t.amount.is_sign_negative() {
        return Err(Error::Corrupt("negative transaction amount".into()));
    }
    let tags = serde_json::to_string(t.tags).map_err(|e| Error::Corrupt(e.to_string()))?;
    conn.execute(
        "INSERT INTO transactions (kind, date, booked_at, description, source_account_id,
            destination_account_id, external_id, internal_reference, notes, tags, reconciled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
        params![
            t.kind,
            t.date,
            format!("{}T00:00:00", t.date),
            t.description,
            t.source_account_id,
            t.destination_account_id,
            t.external_id,
            t.internal_reference,
            t.notes,
            tags,
        ],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute("UPDATE transactions SET group_id = ?1 WHERE id = ?1", [id])?;
    let mut posting = conn.prepare_cached(
        "INSERT INTO postings (transaction_id, account_id, amount, currency) VALUES (?1, ?2, ?3, ?4)",
    )?;
    posting.execute(params![
        id,
        t.source_account_id,
        (-t.amount).to_string(),
        t.currency
    ])?;
    posting.execute(params![
        id,
        t.destination_account_id,
        t.amount.to_string(),
        t.currency
    ])?;
    let name = |account: i64| -> Result<Option<String>> {
        Ok(conn
            .query_row("SELECT name FROM accounts WHERE id = ?1", [account], |r| {
                r.get(0)
            })
            .optional()?)
    };
    let text = search_text(
        t.description,
        name(t.source_account_id)?.as_deref(),
        name(t.destination_account_id)?.as_deref(),
        None,
        t.notes,
        t.tags,
    );
    conn.execute(
        "INSERT INTO transactions_search (rowid, text) VALUES (?1, ?2)",
        params![id, text],
    )?;
    Ok(id)
}

/// Remove one transaction with its postings and search row; Ok(false) when absent.
pub fn delete_transaction(conn: &Connection, id: i64) -> Result<bool> {
    conn.execute("DELETE FROM transactions_search WHERE rowid = ?1", [id])?;
    conn.execute("DELETE FROM postings WHERE transaction_id = ?1", [id])?;
    let gone = conn.execute("DELETE FROM transactions WHERE id = ?1", [id])?;
    Ok(gone > 0)
}

/// The id of the `kind` (`expense` or `revenue`) account called `name`,
/// created when there is none, as Firefly did for a counterparty name.
pub fn counterparty_account(conn: &Connection, name: &str, kind: &str) -> Result<i64> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM accounts WHERE kind = ?1 AND name = ?2 ORDER BY active DESC, id LIMIT 1",
            params![kind, name],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO accounts (name, kind, active, include_net_worth) VALUES (?1, ?2, 1, 1)",
        params![name, kind],
    )?;
    Ok(conn.last_insert_rowid())
}
