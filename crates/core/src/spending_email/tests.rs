use std::cell::RefCell;

use serde_json::Value;

use super::*;

/// A made-up risk (shaped like a real day, with every name replaced) and
/// what the Python code rendered from it: `python_golden.json`.
const GOLDEN: &str = include_str!("python_golden.json");

fn golden() -> (Value, Risk) {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let risk: Risk = serde_json::from_value(golden["risk"].clone()).unwrap();
    (golden, risk)
}

#[test]
fn digest_content_matches_python() {
    let (golden, risk) = golden();
    assert_eq!(factual_summary(&risk), golden["factual_summary"]);
    assert_eq!(
        forecast_summary(&risk.forecasts, true),
        golden["forecast_summary"]
    );
    let digest = digest_message(&risk.forecasts, false);
    assert_eq!(digest.subject, "Rafael: your spending forecast 🔮");
    assert!(digest.body.starts_with(&format!(
        "{DIGEST_INTRO}\n\nNext 7 days (EUR): habits EUR 368.17"
    )));
    assert!(digest.body.chars().count() <= 1800);
}

struct FakeWriter(std::result::Result<&'static str, ()>);

impl Writer for FakeWriter {
    fn write(&self, _: &Risk) -> std::result::Result<Copy, String> {
        self.0
            .map_err(|()| "down".to_owned())
            .and_then(parse_writer_reply)
    }
}

#[derive(Default)]
struct FakeSender(RefCell<Vec<Message>>);

impl Sender for FakeSender {
    fn send(&self, message: &Message) -> std::result::Result<(), String> {
        self.0.borrow_mut().push(message.clone());
        Ok(())
    }
}

#[test]
fn alert_uses_model_copy_then_every_figure() {
    let (_, risk) = golden();
    let settings = EmailSettings::default();
    let writer = FakeWriter(Ok(
        r#"{"subject":"Rafael: easy does it","body":"Hey, slow down a little. Check your plan first."}"#,
    ));
    let message = compose(
        Plan::Alert {
            severity: Decimal::ONE,
        },
        Some(&risk),
        &risk.forecasts,
        &settings,
        &writer,
        false,
    )
    .unwrap();
    assert_eq!(message.subject, "Rafael: easy does it");
    assert!(message.body.starts_with(
        "Hey, slow down a little. Check your plan first.\n\n2026-10-09–2026-10-11: EUR 274.99"
    ));
    assert!(
        message
            .html_body
            .unwrap()
            .contains("Hey, slow down a little.")
    );

    // A reply with figures (or the model being down) falls back to fixed copy.
    for writer in [
        FakeWriter(Ok(r#"{"subject":"Spent 5","body":"x"}"#)),
        FakeWriter(Err(())),
    ] {
        let m = compose(
            Plan::Alert {
                severity: Decimal::ONE,
            },
            Some(&risk),
            &risk.forecasts,
            &settings,
            &writer,
            false,
        )
        .unwrap();
        assert_eq!(m.subject, Copy::fallback().subject);
    }
    // With the model off, the writer is never asked.
    let off = EmailSettings {
        ollama: false,
        ..EmailSettings::default()
    };
    let m = compose(
        Plan::Alert {
            severity: Decimal::ONE,
        },
        Some(&risk),
        &[],
        &off,
        &FakeWriter(Ok(r#"{"subject":"Model","body":"Model."}"#)),
        false,
    )
    .unwrap();
    assert_eq!(m.subject, Copy::fallback().subject);
}

#[test]
fn writer_reply_is_checked_like_python() {
    assert!(parse_writer_reply(r#"{"subject":"Hi","body":"Check your budget."}"#).is_ok());
    assert!(parse_writer_reply(r#"{"subject":"Hi","body":"Spend under 20."}"#).is_err());
    assert!(parse_writer_reply(r#"{"subject":"Hi","body":"ok","extra":1}"#).is_err());
    assert!(parse_writer_reply(r#"{"subject":"","body":"ok"}"#).is_err());
    assert!(parse_writer_reply("not json").is_err());
    let long = vec!["word"; 61].join(" ");
    assert!(parse_writer_reply(&format!(r#"{{"subject":"Hi","body":"{long}"}}"#)).is_err());
    let prompt = writer_prompt(&golden().1);
    assert_eq!(
        prompt,
        r#"{"recent_spending_above_plan":true,"spending_accelerating":true,"unusual_unplanned_spending":false,"budget_group_at_risk":true,"salary_budget_at_risk":true,"next_month_shortfall":true}"#
    );
}

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

#[test]
fn plan_sends_once_a_day_and_repeats_on_worsening() {
    let (_, risk) = golden();
    let settings = EmailSettings::default();
    let now = at("2026-10-26T19:30:00Z");
    let receipt = |sent: &str, severity: Option<&str>| Receipt {
        sent_at: at(sent),
        severity: severity.map(|s| s.parse().unwrap()),
    };
    let severity = severity(Some(&risk));
    assert_eq!(
        plan(Some(&risk), true, &settings, None, None, now),
        Plan::Alert { severity }
    );
    // Already sent today (Amsterdam): the digest is held back too.
    let today = receipt("2026-10-26T06:00:00Z", Some("1"));
    assert_eq!(
        plan(Some(&risk), true, &settings, Some(&today), None, now),
        Plan::Quiet
    );
    // Yesterday, much milder: send again.
    let mild = receipt("2026-10-25T19:00:00Z", Some("10"));
    assert!(matches!(
        plan(Some(&risk), true, &settings, Some(&mild), None, now),
        Plan::Alert { .. }
    ));
    // Yesterday, just as bad: the digest instead.
    let same = receipt("2026-10-25T19:00:00Z", Some(&severity.to_string()));
    assert_eq!(
        plan(Some(&risk), true, &settings, Some(&same), None, now),
        Plan::Digest
    );
    // A week later: again.
    let week = receipt("2026-10-19T19:30:00Z", Some(&severity.to_string()));
    assert!(matches!(
        plan(Some(&risk), true, &settings, Some(&week), None, now),
        Plan::Alert { .. }
    ));
    // A good day: the digest once a day, only with forecasts, only when on.
    assert_eq!(plan(None, true, &settings, None, None, now), Plan::Digest);
    assert_eq!(plan(None, false, &settings, None, None, now), Plan::Quiet);
    let digest_today = receipt("2026-10-25T23:30:00Z", None);
    assert_eq!(
        plan(None, true, &settings, None, Some(&digest_today), now),
        Plan::Quiet
    );
    let off = EmailSettings {
        daily_digest: false,
        ..EmailSettings::default()
    };
    assert_eq!(plan(None, true, &off, None, None, now), Plan::Quiet);
}

#[test]
fn schedule_is_due_once_per_local_slot() {
    let zone = TimeZone::get("Europe/Amsterdam").unwrap();
    // 20:00 in Amsterdam is 19:00 UTC in winter time.
    assert_eq!(
        latest_due(at("2026-10-26T19:00:00Z"), &zone, 20),
        at("2026-10-26T19:00:00Z")
    );
    assert_eq!(
        latest_due(at("2026-10-26T18:59:59Z"), &zone, 20),
        at("2026-10-25T19:00:00Z")
    );
    // The DST change: Saturday's slot was 18:00 UTC.
    assert_eq!(
        latest_due(at("2026-10-25T10:00:00Z"), &zone, 20),
        at("2026-10-24T18:00:00Z")
    );

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../../storage/src/notify.sql"))
        .unwrap();
    let due = at("2026-10-26T19:00:00Z");
    assert!(!claim(&conn, JOB, at("2026-10-26T18:00:00Z"), due).unwrap());
    assert!(claim(&conn, JOB, at("2026-10-26T19:05:00Z"), due).unwrap());
    // A run in progress (or one that failed) holds the slot for 15 minutes.
    assert!(!claim(&conn, JOB, at("2026-10-26T19:10:00Z"), due).unwrap());
    assert!(claim(&conn, JOB, at("2026-10-26T19:20:00Z"), due).unwrap());
    complete(&conn, JOB, at("2026-10-26T19:21:00Z")).unwrap();
    // Done for today, also after a restart; tomorrow's slot is new.
    assert!(!claim(&conn, JOB, at("2026-10-26T22:00:00Z"), due).unwrap());
    let tomorrow = latest_due(at("2026-10-27T19:30:00Z"), &zone, 20);
    assert!(claim(&conn, JOB, at("2026-10-27T19:30:00Z"), tomorrow).unwrap());
    // The row Python wrote stays readable.
    let stored: String = conn
        .query_row("SELECT lease_until FROM evaluation_schedule", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, "2026-10-27T19:45:00+00:00");
}

#[test]
fn receipts_round_trip() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../../storage/src/notify.sql"))
        .unwrap();
    assert_eq!(receipt(&conn, ReceiptKind::Alert).unwrap(), None);
    save_receipt(
        &conn,
        ReceiptKind::Alert,
        at("2026-10-26T19:00:00.5Z"),
        Some("12.30".parse().unwrap()),
    )
    .unwrap();
    let r = receipt(&conn, ReceiptKind::Alert).unwrap().unwrap();
    assert_eq!(r.sent_at, at("2026-10-26T19:00:00.5Z"));
    assert_eq!(r.severity.map(decimal_str).as_deref(), Some("12.30"));
    assert!(sent_today(Some(&r), at("2026-10-26T22:59:00Z")));
    assert!(!sent_today(Some(&r), at("2026-10-26T23:00:00Z")));
}

#[test]
fn settings_are_checked_like_the_environment() {
    let mut s = EmailSettings::default();
    assert_eq!(validate(s.clone()).unwrap(), s);
    s.enabled = true;
    assert!(validate(s.clone()).is_err());
    s.recipient = " me@example.com ".into();
    s.username = "me@example.com".into();
    let ok = validate(s.clone()).unwrap();
    assert_eq!(ok.recipient, "me@example.com");
    assert_eq!(Smtp::from_settings(&ok).from, "me@example.com");
    s.sender = "not an address".into();
    assert!(validate(s.clone()).is_err());
    s.sender = String::new();
    s.hour = 24;
    assert!(validate(s).is_err());
    // Alert emails alone need a complete mail setup too.
    let alerts_only = EmailSettings {
        alerts: true,
        ..EmailSettings::default()
    };
    assert!(validate(alerts_only).is_err());
    assert!(!is_email("a@b"));
    assert!(!is_email("a b@c.d"));
    assert!(is_email("a@b.c"));
}

/// A database file with the write tests' tiny ledger, for the service.
fn service(name: &str) -> (Service, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("finance-email-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("finance.sqlite3");
    let store = finance_storage::Store::open(&path).unwrap();
    store
        .connection()
        .execute_batch(include_str!("../writes/test_fixture.sql"))
        .unwrap();
    drop(store);
    (Service::new(path), dir)
}

#[test]
fn runner_keeps_its_slot_and_never_sends_when_off() {
    let (service, dir) = service("runner");
    let sender = FakeSender::default();
    let writer = FakeWriter(Err(()));
    let now = at("2026-10-26T19:30:00Z");
    // Off by default: nothing happens, nothing is claimed.
    assert_eq!(
        service.run_spending_email(now, &writer, &sender).unwrap(),
        RunOutcome::NotDue
    );
    service
        .save_email_settings(EmailSettings {
            enabled: true,
            recipient: "me@example.com".into(),
            username: "me@example.com".into(),
            ..EmailSettings::default()
        })
        .unwrap();
    // Before today's slot (20:00 Amsterdam = 19:00 UTC) yesterday's missed
    // slot is caught up. The fixture ledger was never refreshed recently,
    // so the run is skipped like Python's, and the slot counts as done.
    assert_eq!(
        service
            .run_spending_email(at("2026-10-26T18:30:00Z"), &writer, &sender)
            .unwrap(),
        RunOutcome::Stale
    );
    assert_eq!(
        service
            .run_spending_email(at("2026-10-26T18:45:00Z"), &writer, &sender)
            .unwrap(),
        RunOutcome::NotDue
    );
    assert_eq!(
        service.run_spending_email(now, &writer, &sender).unwrap(),
        RunOutcome::Stale
    );
    // Also for a restarted app.
    let again = Service::new(dir.join("finance.sqlite3"));
    assert_eq!(
        again
            .run_spending_email(at("2026-10-26T21:00:00Z"), &writer, &sender)
            .unwrap(),
        RunOutcome::NotDue
    );
    assert!(sender.0.borrow().is_empty());
    let status = again.email_status(false).unwrap();
    assert_eq!(
        status.last_run.as_deref(),
        Some("2026-10-26T19:30:00+00:00")
    );
    assert!(status.settings.enabled);
    let _ = std::fs::remove_dir_all(dir);
}
