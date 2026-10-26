//! Read queries behind the app's screens.
//!
//! Amounts are stored as exact decimal strings, so every sum happens in Rust
//! with [`Decimal`], never as SQLite floating point.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use finance_types::{
    AccountCheck, AccountKind, AccountSummary, CategoryTotal, DataStatus, Facets, Money, MonthFlow,
    Named, Overview, TxKind, TxPage, TxQuery, TxRow,
};
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Row, params_from_iter};
use rust_decimal::Decimal;

use crate::{Error, Result, Store};

const MAX_PAGE: u32 = 500;

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

/// The posting that represents a transaction from the owner's point of view.
struct Leg {
    amount: Decimal,
    currency: String,
    foreign: Option<Money>,
}

/// A transaction with all its postings, keyed by account.
struct TxLegs {
    id: i64,
    date: String,
    kind: TxKind,
    legs: Vec<(i64, Leg)>,
}

fn leg_for(kind: TxKind, legs: &[(i64, Leg)], account_id: Option<i64>) -> Option<&Leg> {
    if let Some(account) = account_id
        && let Some((_, leg)) = legs.iter().find(|(a, _)| *a == account)
    {
        return Some(leg);
    }
    let negative = legs
        .iter()
        .find(|(_, l)| l.amount.is_sign_negative())
        .map(|(_, l)| l);
    let positive = legs
        .iter()
        .find(|(_, l)| !l.amount.is_sign_negative())
        .map(|(_, l)| l);
    match kind {
        TxKind::Withdrawal => negative.or(positive),
        _ => positive.or(negative),
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

    pub fn facets(&self) -> Result<Facets> {
        let own = own_kinds_sql();
        let accounts = self.named(&format!(
            "SELECT id, name FROM accounts WHERE kind IN ({own}) ORDER BY active DESC, name COLLATE NOCASE"
        ))?;
        let categories =
            self.named("SELECT id, name FROM categories ORDER BY name COLLATE NOCASE")?;
        let status = self.status()?;
        Ok(Facets {
            accounts,
            categories,
            first_date: status.first_date,
            last_date: status.last_date,
        })
    }

    fn named(&self, sql: &str) -> Result<Vec<Named>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(Named {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

    /// Every transaction with its legs, filtered by `where_sql`.
    fn legs(&self, where_sql: &str, values: &[Value]) -> Result<Vec<TxLegs>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT t.id, t.date, t.kind, p.account_id, p.amount, p.currency, p.foreign_amount, p.foreign_currency
             FROM transactions t JOIN postings p ON p.transaction_id = t.id
             WHERE {where_sql}
             ORDER BY t.date DESC, t.id DESC, p.id"
        ))?;
        let mut rows = stmt.query(params_from_iter(values))?;
        let mut out: Vec<TxLegs> = Vec::new();
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let foreign = match (
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ) {
                (Some(amount), Some(currency)) => Some(Money {
                    amount: decimal(&amount)?,
                    currency,
                }),
                _ => None,
            };
            let leg = Leg {
                amount: decimal(&r.get::<_, String>(4)?)?,
                currency: r.get(5)?,
                foreign,
            };
            match out.last_mut() {
                Some(last) if last.id == id => last.legs.push((r.get(3)?, leg)),
                _ => out.push(TxLegs {
                    id,
                    date: r.get(1)?,
                    kind: TxKind::parse(&r.get::<_, String>(2)?),
                    legs: vec![(r.get(3)?, leg)],
                }),
            }
        }
        Ok(out)
    }

    /// Income and spending per month for the last `months` months that have data.
    fn month_flows(&self, months: usize) -> Result<Vec<MonthFlow>> {
        let mut flows: BTreeMap<(String, String), (Decimal, Decimal)> = BTreeMap::new();
        for tx in self.legs("t.kind IN ('withdrawal', 'deposit')", &[])? {
            let Some(leg) = leg_for(tx.kind, &tx.legs, None) else {
                continue;
            };
            let entry = flows
                .entry((tx.date[..7].to_owned(), leg.currency.clone()))
                .or_default();
            match tx.kind {
                TxKind::Withdrawal => entry.1 -= leg.amount,
                _ => entry.0 += leg.amount,
            }
        }
        let mut recent: Vec<String> = flows.keys().map(|(m, _)| m.clone()).collect();
        recent.dedup();
        let keep: Vec<String> = recent.into_iter().rev().take(months).collect();
        Ok(flows
            .into_iter()
            .filter(|((m, _), _)| keep.contains(m))
            .map(|((month, currency), (income, spending))| MonthFlow {
                month,
                currency,
                income: income.normalize(),
                spending: spending.normalize(),
            })
            .collect())
    }

    /// Spending per category in `month` (`YYYY-MM`), largest first.
    pub fn category_totals(&self, month: &str) -> Result<Vec<CategoryTotal>> {
        let names: HashMap<i64, String> = self
            .named("SELECT id, name FROM categories")?
            .into_iter()
            .map(|n| (n.id, n.name))
            .collect();
        let mut stmt = self
            .conn
            .prepare("SELECT id, category_id FROM transactions WHERE substr(date, 1, 7) = ?1")?;
        let categories: HashMap<i64, Option<i64>> = stmt
            .query_map([month], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut totals: HashMap<(Option<i64>, String), (Decimal, u64)> = HashMap::new();
        for tx in self.legs(
            "t.kind = 'withdrawal' AND substr(t.date, 1, 7) = ?1",
            &[Value::Text(month.to_owned())],
        )? {
            let Some(leg) = leg_for(tx.kind, &tx.legs, None) else {
                continue;
            };
            let category = categories.get(&tx.id).copied().flatten();
            let entry = totals.entry((category, leg.currency.clone())).or_default();
            entry.0 -= leg.amount;
            entry.1 += 1;
        }
        let mut out: Vec<CategoryTotal> = totals
            .into_iter()
            .map(|((category, currency), (amount, count))| CategoryTotal {
                category: category.map(|id| Named {
                    id,
                    name: names.get(&id).cloned().unwrap_or_default(),
                }),
                currency,
                amount: amount.normalize(),
                count,
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.amount));
        Ok(out)
    }

    pub fn overview(&self) -> Result<Overview> {
        let accounts = self.accounts()?;
        let include: HashMap<i64, bool> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, include_net_worth FROM accounts")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut net = Sums::default();
        for a in accounts
            .iter()
            .filter(|a| include.get(&a.id).copied().unwrap_or(true))
        {
            for m in &a.balances {
                net.add(&m.currency, m.amount);
            }
        }
        let months = self.month_flows(13)?;
        let category_month = months.last().map(|m| m.month.clone());
        let categories = match &category_month {
            Some(month) => self.category_totals(month)?,
            None => Vec::new(),
        };
        Ok(Overview {
            net_worth: net.into_money(),
            accounts,
            months,
            categories,
            category_month,
            status: self.status()?,
        })
    }

    pub fn transactions(&self, query: &TxQuery) -> Result<TxPage> {
        let mut clauses = vec!["1 = 1".to_owned()];
        let mut values: Vec<Value> = Vec::new();
        // Every `?` in a clause refers to that clause's one bound value.
        let mut bind = |clause: &str, value: Value, clauses: &mut Vec<String>| {
            values.push(value);
            clauses.push(clause.replace('?', &format!("?{}", values.len())));
        };
        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            if text.chars().count() >= 3 {
                let phrase = format!("\"{}\"", text.replace('"', "\"\""));
                bind(
                    "t.id IN (SELECT rowid FROM transactions_search WHERE transactions_search MATCH ?)",
                    Value::Text(phrase),
                    &mut clauses,
                );
            } else {
                bind(
                    "t.description LIKE ? ESCAPE '\\'",
                    Value::Text(format!("%{}%", escape_like(text))),
                    &mut clauses,
                );
            }
        }
        if let Some(account) = query.account_id {
            bind(
                "(t.source_account_id = ? OR t.destination_account_id = ?)",
                Value::Integer(account),
                &mut clauses,
            );
        }
        if let Some(category) = query.category_id {
            bind("t.category_id = ?", Value::Integer(category), &mut clauses);
        } else if query.uncategorised {
            clauses.push("t.category_id IS NULL AND t.kind = 'withdrawal'".to_owned());
        }
        if let Some(kind) = query.kind {
            bind(
                "t.kind = ?",
                Value::Text(kind.as_str().to_owned()),
                &mut clauses,
            );
        }
        if let Some(from) = &query.from {
            bind("t.date >= ?", Value::Text(from.clone()), &mut clauses);
        }
        if let Some(to) = &query.to {
            bind("t.date <= ?", Value::Text(to.clone()), &mut clauses);
        }
        let where_sql = clauses.join(" AND ");

        let all = self.legs(&where_sql, &values)?;
        // Net money in and out: on one account everything counts; across all
        // accounts, moves between own accounts cancel out and are left out.
        let mut sums = Sums::default();
        for tx in &all {
            let counts = query.account_id.is_some()
                || matches!(tx.kind, TxKind::Withdrawal | TxKind::Deposit);
            if counts && let Some(leg) = leg_for(tx.kind, &tx.legs, query.account_id) {
                sums.add(&leg.currency, leg.amount);
            }
        }
        let total = all.len() as u64;
        let limit = query.limit.clamp(1, MAX_PAGE) as usize;
        let page: Vec<_> = all
            .into_iter()
            .skip(query.offset as usize)
            .take(limit)
            .collect();

        let mut rows = Vec::with_capacity(page.len());
        let mut detail = self.conn.prepare(
            "SELECT t.description, t.source_account_id, sa.name, t.destination_account_id, da.name,
                    t.category_id, c.name, b.name, t.tags, t.notes, t.reconciled
             FROM transactions t
             JOIN accounts sa ON sa.id = t.source_account_id
             JOIN accounts da ON da.id = t.destination_account_id
             LEFT JOIN categories c ON c.id = t.category_id
             LEFT JOIN bills b ON b.id = t.bill_id
             WHERE t.id = ?1",
        )?;
        for TxLegs {
            id,
            date,
            kind,
            legs,
        } in page
        {
            let leg = leg_for(kind, &legs, query.account_id)
                .ok_or_else(|| Error::Corrupt(format!("transaction {id} has no postings")))?;
            let amount = Money {
                amount: leg.amount.normalize(),
                currency: leg.currency.clone(),
            };
            let foreign = leg.foreign.clone();
            rows.push(detail.query_row([id], |r| row_from(r, id, date, kind, amount, foreign))?);
        }
        Ok(TxPage {
            rows,
            total,
            sums: sums.into_money(),
        })
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

fn row_from(
    r: &Row<'_>,
    id: i64,
    date: String,
    kind: TxKind,
    amount: Money,
    foreign: Option<Money>,
) -> rusqlite::Result<TxRow> {
    let category = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<String>>(6)?) {
        (Some(id), Some(name)) => Some(Named { id, name }),
        _ => None,
    };
    let tags: String = r.get(8)?;
    Ok(TxRow {
        id,
        date,
        description: r.get(0)?,
        kind,
        amount,
        foreign,
        source: Named {
            id: r.get(1)?,
            name: r.get(2)?,
        },
        destination: Named {
            id: r.get(3)?,
            name: r.get(4)?,
        },
        category,
        bill: r.get(7)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        notes: r.get(9)?,
        reconciled: r.get(10)?,
    })
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

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
