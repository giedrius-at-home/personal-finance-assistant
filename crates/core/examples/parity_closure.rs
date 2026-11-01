//! Development check for month closure against `golden_closure.py`.
//!
//! usage:
//!   parity_closure view DB [NOW]      the closure views of 2026-09-01 and
//!                                     2026-10-01 and the unclassified
//!                                     accounts, as JSON (pot steps are
//!                                     refused: pots are retired)
//!   parity_closure actions DB STEPS   replay the steps the Python run wrote
//!                                     on DB (a COPY: it is changed) and
//!                                     print each outcome and the rows

use std::path::PathBuf;

use finance_core::closure::{closure_view, parse_month, unclassified_accounts};
use finance_core::writes::Writer;
use finance_core::writes::closure as closure_writes;
use finance_core::{Error, Snapshot};
use finance_storage::Store;
use finance_types::report::closure::{AccountRoleDraft, ClosureAction};
use jiff::Zoned;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use serde_json::{Value, json};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, path) = (
        args.first().map(String::as_str).unwrap_or("view"),
        PathBuf::from(args.get(1).cloned().unwrap_or_default()),
    );
    match mode {
        "view" => view(&path, args.get(2).map(String::as_str)),
        _ => actions(
            &path,
            &PathBuf::from(args.get(2).cloned().unwrap_or_default()),
        ),
    }
}

fn view(path: &std::path::Path, now: Option<&str>) -> anyhow::Result<()> {
    let now: Zoned = now
        .unwrap_or("2026-10-26T21:00:00+01:00[Europe/Amsterdam]")
        .parse()?;
    let store = Store::open_read_only(path)?;
    let snap = Snapshot::load(&store)?;
    let mut out = serde_json::Map::new();
    for month in ["2026-09-01", "2026-10-01"] {
        let view = closure_view(&store, &snap, parse_month(month)?, &now, None)?;
        out.insert(format!("closure_{month}"), serde_json::to_value(view)?);
    }
    out.insert(
        "unclassified".into(),
        serde_json::to_value(unclassified_accounts(&snap))?,
    );
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn actions(path: &std::path::Path, steps: &std::path::Path) -> anyhow::Result<()> {
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(steps)?)?;
    let mut store = Store::open(path)?;
    let mut results = Vec::new();
    for step in &steps {
        let now: Zoned = step["now"]
            .as_str()
            .unwrap_or("")
            .parse::<jiff::Timestamp>()?
            .to_zoned(TimeZone::get("Europe/Amsterdam")?);
        let request = step["request"].clone();
        let kind = step["kind"].as_str().unwrap_or("");
        if kind == "sql" {
            store
                .connection()
                .execute_batch(request.as_str().unwrap_or(""))?;
            results.push(json!({"ok": true}));
            continue;
        }
        if kind == "closure"
            && let Ok(action) = serde_json::from_value::<ClosureAction>(request.clone())
        {
            // The sync step commits on its own, as the service runs it.
            let snap = Snapshot::load(&store)?;
            let w = Writer {
                tx: store.connection_mut().transaction()?,
                now: now.clone(),
            };
            match closure_writes::prepare(&w, &snap, &action) {
                Ok(()) => w.tx.commit()?,
                Err(error) => {
                    results.push(json!({"ok": false, "error": error.to_string()}));
                    continue;
                }
            }
        }
        let snap = Snapshot::load(&store)?;
        let tx = store.connection_mut().transaction()?;
        let w = Writer { tx, now };
        let outcome: Result<Value, Error> = (|| match kind {
            "pot" => Err(Error::Rejected("pots are no longer available".into())),
            "role" => {
                let draft: AccountRoleDraft = serde_json::from_value(request.clone())
                    .map_err(|_| Error::Rejected("invalid role".into()))?;
                closure_writes::save_account_role(&w, &snap, draft)?;
                Ok(json!({"ok": true}))
            }
            _ => {
                let action: ClosureAction = serde_json::from_value(request.clone())
                    .map_err(|_| Error::Rejected("invalid lines".into()))?;
                let month = closure_writes::act(&w, &snap, action)?;
                let period = finance_core::closure::load_periods(&w.tx)?
                    .into_iter()
                    .find(|p| p.month == month.to_string());
                Ok(json!({
                    "ok": true,
                    "revision": period.as_ref().map(|p| p.revision),
                    "status": period.as_ref().map(|p| p.status),
                }))
            }
        })();
        match outcome {
            Ok(value) => {
                w.tx.commit()?;
                results.push(value);
            }
            Err(error) => results.push(json!({"ok": false, "error": error.to_string()})),
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "results": results,
            "rows": rows(store.connection())?,
        }))?
    );
    Ok(())
}

/// The closure rows, like the Python dump.
fn rows(conn: &Connection) -> anyhow::Result<Value> {
    let parse = |text: String| serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    let query = |sql: &str, columns: &[&str]| -> anyhow::Result<Vec<Value>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            let mut row = serde_json::Map::new();
            for (i, name) in columns.iter().enumerate() {
                let value: Value = match r.get_ref(i)? {
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Null => Value::Null,
                    other => {
                        let text =
                            String::from_utf8_lossy(other.as_bytes().unwrap_or(b"")).into_owned();
                        if *name == "payload" {
                            parse(text)
                        } else {
                            Value::String(text)
                        }
                    }
                };
                row.insert((*name).to_owned(), value);
            }
            Ok(Value::Object(row))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    Ok(json!({
        "month_periods": query("SELECT month, revision, updated_at, payload_json FROM month_periods ORDER BY month", &["month", "revision", "updated_at", "payload"])?,
        "pots": query("SELECT pot_id, payload_json FROM pots ORDER BY rowid", &["pot_id", "payload"])?,
        "pot_entries": query("SELECT entry_id, pot_id, month, payload_json FROM pot_entries ORDER BY month, entry_id", &["entry_id", "pot_id", "month", "payload"])?,
        "account_roles": query("SELECT account_id, payload_json FROM account_roles ORDER BY account_id", &["account_id", "payload"])?,
        "planned_payments": query("SELECT plan_id, payload_json, archived_at FROM planned_payments ORDER BY plan_id", &["plan_id", "payload", "archived_at"])?,
    }))
}
