//! Development check: run the alert evaluation on a database copy at fixed
//! moments and print the decisions and the new lifecycle, event and attempt
//! rows, in the shape the Python golden script prints them.
//!
//! usage: parity_alerts <db copy> <disabled|delivered> <rfc3339 moment>...

use std::cell::RefCell;

use finance_core::Snapshot;
use finance_core::alerts::{self, AlertPolicy, Notification, Notifier, Outcome};
use finance_storage::Store;
use jiff::Timestamp;
use serde_json::{Value, json};

struct Recorder {
    deliver: bool,
    sent: RefCell<Vec<Value>>,
}

impl Notifier for Recorder {
    fn send(&self, m: &Notification) -> Outcome {
        if !self.deliver {
            return Outcome::Disabled;
        }
        self.sent.borrow_mut().push(json!([m.title, m.body]));
        Outcome::Delivered
    }
}

fn rows(conn: &rusqlite::Connection, sql: &str, base: i64) -> anyhow::Result<Value> {
    let mut stmt = conn.prepare(sql)?;
    let width = stmt.column_count();
    let out = stmt
        .query_map([base], |r| {
            (0..width)
                .map(|i| {
                    Ok(match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(n) => json!(n),
                        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                        other => json!(format!("{other:?}")),
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map(Value::from)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::from(out))
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("db path");
    let deliver = args.next().as_deref() == Some("delivered");
    let moments: Vec<Timestamp> = args.map(|m| m.parse()).collect::<Result<_, _>>()?;
    let mut store = Store::open(path.as_ref())?;
    let snap = Snapshot::load(&store)?;
    let policy = AlertPolicy::from_settings(&snap.settings);
    let goals: Vec<_> = snap.goals.iter().map(|g| g.goal.clone()).collect();
    let conn = store.connection_mut();
    let base_event: i64 = conn.query_row(
        "SELECT COALESCE(MAX(event_id),0) FROM alert_events",
        [],
        |r| r.get(0),
    )?;
    let base_attempt: i64 = conn.query_row(
        "SELECT COALESCE(MAX(attempt_id),0) FROM notification_attempts",
        [],
        |r| r.get(0),
    )?;
    let recorder = Recorder {
        deliver,
        sent: RefCell::new(Vec::new()),
    };
    let mut decisions = Vec::new();
    for now in moments {
        let found = alerts::candidates(&goals, &[], &snap.settings, now)?;
        for d in alerts::dispatch(conn, &found, &policy, &recorder)? {
            decisions.push(json!([
                d.candidate.subject_id,
                d.disposition.as_str(),
                d.notify
            ]));
        }
    }
    let out = json!({
        "decisions": decisions,
        "sent": recorder.sent.into_inner(),
        "lifecycle": rows(conn, "SELECT * FROM alert_lifecycle WHERE ?1 = ?1 ORDER BY subject_kind, subject_id", 0)?,
        "events": rows(conn, "SELECT subject_kind, subject_id, severity, input_hash, fingerprint, policy_version, evidence_json, reasons_json, disposition, occurred_at FROM alert_events WHERE event_id > ?1 ORDER BY event_id", base_event)?,
        "attempts": rows(conn, "SELECT subject_kind, subject_id, fingerprint, outcome, attempted_at FROM notification_attempts WHERE attempt_id > ?1 ORDER BY attempt_id", base_attempt)?,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
