//! Write downloaded entries into the ledger as the Firefly importer did
//! (`GenerateTransactions.php`): money out is a withdrawal to the creditor's
//! expense account, money in a deposit from the debtor's revenue account, and
//! a counterparty IBAN of another own asset account makes it a transfer.
//! Entries are deduplicated on the bank's reference per account.

use std::collections::HashMap;

use finance_storage::{NewTransaction, counterparty_account, insert_transaction};
use jiff::civil::Date;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;

use super::{Connection as Consent, Entry, Window, normalize_iban};
use crate::Result;
use crate::writes::Writer;

/// An account of the ledger, as sync needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnAccount {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub active: bool,
    /// Normalised; empty when the account has none.
    pub iban: String,
}

/// Every non-counterparty account of the ledger by id.
pub fn own_accounts(conn: &Connection) -> Result<HashMap<i64, OwnAccount>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, active, coalesce(iban, '') FROM accounts \
         WHERE kind NOT IN ('expense', 'revenue')",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(OwnAccount {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            active: r.get(3)?,
            iban: normalize_iban(&r.get::<_, String>(4)?),
        })
    })?;
    let mut accounts = HashMap::new();
    for row in rows {
        let row = row?;
        accounts.insert(row.id, row);
    }
    Ok(accounts)
}

/// The currency an account's postings use most recently (the importer took
/// the account's currency when an entry named none).
fn account_currency(conn: &Connection, account: i64) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT currency FROM postings WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "EUR".to_owned()))
}

/// Another account of the same consent this entry moves money to or from,
/// by the bank's account name: banks without IBANs for their pockets
/// describe those transfers as "To <pocket>" / "From <pocket>" or name the
/// pocket as the counterparty.
fn sibling(connection: &Consent, uid: &str, account: i64, entry: &Entry) -> Option<i64> {
    let description = entry.description().to_lowercase();
    let counterparty = entry.counterparty().to_lowercase();
    let lead = if entry.amount.is_sign_negative() {
        "to "
    } else {
        "from "
    };
    connection
        .names
        .iter()
        .filter(|(other, name)| *other != uid && name.chars().count() >= 3)
        .filter_map(|(other, name)| {
            let target = *connection.accounts.get(other)?;
            let name = name.to_lowercase();
            (target != account && (description == format!("{lead}{name}") || counterparty == name))
                .then_some(target)
        })
        .next()
}

/// What one sync wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// New ledger transactions.
    pub created: u32,
    /// Booked, non-zero entries inside the window.
    pub checked: u32,
}

fn already_imported(conn: &Connection, external_id: &str, account: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM transactions WHERE external_id = ?1 \
             AND (source_account_id = ?2 OR destination_account_id = ?2) LIMIT 1",
            params![external_id, account],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// A transfer between two own accounts shows up at both banks; the second
/// leg finds the first (same accounts and amount, a few days apart).
fn transfer_exists(
    conn: &Connection,
    source: i64,
    destination: i64,
    amount: Decimal,
    date: Date,
) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT p.amount FROM transactions t \
         JOIN postings p ON p.transaction_id = t.id AND p.account_id = t.destination_account_id \
         WHERE t.kind = 'transfer' AND t.source_account_id = ?1 AND t.destination_account_id = ?2 \
         AND abs(julianday(t.date) - julianday(?3)) <= 4",
    )?;
    let amounts = stmt.query_map(params![source, destination, date.to_string()], |r| {
        r.get::<_, String>(0)
    })?;
    for value in amounts {
        if value?.parse::<Decimal>().ok() == Some(amount) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_source_versions(conn: &Connection) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'source_versions'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Write `downloads` (bank account uid → its entries) for `bank` into the
/// ledger. `accounts` maps uids to ledger accounts.
pub fn import(
    w: &Writer<'_>,
    bank: &str,
    connection: &Consent,
    downloads: &[(String, Vec<Entry>)],
    window: Window,
    own: &HashMap<i64, OwnAccount>,
) -> Result<Imported> {
    let conn: &Connection = &w.tx;
    let observed = has_source_versions(conn)?;
    let today = w.now.date();
    let run_tag = format!("Data Import on {} @ {}", today, w.now.strftime("%H:%M"));
    let mut imported = Imported::default();
    for (uid, entries) in downloads {
        let Some(&account) = connection.accounts.get(uid) else {
            continue;
        };
        let currency = account_currency(conn, account)?;
        for entry in entries {
            // Pending entries are skipped; the booked one comes later.
            if entry.status == "pending" || entry.amount.is_zero() {
                continue;
            }
            let date = entry.date.unwrap_or(today);
            if date < window.from || date > window.to {
                continue;
            }
            imported.checked += 1;
            if already_imported(conn, &entry.id, account)? {
                continue;
            }
            let amount = entry.amount.abs();
            let outgoing = entry.amount.is_sign_negative();
            let other = entry
                .counterparty_iban()
                .and_then(|iban| {
                    own.values()
                        .filter(|a| a.kind == "asset" && a.id != account && a.iban == iban)
                        .min_by_key(|a| (!a.active, a.id))
                        .map(|a| a.id)
                })
                .or_else(|| sibling(connection, uid, account, entry));
            let (kind, source, destination) = match other {
                Some(other) => {
                    let (s, d) = if outgoing {
                        (account, other)
                    } else {
                        (other, account)
                    };
                    if transfer_exists(conn, s, d, amount, date)? {
                        continue;
                    }
                    ("transfer", s, d)
                }
                None if outgoing => (
                    "withdrawal",
                    account,
                    counterparty_account(conn, &entry.counterparty(), "expense")?,
                ),
                None => (
                    "deposit",
                    counterparty_account(conn, &entry.counterparty(), "revenue")?,
                    account,
                ),
            };
            let mut tags = Vec::new();
            if !entry.status.is_empty() {
                tags.push(entry.status.clone());
            }
            tags.extend(entry.sub_code.clone());
            tags.push(run_tag.clone());
            let description = entry.description();
            let notes = entry.notes();
            let day = date.to_string();
            let currency = if entry.currency.is_empty() {
                currency.as_str()
            } else {
                entry.currency.as_str()
            };
            let id = insert_transaction(
                conn,
                &NewTransaction {
                    kind,
                    date: &day,
                    description: &description,
                    source_account_id: source,
                    destination_account_id: destination,
                    amount,
                    currency,
                    external_id: Some(&entry.id),
                    internal_reference: Some(uid),
                    notes: notes.as_deref(),
                    tags: &tags,
                },
            )?;
            // When it was first seen, for the closed-month checks.
            if observed {
                let payload = serde_json::json!({
                    "bank": bank,
                    "date": day,
                    "amount": entry.amount.to_string(),
                    "currency": currency,
                });
                conn.execute(
                    "INSERT OR IGNORE INTO source_versions (source_system, source_entity, external_id, \
                     content_hash, observed_at, record_kind, canonical_id, payload_json) \
                     VALUES ('enable_banking', 'transaction', ?1, ?2, ?3, 'transaction', ?4, ?5)",
                    params![
                        format!("{uid}:{}", entry.id),
                        entry.content_hash,
                        w.stamp(),
                        format!("firefly:{id}"),
                        payload.to_string(),
                    ],
                )?;
            }
            imported.created += 1;
        }
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::writes::fixture::db;

    fn entry(value: serde_json::Value) -> Entry {
        Entry::parse(&value, "booked")
    }

    fn consent(accounts: &[(&str, i64)], names: &[(&str, &str)]) -> Consent {
        Consent {
            sessions: vec!["s".into()],
            accounts: accounts
                .iter()
                .map(|(u, t)| ((*u).to_owned(), *t))
                .collect(),
            verified_identity: BTreeMap::new(),
            import_not_before: None,
            names: names
                .iter()
                .map(|(u, n)| ((*u).to_owned(), (*n).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn imports_new_entries_once() {
        let mut db = db();
        db.store
            .connection()
            .execute_batch(
                "UPDATE accounts SET iban = 'NL01BANK0000000001' WHERE id = 1;
                 INSERT INTO accounts (id, name, kind, iban) VALUES (50, 'Savings', 'asset', 'NL02BANK0000000002');",
            )
            .unwrap();
        let window = Window {
            from: jiff::civil::date(2026, 10, 1),
            to: jiff::civil::date(2026, 10, 26),
        };
        let accounts = consent(&[("uid-1", 1)], &[]);
        let downloads = vec![(
            "uid-1".to_owned(),
            vec![
                entry(
                    json!({"entry_reference": "r1", "booking_date": "2026-10-20",
                    "transaction_amount": {"amount": "4.50", "currency": "EUR"},
                    "credit_debit_indicator": "DBIT", "creditor": {"name": "Bakery"},
                    "remittance_information": ["Bakery Utrecht"]}),
                ),
                entry(
                    json!({"entry_reference": "r2", "booking_date": "2026-10-21",
                    "transaction_amount": {"amount": "100", "currency": "EUR"},
                    "credit_debit_indicator": "DBIT",
                    "creditor_account": {"iban": "NL02 BANK 0000 0000 02"}}),
                ),
                // Outside the window, zero and pending entries are left out.
                entry(
                    json!({"entry_reference": "r3", "booking_date": "2026-09-01",
                    "transaction_amount": {"amount": "1", "currency": "EUR"}}),
                ),
                entry(
                    json!({"entry_reference": "r4", "booking_date": "2026-10-21",
                    "transaction_amount": {"amount": "0", "currency": "EUR"}}),
                ),
            ],
        )];
        let first = db
            .write(|w, _| {
                let own = own_accounts(&w.tx)?;
                import(w, "abnamro", &accounts, &downloads, window, &own)
            })
            .unwrap();
        assert_eq!(
            first,
            Imported {
                created: 2,
                checked: 2
            }
        );
        let kinds: String = db.one(
            "SELECT group_concat(kind || ':' || internal_reference, ',') FROM \
             (SELECT kind, internal_reference FROM transactions WHERE external_id IN ('r1','r2') ORDER BY external_id)",
        );
        assert_eq!(kinds, "withdrawal:uid-1,transfer:uid-1");
        let bakery: String = db.one(
            "SELECT a.name FROM transactions t JOIN accounts a ON a.id = t.destination_account_id WHERE external_id = 'r1'",
        );
        assert_eq!(bakery, "Bakery");
        let observed: i64 =
            db.one("SELECT count(*) FROM source_versions WHERE source_system = 'enable_banking'");
        assert_eq!(observed, 2);

        // A second run finds both again and writes nothing.
        let second = db
            .write(|w, _| {
                let own = own_accounts(&w.tx)?;
                import(w, "abnamro", &accounts, &downloads, window, &own)
            })
            .unwrap();
        assert_eq!(
            second,
            Imported {
                created: 0,
                checked: 2
            }
        );

        // The savings bank reports the same transfer from its side.
        let savings = consent(&[("uid-2", 50)], &[]);
        let other_leg = vec![(
            "uid-2".to_owned(),
            vec![entry(
                json!({"entry_reference": "s1", "booking_date": "2026-10-22",
                "transaction_amount": {"amount": "100", "currency": "EUR"},
                "credit_debit_indicator": "CRDT",
                "debtor_account": {"iban": "NL01BANK0000000001"}}),
            )],
        )];
        let third = db
            .write(|w, _| {
                let own = own_accounts(&w.tx)?;
                import(w, "savings", &savings, &other_leg, window, &own)
            })
            .unwrap();
        assert_eq!(
            third,
            Imported {
                created: 0,
                checked: 1
            }
        );
    }

    #[test]
    fn pocket_transfers_match_by_account_name() {
        let mut db = db();
        db.store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (60, 'Revolut Savings', 'asset'),
                 (61, 'Revolut Fun out', 'asset');",
            )
            .unwrap();
        let window = Window {
            from: jiff::civil::date(2026, 10, 1),
            to: jiff::civil::date(2026, 10, 26),
        };
        let revolut = consent(
            &[("savings", 60), ("fun", 61)],
            &[("savings", "Instant Access Savings"), ("fun", "Fun out")],
        );
        let downloads = vec![
            (
                "fun".to_owned(),
                vec![entry(
                    json!({"entry_reference": "f1", "booking_date": "2026-10-16",
                    "transaction_amount": {"amount": "300", "currency": "EUR"},
                    "credit_debit_indicator": "CRDT",
                    "remittance_information": ["From Instant Access Savings"]}),
                )],
            ),
            (
                "savings".to_owned(),
                vec![entry(
                    json!({"entry_reference": "s1", "booking_date": "2026-10-16",
                    "transaction_amount": {"amount": "300", "currency": "EUR"},
                    "credit_debit_indicator": "DBIT",
                    "remittance_information": ["To Fun out"]}),
                )],
            ),
        ];
        let done = db
            .write(|w, _| {
                let own = own_accounts(&w.tx)?;
                import(w, "revolut", &revolut, &downloads, window, &own)
            })
            .unwrap();
        // One transfer: the second leg finds the first.
        assert_eq!(
            done,
            Imported {
                created: 1,
                checked: 2
            }
        );
        let kind: String = db.one(
            "SELECT kind || ':' || source_account_id || '>' || destination_account_id FROM transactions WHERE external_id = 'f1'",
        );
        assert_eq!(kind, "transfer:60>61");
    }
}
