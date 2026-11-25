use finance_types::bank::{BankRun, SyncStatus};
use jiff::Timestamp;
use serde_json::json;

use super::*;
use crate::closure::tests::{account, snapshot};
use crate::model::Tx;
use crate::trends::tests::out;

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn store() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../storage/src/nudges.sql"))
        .unwrap();
    conn
}

fn entry(id: &str, amount: &str, status: &str, memo: &str) -> Entry {
    Entry::parse(
        &json!({
            "entry_reference": id,
            "transaction_amount": {"amount": amount, "currency": "EUR"},
            "booking_date": "2026-11-20",
            "credit_debit_indicator": if amount.starts_with('-') { "DBIT" } else { "CRDT" },
            "remittance_information": [memo],
        }),
        status,
    )
}

#[test]
fn sightings_are_recorded_once_and_pending_is_not_counted_twice() {
    let conn = store();
    let since = Some(at("2026-11-20T10:00:00Z"));
    let downloads = vec![(
        "uid-1".to_owned(),
        vec![
            entry("p1", "-12.50", "pending", "Thuisbezorgd"),
            entry("in", "100.00", "booked", "Salary"),
            entry(
                "b2",
                "-4.50",
                "booked",
                "BEA, Betaalpas CAFE,PAS123 NR:XYZ, 20.11.26/08:15 AMSTERDAM",
            ),
        ],
    )];
    let first = record_sightings(
        &conn,
        "abnamro",
        &downloads,
        since,
        at("2026-11-20T13:00:00Z"),
    )
    .unwrap();
    assert_eq!(first, 2, "only outflows");
    // The next sync sees the same pending entry, and its booked form.
    let later = vec![(
        "uid-1".to_owned(),
        vec![
            entry("p1", "-12.50", "pending", "Thuisbezorgd"),
            entry("b1", "-12.50", "booked", "Thuisbezorgd"),
        ],
    )];
    let again =
        record_sightings(&conn, "abnamro", &later, since, at("2026-11-20T19:00:00Z")).unwrap();
    assert_eq!(again, 0);
    let exact: Option<String> = conn
        .query_row(
            "SELECT exact_at FROM purchase_sightings WHERE entry_key = 'b:uid-1:b2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exact.as_deref(), Some("2026-11-20T08:15:00"));
    // Only the pending one needs an estimate.
    let found = sightings(&conn, at("2026-11-21T00:00:00Z")).unwrap();
    assert_eq!(
        found,
        vec![Sighting {
            since: at("2026-11-20T10:00:00Z"),
            seen_at: at("2026-11-20T13:00:00Z"),
        }]
    );
}

#[test]
fn peaks_are_the_busiest_hours_apart() {
    let mut hours = [0.0; 24];
    hours[12] = 10.0;
    hours[13] = 6.0;
    hours[18] = 8.0;
    hours[19] = 1.0;
    hours[3] = 0.5;
    // 13 is next to 12, so the second peak is 18; 3 is below a tenth.
    assert_eq!(peaks(&hours), vec![12, 18]);
    assert!(peaks(&[0.0; 24]).is_empty());
}

#[test]
fn slots_are_half_past_the_hour_after_each_peak_in_local_time() {
    let zone = TimeZone::get("Europe/Amsterdam").unwrap();
    let today = jiff::civil::date(2026, 11, 25);
    let slots = slot_times(&[20, 12], today, &zone);
    assert_eq!(
        slots,
        vec![at("2026-11-25T12:30:00Z"), at("2026-11-25T20:30:00Z")]
    );
}

fn run(status: SyncStatus, last: &str) -> BankRun {
    BankRun {
        status: Some(status),
        last_attempt: Some(last.into()),
        last_success: Some(last.into()),
        ..BankRun::default()
    }
}

#[test]
fn a_passed_slot_makes_a_bank_due_once() {
    let slots = [at("2026-11-25T12:30:00Z"), at("2026-11-25T20:30:00Z")];
    let synced_morning = run(SyncStatus::Success, "2026-11-25T07:00:00Z");
    assert!(!is_due_adaptive(
        &synced_morning,
        at("2026-11-25T12:00:00Z"),
        &slots
    ));
    assert!(is_due_adaptive(
        &synced_morning,
        at("2026-11-25T12:45:00Z"),
        &slots
    ));
    // Synced after the slot: not again until the next one.
    let synced_after = run(SyncStatus::Success, "2026-11-25T12:45:00Z");
    assert!(!is_due_adaptive(
        &synced_after,
        at("2026-11-25T14:00:00Z"),
        &slots
    ));
    assert!(is_due_adaptive(
        &synced_after,
        at("2026-11-25T20:31:00Z"),
        &slots
    ));
    // A slot long past (the server was down) is skipped.
    assert!(!is_due_adaptive(
        &synced_morning,
        at("2026-11-25T16:00:00Z"),
        &slots
    ));
    // A rate limit keeps its six-hour backoff.
    let limited = run(SyncStatus::RateLimited, "2026-11-25T11:00:00Z");
    assert!(!is_due_adaptive(
        &limited,
        at("2026-11-25T12:45:00Z"),
        &slots
    ));
}

fn memo(t: Tx, description: &str) -> Tx {
    Tx {
        description: Some(description.into()),
        ..t
    }
}

#[test]
fn the_pattern_comes_from_card_memos_and_sightings() {
    let now = at("2026-11-25T12:00:00Z");
    let mut txs = Vec::new();
    for day in 1..=20 {
        let booked = format!("2026-11-{:02}T12:00:00+01:00[Europe/Amsterdam]", day + 1);
        let stamp = format!("{day:02}.11.26");
        txs.push(memo(
            out(&format!("l{day}"), "9", Some("Restaurant"), &booked),
            &format!("BEA, Betaalpas LUNCH,PAS1 NR:X, {stamp}/12:20 AMSTERDAM"),
        ));
    }
    let snap = snapshot(vec![account("firefly:1")], txs, &[]);
    // Two evenings seen within a two-hour window.
    let seen = vec![
        Sighting {
            since: at("2026-11-20T18:00:00Z"),
            seen_at: at("2026-11-20T20:00:00Z"),
        },
        Sighting {
            since: at("2026-11-21T18:00:00Z"),
            seen_at: at("2026-11-21T20:00:00Z"),
        },
        // Too wide to say anything.
        Sighting {
            since: at("2026-11-19T00:00:00Z"),
            seen_at: at("2026-11-20T00:00:00Z"),
        },
    ];
    let timing = purchase_timing(&snap, &seen, now);
    assert_eq!(timing.exact, 20);
    assert_eq!(timing.estimated, 2);
    assert!(timing.learned);
    assert_eq!(timing.peaks[0], 12);
    assert_eq!(timing.hours[12], 200);
    // 19:00–21:00 Amsterdam: two purchases spread over two hours.
    assert_eq!(timing.hours[19] + timing.hours[20], 20);
    assert_eq!(timing.slots[0], "13:30");
    assert_eq!(timing.weekdays.iter().sum::<u32>(), 20);
    // Without history: the defaults.
    let empty = purchase_timing(&snapshot(vec![], vec![], &[]), &[], now);
    assert!(!empty.learned);
    assert_eq!(empty.peaks, DEFAULT_PEAKS.to_vec());
    assert_eq!(empty.slots, vec!["13:30", "18:30", "21:30"]);
}
