//! Category consolidation (Python `web.py` `/categories/consolidate/{action}`
//! and `category_catalog.consolidation_plan`, behind the site's
//! `/api/finance/categories` route, whose messages the owner sees).
//!
//! Python changed each transaction's category in Firefly one by one; here the
//! ledger is local, so every affected transaction moves to the kept category
//! in one transaction and the search index follows. Like Python, saved group
//! mappings, budget plans and other gateway rows are left as they are: they
//! name categories, and the old name simply stops being used.

use rusqlite::{Connection, params, params_from_iter};
use sha2::{Digest, Sha256};

use finance_types::report::catalog::{ConsolidationPreview, ConsolidationResult};

use super::{Writer, invalid};
use crate::{Error, Result};

/// Review policy (`config/review-policy.json` → `categories.consolidation_limit`).
pub const CONSOLIDATION_LIMIT: usize = 100;

const CHOOSE: &str = "Choose two current categories.";
const PREVIEW_FIRST: &str = "Preview the consolidation first.";
const CHANGED: &str = "Transactions changed. Refresh and preview again.";

fn cannot() -> Error {
    invalid(format!(
        "Cannot consolidate this selection. Choose two current categories with 1–{CONSOLIDATION_LIMIT} affected transactions."
    ))
}

fn check_names(source: &str, target: &str) -> Result<()> {
    let ok = |s: &str| (1..=128).contains(&s.chars().count());
    if ok(source) && ok(target) {
        Ok(())
    } else {
        Err(invalid(CHOOSE))
    }
}

/// The transactions filed under `source` (by ledger id) and a fingerprint of
/// them that changes whenever any of them changes.
fn plan(conn: &Connection, source: &str, target: &str) -> Result<(Vec<i64>, String)> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT c.name FROM transactions t JOIN categories c ON c.id = t.category_id \
         WHERE c.name <> ''",
    )?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    if source == target || !names.iter().any(|n| n == source) || !names.iter().any(|n| n == target)
    {
        return Err(cannot());
    }
    let mut stmt = conn.prepare(
        "SELECT t.id, t.category_id, t.kind, t.booked_at, t.description, t.notes, t.tags, \
                t.source_account_id, t.destination_account_id, t.bill_id, \
                (SELECT group_concat(p.account_id || ':' || p.amount || ':' || p.currency, ',') \
                 FROM postings p WHERE p.transaction_id = t.id) \
         FROM transactions t JOIN categories c ON c.id = t.category_id \
         WHERE c.name = ?1 ORDER BY t.id",
    )?;
    let rows: Vec<serde_json::Value> = stmt
        .query_map([source], |r| {
            Ok(serde_json::json!([
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<String>>(10)?,
            ]))
        })?
        .collect::<std::result::Result<_, _>>()?;
    if rows.is_empty() || rows.len() > CONSOLIDATION_LIMIT {
        return Err(cannot());
    }
    let ids = rows.iter().filter_map(|r| r[0].as_i64()).collect();
    let payload = serde_json::json!([source, target, rows]).to_string();
    let token = Sha256::digest(payload.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((ids, token))
}

/// How many transactions would move from `source` to `target`.
pub fn preview_consolidation(
    conn: &Connection,
    source: &str,
    target: &str,
) -> Result<ConsolidationPreview> {
    check_names(source, target)?;
    let (ids, token) = plan(conn, source, target)?;
    Ok(ConsolidationPreview {
        source: source.to_owned(),
        target: target.to_owned(),
        count: ids.len() as u64,
        token,
    })
}

/// Move every transaction filed under `source` to `target`, if they are still
/// exactly the ones previewed with `token`.
pub fn apply_consolidation(
    w: &Writer<'_>,
    source: &str,
    target: &str,
    token: Option<&str>,
) -> Result<ConsolidationResult> {
    check_names(source, target)?;
    let Some(token) = token else {
        return Err(invalid(PREVIEW_FIRST));
    };
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid(CHOOSE));
    }
    let (ids, current) = plan(&w.tx, source, target)?;
    if current != token {
        return Err(invalid(CHANGED));
    }
    let target_id: i64 = w.tx.query_row(
        "SELECT MIN(c.id) FROM categories c JOIN transactions t ON t.category_id = c.id \
         WHERE c.name = ?1",
        [target],
        |r| r.get(0),
    )?;
    let marks = vec!["?"; ids.len()].join(",");
    w.tx.execute(
        &format!("UPDATE transactions SET category_id = ? WHERE id IN ({marks})"),
        params_from_iter(std::iter::once(target_id).chain(ids.iter().copied())),
    )?;
    reindex(&w.tx, &ids)?;
    let applied = ids.len() as u64;
    w.record(
        "category-consolidation",
        &serde_json::json!({ "source": source, "target": target, "transactions": applied }),
    )?;
    Ok(ConsolidationResult {
        applied,
        total: applied,
        complete: true,
        refreshed: true,
        error: None,
    })
}

/// Rebuild the search text of `ids` the way the ledger import writes it:
/// description, both account names, category, notes and tags.
pub fn reindex(conn: &Connection, ids: &[i64]) -> Result<()> {
    let mut read = conn.prepare(
        "SELECT t.description, s.name, d.name, c.name, t.notes, t.tags FROM transactions t \
         LEFT JOIN accounts s ON s.id = t.source_account_id \
         LEFT JOIN accounts d ON d.id = t.destination_account_id \
         LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = ?1",
    )?;
    let mut delete = conn.prepare("DELETE FROM transactions_search WHERE rowid = ?1")?;
    let mut insert =
        conn.prepare("INSERT INTO transactions_search (rowid, text) VALUES (?1, ?2)")?;
    for id in ids {
        let (description, source, destination, category, notes, tags) =
            read.query_row([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
        let tags: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
        let mut text = vec![description];
        text.extend(source);
        text.extend(destination);
        text.extend(category);
        text.extend(notes);
        text.extend(tags);
        delete.execute([id])?;
        insert.execute(params![id, text.join(" \u{2022} ")])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_storage::Store;
    use jiff::Zoned;

    fn store() -> Store {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind) VALUES (1, 'Checking', 'asset'), (2, 'Shop', 'expense');
                 INSERT INTO categories (id, name) VALUES (10, 'Groceries'), (11, 'groceries'), (12, 'Unused');
                 INSERT INTO transactions (id, kind, date, booked_at, description, source_account_id,
                     destination_account_id, category_id, tags)
                 VALUES (1, 'withdrawal', '2026-10-01', '2026-10-01T10:00:00+02:00', 'AH', 1, 2, 10, '[]'),
                        (2, 'withdrawal', '2026-10-02', '2026-10-02T10:00:00+02:00', 'Jumbo', 1, 2, 11, '[\"food\"]'),
                        (3, 'withdrawal', '2026-10-03', '2026-10-03T10:00:00+02:00', 'Lidl', 1, 2, 11, '[]');
                 INSERT INTO postings (transaction_id, account_id, amount, currency)
                 VALUES (1, 1, '-5', 'EUR'), (1, 2, '5', 'EUR'), (2, 1, '-6', 'EUR'), (2, 2, '6', 'EUR'),
                        (3, 1, '-7', 'EUR'), (3, 2, '7', 'EUR');
                 INSERT INTO transactions_search (rowid, text) VALUES (1, 'AH'), (2, 'Jumbo'), (3, 'Lidl');",
            )
            .unwrap();
        store
    }

    fn writer(store: &mut Store) -> Writer<'_> {
        Writer {
            tx: store.connection_mut().transaction().unwrap(),
            now: Zoned::now(),
        }
    }

    #[test]
    fn previews_then_moves_transactions_and_reindexes() {
        let mut store = store();
        let preview = preview_consolidation(store.connection(), "groceries", "Groceries").unwrap();
        assert_eq!(preview.count, 2);
        assert_eq!(preview.token.len(), 64);
        let w = writer(&mut store);
        let result =
            apply_consolidation(&w, "groceries", "Groceries", Some(&preview.token)).unwrap();
        assert_eq!(
            (result.applied, result.total, result.complete),
            (2, 2, true)
        );
        let moved: i64 =
            w.tx.query_row(
                "SELECT COUNT(*) FROM transactions WHERE category_id = 10",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(moved, 3);
        let found: Vec<i64> = w
            .tx
            .prepare("SELECT rowid FROM transactions_search WHERE transactions_search MATCH '\"Groceries\"' ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(found, vec![2, 3]);
        // The old name is no longer in use, so it cannot be chosen again.
        assert!(preview_consolidation(&w.tx, "groceries", "Groceries").is_err());
    }

    #[test]
    fn refuses_stale_previews_and_bad_selections() {
        let mut store = store();
        let preview = preview_consolidation(store.connection(), "groceries", "Groceries").unwrap();
        store
            .connection()
            .execute(
                "UPDATE transactions SET description = 'Lidl NL' WHERE id = 3",
                [],
            )
            .unwrap();
        let w = writer(&mut store);
        let stale = apply_consolidation(&w, "groceries", "Groceries", Some(&preview.token));
        assert_eq!(stale.unwrap_err().to_string(), CHANGED);
        let missing = apply_consolidation(&w, "groceries", "Groceries", None);
        assert_eq!(missing.unwrap_err().to_string(), PREVIEW_FIRST);
        let bad = apply_consolidation(&w, "groceries", "Groceries", Some("xyz"));
        assert_eq!(bad.unwrap_err().to_string(), CHOOSE);
        for (source, target) in [
            ("Groceries", "Groceries"),
            ("Unused", "Groceries"),
            ("", "x"),
        ] {
            assert!(preview_consolidation(&w.tx, source, target).is_err());
        }
        assert!(
            preview_consolidation(&w.tx, "Unused", "Groceries")
                .unwrap_err()
                .to_string()
                .starts_with("Cannot consolidate")
        );
    }
}
