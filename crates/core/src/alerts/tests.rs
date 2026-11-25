use std::cell::RefCell;

use finance_types::report::UnitMoney;
use serde_json::Value;

use super::*;

/// Python's rows for the same goals, lifecycle and moments
/// (`python_golden.json`, recorded from `AlertEvaluationService`).
const GOLDEN: &str = include_str!("python_golden.json");

struct Recorder {
    deliver: bool,
    sent: RefCell<Vec<Value>>,
}

impl Notifier for Recorder {
    fn send(&self, m: &Notification) -> Outcome {
        if !self.deliver {
            return Outcome::Disabled;
        }
        self.sent
            .borrow_mut()
            .push(Value::from(vec![m.title.clone(), m.body.clone()]));
        Outcome::Delivered
    }
}

fn database(lifecycle: &[Value]) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../../storage/src/notify.sql"))
        .unwrap();
    for row in lifecycle {
        let cell = |i: usize| row[i].clone();
        conn.execute(
            "INSERT INTO alert_lifecycle VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                cell(0).as_str(),
                cell(1).as_str(),
                cell(2).as_i64(),
                cell(3).as_str(),
                cell(4).as_str(),
                cell(5).as_str(),
                cell(6).as_str()
            ],
        )
        .unwrap();
    }
    conn
}

fn rows(conn: &Connection, sql: &str) -> Value {
    let mut stmt = conn.prepare(sql).unwrap();
    let width = stmt.column_count();
    let out: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(Value::from(
                (0..width)
                    .map(|i| match r.get_ref(i).unwrap() {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(n) => Value::from(n),
                        rusqlite::types::ValueRef::Text(t) => {
                            Value::from(String::from_utf8_lossy(t).into_owned())
                        }
                        _ => unreachable!(),
                    })
                    .collect::<Vec<_>>(),
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    Value::from(out)
}

fn settings() -> OwnerSettings {
    serde_json::from_str(r#"{"timezone":"Europe/Amsterdam","heads_up_cooldown_hours":168,"warning_cooldown_hours":72,"critical_cooldown_hours":24}"#).unwrap()
}

#[test]
fn lifecycle_rows_match_python() {
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let goals: Vec<SavingsGoal> = serde_json::from_value(golden["goals"].clone()).unwrap();
    let settings = settings();
    let policy = AlertPolicy::from_settings(&settings);
    for run in golden["runs"].as_array().unwrap() {
        let mut conn = database(golden["lifecycle"].as_array().unwrap());
        let recorder = Recorder {
            deliver: run["mode"] == "delivered",
            sent: RefCell::new(Vec::new()),
        };
        let mut decisions = Vec::new();
        for moment in run["moments"].as_array().unwrap() {
            let now: Timestamp = moment.as_str().unwrap().parse().unwrap();
            let found = candidates(&goals, &[], &settings, now).unwrap();
            for d in dispatch(&mut conn, &found, &policy, &recorder).unwrap() {
                decisions.push(Value::from(vec![
                    Value::from(d.candidate.subject_id.clone()),
                    Value::from(d.disposition.as_str()),
                    Value::from(d.notify),
                ]));
            }
        }
        let expected = &run["expected"];
        assert_eq!(Value::from(decisions), expected["decisions"]);
        assert_eq!(Value::from(recorder.sent.into_inner()), expected["sent"]);
        assert_eq!(
            rows(
                &conn,
                "SELECT * FROM alert_lifecycle ORDER BY subject_kind, subject_id"
            ),
            expected["lifecycle"]
        );
        assert_eq!(
            rows(
                &conn,
                "SELECT subject_kind, subject_id, severity, input_hash, fingerprint, \
                 policy_version, evidence_json, reasons_json, disposition, occurred_at \
                 FROM alert_events ORDER BY event_id"
            ),
            expected["events"]
        );
        assert_eq!(
            rows(
                &conn,
                "SELECT subject_kind, subject_id, fingerprint, outcome, attempted_at \
                 FROM notification_attempts ORDER BY attempt_id"
            ),
            expected["attempts"]
        );
    }
}

fn window(spent: &str, cash: &str, elapsed: i64) -> BudgetWindow {
    let d = |s: &str| s.parse::<Decimal>().unwrap();
    BudgetWindow {
        category: "Groceries".into(),
        currency: Currency::eur(),
        budget: d("300.00"),
        spent: d(spent),
        available_cash: d(cash),
        upcoming_committed: d("100.00"),
        planned_savings: d("50.00"),
        minimum_buffer: d("300"),
        elapsed_days: elapsed,
        total_days: 30,
    }
}

#[test]
fn budget_candidates_match_python() {
    let policy = AlertPolicy::from_settings(&settings());
    let at: Timestamp = "2026-10-15T08:00:00Z".parse().unwrap();
    // (spent, available cash, elapsed days) → Python's severity, evidence
    // and fingerprint for the same window.
    let cases = [
        (
            ("50.00", "2000.00", 15),
            AlertSeverity::None,
            r#"{"category":"Groceries","currency":{"code":"EUR","minor_unit":2},"elapsed_ratio":"0.5","forecast_spend":"100.0","pace_ratio":"0.3333333333333333333333333334","policy_version":"2026-09-v1","reasons":[],"remaining_budget":"250.00","safe_to_spend":"1550.00","severity":"none","spent_ratio":"0.1666666666666666666666666667"}"#,
            "1e10725323e0299c620a0dde9a6b32c042749cf4f20e36dc2c3307e39eb12b8f",
        ),
        (
            ("200.00", "2000.00", 20),
            AlertSeverity::HeadsUp,
            r#"{"category":"Groceries","currency":{"code":"EUR","minor_unit":2},"elapsed_ratio":"0.6666666666666666666666666667","forecast_spend":"300.0000000000000000000000000","pace_ratio":"1","policy_version":"2026-09-v1","reasons":["spending is ahead of the configured budget pace"],"remaining_budget":"100.00","safe_to_spend":"1550.00","severity":"heads_up","spent_ratio":"0.6666666666666666666666666667"}"#,
            "916f1c31c0358df1677949c5fca4f20d2bf3c4ce8299e1ae7fd380e87e2ac311",
        ),
        (
            ("200.00", "2000.00", 15),
            AlertSeverity::Warning,
            r#"{"category":"Groceries","currency":{"code":"EUR","minor_unit":2},"elapsed_ratio":"0.5","forecast_spend":"400.0","pace_ratio":"1.333333333333333333333333333","policy_version":"2026-09-v1","reasons":["forecast spend exceeds the category budget"],"remaining_budget":"100.00","safe_to_spend":"1550.00","severity":"warning","spent_ratio":"0.6666666666666666666666666667"}"#,
            "7df08c951cf41f98bfe446a2575c32ffc937da617e154927db8cd08cf53d6cba",
        ),
        (
            ("300.00", "400.00", 15),
            AlertSeverity::Critical,
            r#"{"category":"Groceries","currency":{"code":"EUR","minor_unit":2},"elapsed_ratio":"0.5","forecast_spend":"600.0","pace_ratio":"2","policy_version":"2026-09-v1","reasons":["available cash does not cover commitments, savings, and buffer","category budget is exhausted"],"remaining_budget":"0.00","safe_to_spend":"0","severity":"critical","spent_ratio":"1"}"#,
            "680e4a3a9328d1851156f4e3ddb0bbeceb6e9ac7885cdbdce07aafc6d938736d",
        ),
    ];
    for ((spent, cash, elapsed), severity, evidence, fingerprint) in cases {
        let c = budget_candidate(&window(spent, cash, elapsed), at, &policy);
        assert_eq!(c.severity, severity);
        assert_eq!(c.evidence_json, evidence);
        assert_eq!(c.fingerprint, fingerprint);
    }
    let calm = budget_candidate(&window("50.00", "2000.00", 15), at, &policy);
    assert_eq!(calm.reasons, ["budget is within configured limits"]);
}

#[test]
fn disabled_outcome_retries_after_an_hour() {
    let mut conn = database(&[]);
    let policy = AlertPolicy::from_settings(&settings());
    let goal = SavingsGoal {
        goal_id: "g".into(),
        name: "G".into(),
        target: UnitMoney {
            amount: Decimal::from(100),
            currency: Currency::eur(),
        },
        current: UnitMoney {
            amount: Decimal::ZERO,
            currency: Currency::eur(),
        },
        started_on: "2026-01-01".into(),
        target_date: "2026-02-01".into(),
    };
    let at = |s: &str| s.parse::<Timestamp>().unwrap();
    let run = |conn: &mut Connection, now: Timestamp| {
        let found = candidates(std::slice::from_ref(&goal), &[], &settings(), now).unwrap();
        dispatch(conn, &found, &policy, &Disabled).unwrap()[0].disposition
    };
    assert_eq!(
        run(&mut conn, at("2026-03-01T10:00:00Z")),
        Disposition::Dispatch
    );
    assert_eq!(
        run(&mut conn, at("2026-03-01T10:59:59Z")),
        Disposition::Suppressed
    );
    assert_eq!(
        run(&mut conn, at("2026-03-01T11:00:00Z")),
        Disposition::Dispatch
    );
    let outcomes: Vec<String> = conn
        .prepare("SELECT outcome FROM notification_attempts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(outcomes, ["disabled", "disabled"]);
}

#[test]
fn a_budget_line_alerts_once_at_eighty_percent_and_again_when_over() {
    use crate::budget::{BudgetInputs, default_budget, monthly_budget};
    use std::collections::HashSet;
    let now = crate::budget::tests::now();
    let plan = default_budget(jiff::civil::date(2026, 9, 1), &Currency::eur());
    let mappings = vec![finance_types::gateway::GroupMapping {
        category: "Jumbo".into(),
        group: Some("Groceries".into()),
        revision: 1,
    }];
    let fixed = HashSet::new();
    let build = |spent: &[&str]| {
        let rows: Vec<crate::model::Tx> = spent
            .iter()
            .enumerate()
            .map(|(i, a)| crate::budget::tests::tx(&format!("t{i}"), a))
            .collect();
        vec![monthly_budget(
            &plan,
            &crate::budget::tests::empty_subscriptions(),
            None,
            &BudgetInputs {
                transactions: &rows,
                mappings: &mappings,
                now: &now,
                fixed_ids: &fixed,
                payments: &[],
                resolver: None,
                current_month: None,
                events: &[],
            },
        )]
    };
    let policy = AlertPolicy::from_settings(&settings());
    let mut conn = database(&[]);
    let recorder = Recorder {
        deliver: true,
        sent: RefCell::new(Vec::new()),
    };
    let at = |s: &str| s.parse::<Timestamp>().unwrap();
    let mut run = |spent: &[&str], when: &str| {
        let found = budget_line_candidates(&build(spent), "2026-09-01", at(when), &policy);
        let groceries: Vec<Candidate> = found
            .into_iter()
            .filter(|c| c.subject_id.ends_with(":groceries"))
            .collect();
        dispatch(&mut conn, &groceries, &policy, &recorder).unwrap()[0].disposition
    };
    // Groceries plan €300: €200 is under 80%, €250 crosses it, €260 is the
    // same level, €310 is over.
    assert_eq!(run(&["200"], "2026-09-10T08:00:00Z"), Disposition::Inactive);
    assert_eq!(run(&["250"], "2026-09-10T09:00:00Z"), Disposition::Dispatch);
    assert_eq!(
        run(&["260"], "2026-09-10T10:00:00Z"),
        Disposition::Duplicate
    );
    assert_eq!(
        run(&["310"], "2026-09-10T11:00:00Z"),
        Disposition::Escalated
    );
    let sent = recorder.sent.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0][0], "Groceries at 83% of budget");
    assert!(
        sent[0][1]
            .as_str()
            .unwrap()
            .contains("€50.00 left this month")
    );
    assert_eq!(sent[1][0], "Groceries is over budget");
}
