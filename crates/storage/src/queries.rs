//! Read queries behind the app's screens.
//!
//! Amounts are stored as exact decimal strings, so every sum happens in Rust
//! with [`Decimal`], never as SQLite floating point.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use finance_types::{AccountCheck, AccountKind, AccountSummary, DataStatus, Money};
use rusqlite::OptionalExtension;
use rust_decimal::Decimal;

use crate::{Error, Result, Store};

fn decimal(text: &str) -> Result<Decimal> {
    Decimal::from_str(text).map_err(|_| Error::Corrupt(format!("amount {text:?}")))
}

/// Per-currency running totals, kept in a stable order.
#[derive(Default)]
struct Sums(BTreeMap<String, Decimal>);

impl Sums {
    fn add(&mut self, currency: &str, amount: Decimal) {
        *self.0.entry(currency.to_owned()).or_default() += amount;
    }

    fn into_money(self) -> Vec<Money> {
        self.0
            .into_iter()
            .map(|(currency, amount)| Money {
                amount: amount.normalize(),
                currency,
            })
            .collect()
    }
}

impl Store {
    pub fn status(&self) -> Result<DataStatus> {
        let (transactions, first_date, last_date): (i64, Option<String>, Option<String>) =
            self.conn.query_row(
                "SELECT count(*), min(date), max(date) FROM transactions",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
        let transactions = transactions.unsigned_abs();
        let last_import: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT finished_at, source FROM import_runs ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (imported_at, source) = last_import.unzip();
        Ok(DataStatus {
            transactions,
            first_date,
            last_date,
            imported_at,
            source,
        })
    }

    /// Balances, posting counts and last activity for every own account.
    pub fn accounts(&self) -> Result<Vec<AccountSummary>> {
        let own = own_kinds_sql();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT a.id, a.name, a.kind, a.role, a.iban, a.active,
                    (SELECT max(t.date) FROM postings p JOIN transactions t ON t.id = p.transaction_id
                      WHERE p.account_id = a.id)
             FROM accounts a WHERE a.kind IN ({own})"
        ))?;
        let mut accounts: Vec<(AccountSummary, bool)> = stmt
            .query_map([], |r| {
                let iban: Option<String> = r.get(4)?;
                Ok((
                    AccountSummary {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        kind: AccountKind::parse(&r.get::<_, String>(2)?),
                        role: r.get(3)?,
                        iban_tail: iban
                            .filter(|i| i.len() >= 4)
                            .map(|i| i[i.len() - 4..].to_owned()),
                        balances: Vec::new(),
                        transaction_count: 0,
                        last_activity: r.get(6)?,
                    },
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut sums: HashMap<i64, (Sums, u64)> = HashMap::new();
        let mut stmt = self
            .conn
            .prepare("SELECT account_id, amount, currency FROM postings")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let entry = sums.entry(r.get(0)?).or_default();
            entry
                .0
                .add(&r.get::<_, String>(2)?, decimal(&r.get::<_, String>(1)?)?);
            entry.1 += 1;
        }
        for (account, _) in &mut accounts {
            if let Some((s, count)) = sums.remove(&account.id) {
                account.balances = s.into_money();
                account.transaction_count = count;
            }
        }
        // Keep closed accounts only while they still hold money.
        accounts.retain(|(a, active)| *active || a.balances.iter().any(|m| !m.amount.is_zero()));
        let mut accounts: Vec<AccountSummary> = accounts.into_iter().map(|(a, _)| a).collect();
        accounts.sort_by(|a, b| {
            let total =
                |s: &AccountSummary| s.balances.iter().map(|m| m.amount.abs()).sum::<Decimal>();
            total(b).cmp(&total(a)).then_with(|| a.name.cmp(&b.name))
        });
        Ok(accounts)
    }

    /// Per-account posting counts and sums for migration checks.
    pub fn account_checks(&self) -> Result<Vec<AccountCheck>> {
        let mut checks: BTreeMap<i64, (u64, Sums)> = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT a.legacy_firefly_id, p.amount, p.currency FROM postings p JOIN accounts a ON a.id = p.account_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let entry = checks.entry(r.get(0)?).or_default();
            entry.0 += 1;
            entry
                .1
                .add(&r.get::<_, String>(2)?, decimal(&r.get::<_, String>(1)?)?);
        }
        Ok(checks
            .into_iter()
            .map(|(legacy_id, (postings, sums))| AccountCheck {
                legacy_id,
                postings,
                sums: sums.into_money(),
            })
            .collect())
    }
}

fn own_kinds_sql() -> String {
    [
        AccountKind::Asset,
        AccountKind::Liability,
        AccountKind::Cash,
    ]
    .iter()
    .map(|k| format!("'{}'", k.as_str()))
    .collect::<Vec<_>>()
    .join(", ")
}
