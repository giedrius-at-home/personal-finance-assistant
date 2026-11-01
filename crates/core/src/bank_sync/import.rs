//! Write downloaded entries into the ledger as the Firefly importer did
//! (`GenerateTransactions.php`): money out is a withdrawal to the creditor's
//! expense account, money in a deposit from the debtor's revenue account, and
//! a counterparty IBAN of another own asset account makes it a transfer.
//! Entries are deduplicated on the bank's reference per account.

use std::collections::HashMap;
use std::sync::LazyLock;

use finance_storage::{NewTransaction, counterparty_account, insert_transaction, search_text};
use jiff::civil::Date;
use regex::Regex;
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

/// A description that names a savings pocket.
static SAVINGS_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(savings?|vault|pocket|spaar\w*)\b").expect("savings hint pattern")
});

/// Savings routes from the owner's account roles: (via own account, savings
/// account), for savings accounts that are active assets of the ledger.
/// `account_roles` is a gateway table; without it there are no routes.
fn savings_routes(conn: &Connection, own: &HashMap<i64, OwnAccount>) -> Result<Vec<(i64, i64)>> {
    let present: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'account_roles')",
        [],
        |r| r.get(0),
    )?;
    if !present {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT payload_json FROM account_roles")?;
    let payloads = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let firefly_id = |value: Option<&str>| {
        value?
            .strip_prefix("firefly:")
            .and_then(|id| id.parse::<i64>().ok())
    };
    let mut routes = Vec::new();
    for payload in payloads {
        let role: serde_json::Value = match serde_json::from_str(&payload?) {
            Ok(role) => role,
            Err(_) => continue,
        };
        if role.get("role").and_then(|r| r.as_str()) != Some("savings") {
            continue;
        }
        let via = firefly_id(role.get("via_account_id").and_then(|v| v.as_str()));
        let target = firefly_id(role.get("account_id").and_then(|v| v.as_str()));
        if let (Some(via), Some(target)) = (via, target)
            && own
                .get(&target)
                .is_some_and(|a| a.kind == "asset" && a.active)
        {
            routes.push((via, target));
        }
    }
    Ok(routes)
}

/// A pocket the consent does not share (Revolut's Instant Access Savings):
/// "To <savings>" / "From <savings>" moves money to or from the one savings
/// account routed via `account`. Zero or several candidates: no match.
fn savings_pocket(routes: &[(i64, i64)], account: i64, entry: &Entry) -> Option<i64> {
    let description = entry.description().to_lowercase();
    let lead = if entry.amount.is_sign_negative() {
        "to "
    } else {
        "from "
    };
    if !description.starts_with(lead) || !SAVINGS_HINT.is_match(&description) {
        return None;
    }
    let mut candidates = routes
        .iter()
        .filter(|(via, target)| *via == account && *target != account)
        .map(|(_, target)| *target);
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}

/// The other leg of a transfer inside one consent, imported before this
/// account was known: the same bank reference on another account of the
/// consent, the opposite way, same amount and day, booked against a bare
/// expense or revenue counterparty. It becomes the transfer; Ok(true) when
/// one was adopted.
#[allow(clippy::too_many_arguments)]
fn adopt_first_leg(
    conn: &Connection,
    connection: &Consent,
    account: i64,
    entry_id: &str,
    outgoing: bool,
    amount: Decimal,
    date: Date,
    own: &HashMap<i64, OwnAccount>,
) -> Result<bool> {
    // This entry pays out: the first leg is a deposit into the other account.
    let (kind, own_side, counter_side) = if outgoing {
        ("deposit", "destination_account_id", "source_account_id")
    } else {
        ("withdrawal", "source_account_id", "destination_account_id")
    };
    let sql = format!(
        "SELECT t.id, t.{own_side}, t.{counter_side}, t.description, t.notes, t.tags, p.amount \
         FROM transactions t \
         JOIN postings p ON p.transaction_id = t.id AND p.account_id = t.{own_side} \
         JOIN accounts c ON c.id = t.{counter_side} AND c.kind IN ('expense', 'revenue') \
         WHERE t.external_id = ?1 AND t.kind = '{kind}' AND t.date = ?2 AND t.{own_side} = ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    for &other in connection.accounts.values() {
        if other == account {
            continue;
        }
        let found = stmt
            .query_row(params![entry_id, date.to_string(), other], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .optional()?;
        let Some((id, counterparty, description, notes, tags, posted)) = found else {
            continue;
        };
        if posted.parse::<Decimal>().ok().map(|d| d.abs()) != Some(amount) {
            continue;
        }
        let (source, destination) = if outgoing {
            (account, other)
        } else {
            (other, account)
        };
        conn.execute(
            "UPDATE postings SET account_id = ?1 WHERE transaction_id = ?2 AND account_id = ?3",
            params![account, id, counterparty],
        )?;
        conn.execute(
            "UPDATE transactions SET kind = 'transfer', source_account_id = ?1, \
             destination_account_id = ?2, category_id = NULL WHERE id = ?3",
            params![source, destination, id],
        )?;
        let tags: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
        let name = |account: i64| own.get(&account).map(|a| a.name.as_str());
        let text = search_text(
            &description,
            name(source),
            name(destination),
            None,
            notes.as_deref(),
            &tags,
        );
        conn.execute("DELETE FROM transactions_search WHERE rowid = ?1", [id])?;
        conn.execute(
            "INSERT INTO transactions_search (rowid, text) VALUES (?1, ?2)",
            params![id, text],
        )?;
        return Ok(true);
    }
    Ok(false)
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
    let routes = savings_routes(conn, own)?;
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
            // The other leg of a pocket transfer may already be in the ledger
            // as a purchase or a gift; it becomes the transfer.
            if other.is_none()
                && adopt_first_leg(
                    conn, connection, account, &entry.id, outgoing, amount, date, own,
                )?
            {
                continue;
            }
            let other = other.or_else(|| savings_pocket(&routes, account, entry));
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
    use crate::writes::fixture::{Db, db};

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

    fn window() -> Window {
        Window {
            from: jiff::civil::date(2026, 10, 1),
            to: jiff::civil::date(2026, 11, 1),
        }
    }

    fn pocket_db() -> Db {
        let db = db();
        db.store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (70, 'Revolut', 'asset'),
                 (71, 'Revolut Groceries', 'asset');",
            )
            .unwrap();
        db
    }

    type Downloads = Vec<(String, Vec<Entry>)>;

    fn pocket_legs() -> (Downloads, Downloads) {
        let main = vec![(
            "main".to_owned(),
            vec![entry(
                json!({"entry_reference": "X", "booking_date": "2026-11-01",
                "transaction_amount": {"amount": "300.00", "currency": "EUR"},
                "credit_debit_indicator": "DBIT",
                "remittance_information": ["To EUR MB:a4ba"]}),
            )],
        )];
        let pocket = vec![(
            "pocket".to_owned(),
            vec![entry(
                json!({"entry_reference": "X", "booking_date": "2026-11-01",
                "transaction_amount": {"amount": "300.00", "currency": "EUR"},
                "credit_debit_indicator": "CRDT",
                "remittance_information": ["To EUR MB:a4ba"]}),
            )],
        )];
        (main, pocket)
    }

    fn run(db: &mut Db, consent: &Consent, downloads: &[(String, Vec<Entry>)]) -> Imported {
        db.write(|w, _| {
            let own = own_accounts(&w.tx)?;
            import(w, "revolut", consent, downloads, window(), &own)
        })
        .unwrap()
    }

    fn assert_paired(db: &Db) {
        let rows: i64 = db.one("SELECT count(*) FROM transactions WHERE external_id = 'X'");
        assert_eq!(rows, 1);
        let shape: String = db.one(
            "SELECT kind || ':' || source_account_id || '>' || destination_account_id \
             || ':' || coalesce(category_id, 'none') FROM transactions WHERE external_id = 'X'",
        );
        assert_eq!(shape, "transfer:70>71:none");
        let postings: String = db.one(
            "SELECT group_concat(p.account_id || '=' || p.amount, ',') FROM \
             (SELECT p.account_id, p.amount FROM postings p JOIN transactions t ON t.id = p.transaction_id \
              WHERE t.external_id = 'X' ORDER BY p.account_id) p",
        );
        assert_eq!(postings, "70=-300.00,71=300.00");
        let text: i64 = db.one(
            "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH '\"Revolut Groceries\"' \
             AND rowid = (SELECT id FROM transactions WHERE external_id = 'X')",
        );
        assert_eq!(text, 1);
        let stale: i64 = db.one(
            "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH '\"unknown\"'",
        );
        assert_eq!(stale, 0);
    }

    #[test]
    fn second_leg_adopts_the_first_main_account_first() {
        let mut db = pocket_db();
        // No names in the saved consent: the sibling lookup cannot match.
        let revolut = consent(&[("main", 70), ("pocket", 71)], &[]);
        let (main, pocket) = pocket_legs();
        let first = run(&mut db, &revolut, &main);
        assert_eq!(
            first,
            Imported {
                created: 1,
                checked: 1
            }
        );
        let shape: String = db.one(
            "SELECT t.kind || ':' || a.name FROM transactions t JOIN accounts a ON a.id = t.destination_account_id \
             WHERE external_id = 'X'",
        );
        assert_eq!(shape, "withdrawal:(unknown destination)");

        let second = run(&mut db, &revolut, &pocket);
        assert_eq!(
            second,
            Imported {
                created: 0,
                checked: 1
            }
        );
        assert_paired(&db);
        // Nothing was recorded for the adopted leg.
        let observed: i64 =
            db.one("SELECT count(*) FROM source_versions WHERE source_system = 'enable_banking'");
        assert_eq!(observed, 1);

        // Both again: already imported on both accounts.
        let again = run(&mut db, &revolut, &[main[0].clone(), pocket[0].clone()]);
        assert_eq!(
            again,
            Imported {
                created: 0,
                checked: 2
            }
        );
        assert_paired(&db);
    }

    #[test]
    fn second_leg_adopts_the_first_pocket_first() {
        let mut db = pocket_db();
        let revolut = consent(&[("main", 70), ("pocket", 71)], &[]);
        let (main, pocket) = pocket_legs();
        run(&mut db, &revolut, &pocket);
        let shape: String = db.one(
            "SELECT t.kind || ':' || a.name FROM transactions t JOIN accounts a ON a.id = t.source_account_id \
             WHERE external_id = 'X'",
        );
        assert_eq!(shape, "deposit:(unknown source)");
        let second = run(&mut db, &revolut, &main);
        assert_eq!(
            second,
            Imported {
                created: 0,
                checked: 1
            }
        );
        assert_paired(&db);
    }

    #[test]
    fn first_leg_is_not_adopted_when_amount_or_day_differ() {
        let mut db = pocket_db();
        let revolut = consent(&[("main", 70), ("pocket", 71)], &[]);
        let (main, _) = pocket_legs();
        run(&mut db, &revolut, &main);
        let other_amount = vec![(
            "pocket".to_owned(),
            vec![entry(
                json!({"entry_reference": "X", "booking_date": "2026-11-01",
                "transaction_amount": {"amount": "299.00", "currency": "EUR"},
                "credit_debit_indicator": "CRDT",
                "remittance_information": ["To EUR MB:a4ba"]}),
            )],
        )];
        let done = run(&mut db, &revolut, &other_amount);
        assert_eq!(
            done,
            Imported {
                created: 1,
                checked: 1
            }
        );
        let kinds: String = db.one(
            "SELECT group_concat(kind, ',') FROM (SELECT kind FROM transactions WHERE external_id = 'X' ORDER BY id)",
        );
        assert_eq!(kinds, "withdrawal,deposit");
    }

    fn savings_role(db: &Db, account: i64, via: i64) {
        db.store
            .connection()
            .execute(
                "INSERT INTO account_roles (account_id, payload_json) VALUES (?1, ?2)",
                params![
                    format!("firefly:{account}"),
                    json!({"account_id": format!("firefly:{account}"), "role": "savings",
                        "goal_id": null, "via_account_id": format!("firefly:{via}"),
                        "confirmed_at": "2026-10-01T00:00:00Z"})
                    .to_string()
                ],
            )
            .unwrap();
    }

    fn savings_entries() -> Downloads {
        vec![(
            "main".to_owned(),
            vec![
                entry(json!({"entry_reference": "S", "booking_date": "2026-11-01",
                    "transaction_amount": {"amount": "913.82", "currency": "EUR"},
                    "credit_debit_indicator": "DBIT",
                    "remittance_information": ["To Instant Access Savings"]})),
                entry(json!({"entry_reference": "B", "booking_date": "2026-11-01",
                    "transaction_amount": {"amount": "50", "currency": "EUR"},
                    "credit_debit_indicator": "CRDT",
                    "remittance_information": ["From Instant Access Savings"]})),
                // Looks like a transfer but names no savings pocket.
                entry(json!({"entry_reference": "P", "booking_date": "2026-11-01",
                    "transaction_amount": {"amount": "12", "currency": "EUR"},
                    "credit_debit_indicator": "DBIT",
                    "remittance_information": ["To Bakery"]})),
            ],
        )]
    }

    #[test]
    fn savings_pocket_routes_to_the_savings_account_via_role() {
        let mut db = pocket_db();
        db.store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (73, 'Revolut Savings', 'asset');",
            )
            .unwrap();
        savings_role(&db, 73, 70);
        let revolut = consent(&[("main", 70)], &[]);
        let done = run(&mut db, &revolut, &savings_entries());
        assert_eq!(
            done,
            Imported {
                created: 3,
                checked: 3
            }
        );
        let shapes: String = db.one(
            "SELECT group_concat(shape, ',') FROM \
             (SELECT t.external_id || '=' || t.kind || ':' || s.name || '>' || d.name AS shape FROM transactions t \
              JOIN accounts s ON s.id = t.source_account_id JOIN accounts d ON d.id = t.destination_account_id \
              WHERE t.external_id IN ('S', 'B', 'P') ORDER BY t.external_id)",
        );
        assert_eq!(
            shapes,
            "B=transfer:Revolut Savings>Revolut,P=withdrawal:Revolut>(unknown destination),S=transfer:Revolut>Revolut Savings"
        );
    }

    #[test]
    fn savings_pocket_needs_exactly_one_route() {
        let mut db = pocket_db();
        db.store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (73, 'Revolut Savings', 'asset'),
                 (74, 'Revolut Vault', 'asset');",
            )
            .unwrap();
        savings_role(&db, 73, 70);
        savings_role(&db, 74, 70);
        let revolut = consent(&[("main", 70)], &[]);
        run(&mut db, &revolut, &savings_entries());
        let shapes: String = db.one(
            "SELECT group_concat(external_id || '=' || kind, ',') FROM \
             (SELECT * FROM transactions WHERE external_id IN ('S', 'B') ORDER BY external_id)",
        );
        assert_eq!(shapes, "B=deposit,S=withdrawal");
    }
}
