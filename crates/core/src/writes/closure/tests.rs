//! Ports of the stateful cases in Python `tests/unit/test_month_closure.py`,
//! on an in-memory database with the closure tables.

use std::collections::BTreeMap;

use finance_types::report::closure::{
    AccountRoleDraft, AllocationLine, BalanceCheck, ClosureStatus, MonthPeriod, TransferStatus,
};
use jiff::civil::date;
use rusqlite::Connection;

use super::*;
use crate::Snapshot;
use crate::closure::tests::{
    account, at, d, goal, line, period, review, role, snapshot, transfer, tx,
};
use crate::model::Kind;
use finance_types::gateway::{Wish, WishDraft, WishPurchase, WishSource, WishStatus};

const TABLES: &str = "
CREATE TABLE month_periods (month TEXT PRIMARY KEY, revision INTEGER NOT NULL,
    updated_at TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE account_roles (account_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE pots (pot_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
CREATE TABLE pot_entries (entry_id TEXT PRIMARY KEY, pot_id TEXT NOT NULL,
    month TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE planned_payments (plan_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL,
    archived_at TEXT);
";

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(TABLES).unwrap();
    conn
}

/// Run `f` in one committed write at `now`.
fn write<T>(
    conn: &mut Connection,
    now: &str,
    f: impl FnOnce(&Writer<'_>) -> Result<T>,
) -> Result<T> {
    let w = Writer {
        tx: conn.transaction().unwrap(),
        now: at(now),
    };
    let value = f(&w)?;
    w.tx.commit()?;
    Ok(value)
}

/// Store `p` as it is (revision included).
fn put(conn: &Connection, p: &MonthPeriod) {
    conn.execute(
        "INSERT OR REPLACE INTO month_periods VALUES (?1, ?2, '', ?3)",
        params![p.month, p.revision, serde_json::to_string(p).unwrap()],
    )
    .unwrap();
}

fn put_role(conn: &Connection, r: &AccountRole) {
    conn.execute(
        "INSERT INTO account_roles VALUES (?1, ?2)",
        params![r.account_id, serde_json::to_string(r).unwrap()],
    )
    .unwrap();
}

fn stored(conn: &Connection, month: &str) -> MonthPeriod {
    load_periods(conn)
        .unwrap()
        .into_iter()
        .find(|p| p.month == month)
        .unwrap()
}

fn with_periods(mut snap: Snapshot, conn: &Connection) -> Snapshot {
    snap.month_periods = load_periods(conn).unwrap();
    snap
}

/// A check that matches a snapshot account's balance of `ledger`.
fn confirmed(ledger: &str) -> BalanceCheck {
    BalanceCheck {
        balance: d(ledger),
        ledger_balance: d(ledger),
        confirmed_at: "2026-09-29T09:00:00Z".into(),
        transaction_id: None,
    }
}

fn salary_snapshot() -> Snapshot {
    snapshot(
        vec![account("abn")],
        vec![tx(
            "salary",
            "abn",
            Kind::Inflow,
            "100",
            "2026-09-10T00:00:00Z",
        )],
        &[],
    )
}

fn sept() -> Date {
    date(2026, 9, 1)
}

fn error<T: std::fmt::Debug>(result: Result<T>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn record_sync_refuses_when_a_later_month_is_closed() {
    let mut conn = db();
    put(
        &conn,
        &MonthPeriod {
            cutoff_at: Some("2026-09-30T09:00:00Z".into()),
            ..period("2026-09-01", ClosureStatus::Closed)
        },
    );
    let snap = with_periods(salary_snapshot(), &conn);
    let refused = write(&mut conn, "2026-09-30T12:00:00Z", |w| {
        sync(w, &snap, date(2026, 8, 1), 0, &[])
    });
    assert_eq!(error(refused), "a later month is already closed");
    let early = write(&mut conn, "2026-10-20T12:00:00Z", |w| {
        sync(w, &snap, date(2026, 10, 1), 0, &[])
    });
    assert_eq!(error(early), "closing opens from the 25th");
}

#[test]
fn resettle_flags_next_month_only_when_carry_in_changes() {
    for (baseline, lines, flagged) in [
        ("0", vec![], false),
        ("100", vec![line("d", "buffer", "-100")], true),
    ] {
        let mut conn = db();
        put(
            &conn,
            &MonthPeriod {
                cutoff_at: Some("2026-08-31T18:00:00Z".into()),
                settled_at: Some("2026-08-31T18:00:00Z".into()),
                settled_snapshot: Some(review(baseline)),
                revision: 1,
                ..period("2026-08-01", ClosureStatus::ChangedAfterClose)
            },
        );
        put(
            &conn,
            &MonthPeriod {
                cutoff_at: Some("2026-09-30T09:00:00Z".into()),
                revision: 1,
                ..period("2026-09-01", ClosureStatus::Closed)
            },
        );
        let snap = with_periods(salary_snapshot(), &conn);
        write(&mut conn, "2026-10-05T12:00:00Z", |w| {
            resettle(w, &snap, date(2026, 8, 1), 1, lines)
        })
        .unwrap();
        let september = stored(&conn, "2026-09-01");
        assert_eq!(
            september.status == ClosureStatus::ChangedAfterClose,
            flagged
        );
        assert_eq!(stored(&conn, "2026-08-01").status, ClosureStatus::Closed);
    }
}

#[test]
fn resplit_keeps_settle_time_so_earlier_transfers_still_match() {
    let mut conn = db();
    let roles = [
        role("abn", "spending", None, None),
        role("savings", "savings", Some("g"), None),
    ];
    roles.iter().for_each(|r| put_role(&conn, r));
    put(
        &conn,
        &MonthPeriod {
            revision: 1,
            balance_checks: ["abn", "savings"]
                .into_iter()
                .map(|id| (id.to_owned(), confirmed("100")))
                .collect(),
            ..period("2026-09-01", ClosureStatus::Settling)
        },
    );
    let mut snap = snapshot(
        vec![account("abn"), account("savings")],
        vec![tx(
            "salary",
            "abn",
            Kind::Inflow,
            "100",
            "2026-09-10T00:00:00Z",
        )],
        &roles,
    );
    snap.goals = vec![goal("g")];
    let split = || vec![line("a", "goal:g", "100")];
    write(&mut conn, "2026-09-29T10:00:00Z", |w| {
        settle(w, &snap, sept(), 1, split())
    })
    .unwrap();
    snap.transactions.push(transfer(
        "move",
        "abn",
        "savings",
        "100",
        "2026-09-29T12:00:00Z",
    ));
    write(&mut conn, "2026-09-30T10:00:00Z", |w| {
        settle(w, &snap, sept(), 2, split())
    })
    .unwrap();
    let settled = stored(&conn, "2026-09-01");
    assert_eq!(settled.settled_at.as_deref(), Some("2026-09-29T10:00:00Z"));
    write(&mut conn, "2026-09-30T10:00:00Z", |w| {
        finalize(w, &snap, sept(), 3, &[])
    })
    .unwrap();
    let closed = stored(&conn, "2026-09-01");
    let statuses: Vec<_> = closed
        .allocations
        .iter()
        .map(|l| l.transfer_status)
        .collect();
    assert_eq!(statuses, [TransferStatus::Matched]);
    assert_eq!(closed.cutoff_at.as_deref(), Some("2026-09-30T10:00:00Z"));
}

#[test]
fn reopen_only_while_the_next_month_has_not_moved_on() {
    let mut conn = db();
    let august = MonthPeriod {
        cutoff_at: Some("2026-08-31T18:00:00Z".into()),
        settled_at: Some("2026-08-31T18:00:00Z".into()),
        allocations: vec![line("b", "buffer", "50")],
        revision: 1,
        ..period("2026-08-01", ClosureStatus::Closed)
    };
    put(&conn, &august);
    put(
        &conn,
        &MonthPeriod {
            revision: 1,
            ..period("2026-09-01", ClosureStatus::Settled)
        },
    );
    let snap = with_periods(salary_snapshot(), &conn);
    let refused = write(&mut conn, "2026-09-03T12:00:00Z", |w| {
        reopen(w, &snap, date(2026, 8, 1), 1)
    });
    assert_eq!(error(refused), "next month already uses this buffer");
    put(
        &conn,
        &MonthPeriod {
            revision: 2,
            ..period("2026-09-01", ClosureStatus::Settling)
        },
    );
    write(&mut conn, "2026-09-03T12:00:00Z", |w| {
        reopen(w, &snap, date(2026, 8, 1), 1)
    })
    .unwrap();
    let reopened = stored(&conn, "2026-08-01");
    assert_eq!(reopened.status, ClosureStatus::Settled);
    assert_eq!(reopened.cutoff_at, None);
    assert_eq!(
        reopened.reopened_at.as_deref(),
        Some("2026-09-03T12:00:00Z")
    );
    assert_eq!(reopened.allocations, august.allocations);
}

#[test]
fn a_resplit_replaces_its_plans_and_refuses_pot_lines_and_takes() {
    let mut conn = db();
    put(
        &conn,
        &MonthPeriod {
            revision: 1,
            ..period("2026-09-01", ClosureStatus::Settling)
        },
    );
    let snap = salary_snapshot();
    let bike = AllocationLine {
        plan_name: Some("Bike".into()),
        plan_due_on: Some("2026-11-01".into()),
        ..line("p", "plan", "40")
    };
    let now = "2026-09-29T10:00:00Z";
    let pot = vec![line("a", "pot:general", "60"), bike.clone()];
    let refused = write(&mut conn, now, |w| settle(w, &snap, sept(), 1, pot));
    assert_eq!(error(refused), "pots are no longer available");
    let split = || vec![line("a", "buffer", "60"), bike.clone()];
    write(&mut conn, now, |w| settle(w, &snap, sept(), 1, split())).unwrap();
    write(&mut conn, now, |w| settle(w, &snap, sept(), 2, split())).unwrap();
    let plans: Vec<String> = conn
        .prepare("SELECT plan_id FROM planned_payments WHERE archived_at IS NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(plans, ["closure-2026-09-01-p"]);
    let take = vec![line("t", "pot:general", "-50"), line("s", "stay", "150")];
    let refused = write(&mut conn, now, |w| settle(w, &snap, sept(), 3, take));
    assert_eq!(error(refused), "pots are no longer available");
    write(&mut conn, now, |w| {
        settle(w, &snap, sept(), 3, vec![line("s", "stay", "100")])
    })
    .unwrap();
    let entries = crate::closure::load_pot_entries(&conn, None).unwrap();
    assert!(crate::closure::pot_balances(&entries, None).is_empty());
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM planned_payments WHERE archived_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
}

#[test]
fn reopen_refuses_an_open_month() {
    let mut conn = db();
    let snap = salary_snapshot();
    let reopen_open = write(&mut conn, "2026-09-29T10:00:00Z", |w| {
        reopen(w, &snap, date(2026, 8, 1), 0)
    });
    assert_eq!(error(reopen_open), "month is open");
}

#[test]
fn refinalize_after_reopen_adds_delta_lines_with_new_ids() {
    let mut conn = db();
    put(
        &conn,
        &MonthPeriod {
            revision: 1,
            ..period("2026-09-01", ClosureStatus::Settling)
        },
    );
    let mut snap = salary_snapshot();
    let now = "2026-09-30T10:00:00Z";
    write(&mut conn, now, |w| {
        settle(w, &snap, sept(), 1, vec![line("a", "stay", "100")])
    })
    .unwrap();
    let mut ids = Vec::new();
    for key in ["late", "later"] {
        snap.transactions
            .push(tx(key, "abn", Kind::Inflow, "10", "2026-09-10T00:00:00Z"));
        let revision = stored(&conn, "2026-09-01").revision;
        write(&mut conn, now, |w| {
            finalize(w, &snap, sept(), revision, &[])
        })
        .unwrap();
        let closed = stored(&conn, "2026-09-01");
        ids = closed
            .allocations
            .iter()
            .map(|l| l.line_id.clone())
            .collect();
        if key == "late" {
            write(&mut conn, now, |w| {
                reopen(w, &snap, sept(), closed.revision)
            })
            .unwrap();
        }
    }
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 3);
}

#[test]
fn marking_a_hop_by_hand_checks_revision_and_accounts() {
    let mut conn = db();
    let roles = [role("vault", "savings", Some("g"), Some("revolut"))];
    let hopped = crate::closure::transfers::with_transfer_targets(
        vec![line("a", "goal:g", "500")],
        &roles,
        &[],
    );
    put(
        &conn,
        &MonthPeriod {
            settled_at: Some("2026-09-30T10:00:00Z".into()),
            allocations: hopped,
            revision: 1,
            ..period("2026-09-01", ClosureStatus::Settled)
        },
    );
    let snap = snapshot(
        vec![account("abn")],
        vec![
            transfer("topup", "abn", "revolut", "450", "2026-09-30T10:00:00Z"),
            transfer("other", "abn", "swed", "500", "2026-09-30T10:00:00Z"),
        ],
        &roles,
    );
    let now = "2026-09-30T11:00:00Z";
    let ids = |id: &str| vec![id.to_owned()];
    let stale = write(&mut conn, now, |w| {
        mark(w, &snap, sept(), 0, "a", Some(0), &ids("topup"))
    });
    assert_eq!(error(stale), "month closure changed; refresh");
    let wrong = write(&mut conn, now, |w| {
        mark(w, &snap, sept(), 1, "a", Some(0), &ids("other"))
    });
    assert_eq!(error(wrong), "transfer is between other accounts");
    write(&mut conn, now, |w| {
        mark(w, &snap, sept(), 1, "a", Some(0), &ids("topup"))
    })
    .unwrap();
    let marked = stored(&conn, "2026-09-01");
    let hop = &marked.allocations[0].transfer_hops[0];
    assert!(hop.manual && hop.moved_amount == Some(d("450")));
    assert_eq!(
        marked.allocations[0].transfer_status,
        TransferStatus::Pending
    );
    let stale = write(&mut conn, now, |w| unmark(w, sept(), 1, "a", Some(0)));
    assert_eq!(error(stale), "month closure changed; refresh");
    write(&mut conn, now, |w| unmark(w, sept(), 2, "a", Some(0))).unwrap();
    let hop = &stored(&conn, "2026-09-01").allocations[0].transfer_hops[0];
    assert_eq!(hop.transfer_status, TransferStatus::Pending);
}

#[test]
fn a_via_route_is_one_hop_and_replans_open_transfers() {
    let mut conn = db();
    let roles = [
        role("abn", "spending", None, None),
        role("vault", "savings", Some("g"), None),
    ];
    roles.iter().for_each(|r| put_role(&conn, r));
    let direct = crate::closure::transfers::with_transfer_targets(
        vec![line("a", "goal:g", "300")],
        &roles,
        &[],
    );
    put(
        &conn,
        &MonthPeriod {
            settled_at: Some("2026-09-30T10:00:00Z".into()),
            allocations: direct,
            revision: 1,
            ..period("2026-09-01", ClosureStatus::Settled)
        },
    );
    let mut snap = snapshot(
        vec![account("abn"), account("revolut"), account("vault")],
        Vec::new(),
        &roles,
    );
    snap.goals = vec![goal("g")];
    let draft = |id: &str, via: Option<Option<&str>>| AccountRoleDraft {
        account_id: id.into(),
        role: "savings".into(),
        goal_id: Some("g".into()),
        via_account_id: via.map(|v| v.map(str::to_owned)),
    };
    let now = "2026-10-01T10:00:00Z";
    let to_self = write(&mut conn, now, |w| {
        save_account_role(w, &snap, draft("vault", Some(Some("vault"))))
    });
    assert_eq!(error(to_self), "invalid via account");
    let taken = write(&mut conn, now, |w| {
        save_account_role(w, &snap, draft("abn", None))
    });
    assert_eq!(
        error(taken),
        "goal already linked to another savings account"
    );
    write(&mut conn, now, |w| {
        save_account_role(w, &snap, draft("vault", Some(Some("revolut"))))
    })
    .unwrap();
    let routed = stored(&conn, "2026-09-01");
    assert_eq!(routed.revision, 2);
    assert_eq!(routed.allocations[0].transfer_hops.len(), 2);
    let chained = write(&mut conn, now, |w| {
        save_account_role(
            w,
            &snap,
            AccountRoleDraft {
                account_id: "abn".into(),
                role: "spending".into(),
                goal_id: None,
                via_account_id: Some(Some("vault".into())),
            },
        )
    });
    assert_eq!(error(chained), "via account is itself reached via another");
    // Left out, the saved route stays.
    write(&mut conn, now, |w| {
        save_account_role(w, &snap, draft("vault", None))
    })
    .unwrap();
    let kept = load_roles(&conn).unwrap();
    assert_eq!(
        kept.iter()
            .find(|r| r.account_id == "vault")
            .unwrap()
            .via_account_id
            .as_deref(),
        Some("revolut")
    );
}

#[test]
fn a_saved_period_keeps_its_stored_balance_checks() {
    let mut conn = db();
    let checks = BTreeMap::from([(
        "firefly:1".to_string(),
        BalanceCheck {
            balance: d("1234.56"),
            ledger_balance: d("1234.56"),
            confirmed_at: "2026-09-29T09:00:00Z".into(),
            transaction_id: Some("firefly:77".into()),
        },
    )]);
    put(
        &conn,
        &MonthPeriod {
            revision: 1,
            balance_checks: checks.clone(),
            ..period("2026-09-01", ClosureStatus::Settling)
        },
    );
    let snap = salary_snapshot();
    write(&mut conn, "2026-09-29T10:00:00Z", |w| {
        settle(w, &snap, sept(), 1, vec![line("s", "stay", "100")])
    })
    .unwrap();
    assert_eq!(stored(&conn, "2026-09-01").balance_checks, checks);
}

fn confirm_db(status: ClosureStatus) -> crate::writes::fixture::Db {
    let db = crate::writes::fixture::db();
    put_role(
        db.store.connection(),
        &role("firefly:1", "spending", None, None),
    );
    put(
        db.store.connection(),
        &MonthPeriod {
            revision: 1,
            ..period("2026-10-01", status)
        },
    );
    db
}

fn october() -> Date {
    date(2026, 10, 1)
}

fn reconciliations(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE kind = 'reconciliation'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn transaction_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
        .unwrap()
}

fn ledger(db: &crate::writes::fixture::Db) -> Decimal {
    crate::bank_sync::ledger_balance(db.store.connection(), 1, "EUR").unwrap()
}

#[test]
fn confirm_books_the_difference_and_a_retype_replaces_it() {
    let mut db = confirm_db(ClosureStatus::Settling);
    let base = ledger(&db);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", base + d("40")))
        .unwrap();
    let check = stored(db.store.connection(), "2026-10-01").balance_checks["firefly:1"].clone();
    assert!(check.transaction_id.is_some());
    assert_eq!(check.balance, base + d("40"));
    assert_eq!(check.ledger_balance, base + d("40"));
    assert_eq!(reconciliations(db.store.connection()), 1);
    assert_eq!(ledger(&db), base + d("40"));

    db.write(|w, snap| confirm_balance(w, snap, october(), 2, "firefly:1", base - d("10")))
        .unwrap();
    let second = stored(db.store.connection(), "2026-10-01").balance_checks["firefly:1"].clone();
    assert!(second.transaction_id.is_some());
    assert_eq!(reconciliations(db.store.connection()), 1);
    assert_eq!(ledger(&db), base - d("10"));

    db.write(|w, snap| confirm_balance(w, snap, october(), 3, "firefly:1", base))
        .unwrap();
    let third = stored(db.store.connection(), "2026-10-01");
    assert_eq!(third.balance_checks["firefly:1"].transaction_id, None);
    assert_eq!(reconciliations(db.store.connection()), 0);
    assert_eq!(ledger(&db), base);
    assert_eq!(third.revision, 4);
}

#[test]
fn confirm_refuses_more_decimals_than_the_currency() {
    let mut db = confirm_db(ClosureStatus::Settling);
    let rows = transaction_count(db.store.connection());
    let result =
        db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", d("12.345")));
    assert_eq!(error(result), "invalid balance");
    assert_eq!(transaction_count(db.store.connection()), rows);
}

#[test]
fn confirm_accepts_trailing_zeros_past_the_minor_unit() {
    let mut db = confirm_db(ClosureStatus::Settling);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", d("12.100")))
        .unwrap();
    assert_eq!(ledger(&db), d("12.10"));
}

#[test]
fn confirm_books_a_negative_balance() {
    let mut db = confirm_db(ClosureStatus::Open);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", d("-25.00")))
        .unwrap();
    assert_eq!(ledger(&db), d("-25"));
}

#[test]
fn confirm_with_a_stale_revision_books_nothing() {
    let mut db = confirm_db(ClosureStatus::Settling);
    let rows = transaction_count(db.store.connection());
    let result = db.write(|w, snap| confirm_balance(w, snap, october(), 0, "firefly:1", d("500")));
    assert_eq!(error(result), "month closure changed; refresh");
    assert_eq!(transaction_count(db.store.connection()), rows);
}

#[test]
fn confirm_refuses_closed_months_and_unchecked_accounts() {
    let mut db = confirm_db(ClosureStatus::Closed);
    let result = db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", d("5")));
    assert_eq!(error(result), "month is closed");

    let mut db = confirm_db(ClosureStatus::Settling);
    db.store
        .connection()
        .execute("DELETE FROM account_roles", [])
        .unwrap();
    put_role(
        db.store.connection(),
        &role("firefly:1", "ignored", None, None),
    );
    let result = db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", d("5")));
    assert_eq!(error(result), "account cannot be confirmed");
}

fn statuses(db: &crate::writes::fixture::Db) -> Vec<finance_types::report::closure::BalanceStatus> {
    let snap = db.snap();
    let period = stored(db.store.connection(), "2026-10-01");
    let roles = load_roles(db.store.connection()).unwrap();
    crate::closure::balance_statuses(db.store.connection(), &snap, &period, &roles).unwrap()
}

fn october_split() -> Vec<AllocationLine> {
    Vec::new()
}

#[test]
fn settle_needs_every_account_confirmed() {
    let mut db = confirm_db(ClosureStatus::Settling);
    let refused = db.write(|w, snap| settle(w, snap, october(), 1, october_split()));
    assert_eq!(
        error(refused),
        "confirm every account balance first: Checking"
    );
    let base = ledger(&db);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", base))
        .unwrap();
    db.write(|w, snap| settle(w, snap, october(), 2, october_split()))
        .unwrap();
    assert_eq!(
        stored(db.store.connection(), "2026-10-01").status,
        ClosureStatus::Settled
    );
}

#[test]
fn banks_screen_reconciliation_makes_the_check_stale() {
    use finance_types::report::closure::BalanceState;
    let mut db = confirm_db(ClosureStatus::Settling);
    let base = ledger(&db);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", base + d("40")))
        .unwrap();
    let status = &statuses(&db)[0];
    assert_eq!(status.state, BalanceState::Confirmed);
    assert_eq!(status.adjusted_by, Some(d("40")));

    db.write(|w, _| {
        crate::bank_sync::book_reconciliation(w, 1, "Checking", "EUR", base + d("50"), "banks")
    })
    .unwrap();
    assert_eq!(statuses(&db)[0].state, BalanceState::Stale);
    let refused = db.write(|w, snap| settle(w, snap, october(), 2, october_split()));
    assert_eq!(
        error(refused),
        "confirm every account balance first: Checking"
    );

    db.write(|w, snap| confirm_balance(w, snap, october(), 2, "firefly:1", base + d("50")))
        .unwrap();
    assert_eq!(statuses(&db)[0].state, BalanceState::Confirmed);
    let check = stored(db.store.connection(), "2026-10-01").balance_checks["firefly:1"].clone();
    // Confirm replaces its own earlier entry, so the banks entry stays and a new
    // one covers the difference again.
    assert!(check.transaction_id.is_some());
    assert_eq!(ledger(&db), base + d("50"));
    assert_eq!(reconciliations(db.store.connection()), 2);
}

#[test]
fn confirm_never_deletes_a_stored_id_that_is_not_its_reconciliation() {
    let mut db = confirm_db(ClosureStatus::Settling);
    let mut stale = stored(db.store.connection(), "2026-10-01");
    stale.balance_checks.insert(
        "firefly:1".into(),
        BalanceCheck {
            balance: d("1"),
            ledger_balance: d("1"),
            confirmed_at: "2026-10-28T09:00:00Z".into(),
            transaction_id: Some("firefly:10".into()),
        },
    );
    put(db.store.connection(), &stale);
    assert_eq!(statuses(&db)[0].adjusted_by, None);

    let withdrawal = |db: &crate::writes::fixture::Db| -> i64 {
        db.store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE id = 10 AND kind = 'withdrawal'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    let base = ledger(&db);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", base + d("5")))
        .unwrap();
    assert_eq!(withdrawal(&db), 1);
    assert_eq!(ledger(&db), base + d("5"));
    assert_eq!(reconciliations(db.store.connection()), 1);
}

#[test]
fn the_newest_bank_figure_and_its_date_reach_the_status() {
    use finance_types::bank::{AccountBalance, BankRun};
    let mut db = confirm_db(ClosureStatus::Settling);
    let balance = |currency: &str, bank: &str, on: Option<&str>| AccountBalance {
        account_id: 1,
        account: "Checking".into(),
        currency: currency.into(),
        bank: bank.into(),
        ledger: "0".into(),
        balance_type: "CLBD".into(),
        reference_date: on.map(Into::into),
    };
    let run = |balances| BankRun {
        balances,
        ..BankRun::default()
    };
    db.write(|w, _| {
        crate::bank_sync::save_run(
            w,
            "rabobank",
            &run(vec![
                balance("EUR", "100.00", Some("2026-10-27")),
                balance("EUR", "150.00", None),
            ]),
        )?;
        crate::bank_sync::save_run(
            w,
            "revolut",
            &run(vec![balance("EUR", "120.00", Some("2026-10-29"))]),
        )
    })
    .unwrap();
    let status = &statuses(&db)[0];
    assert_eq!(status.bank, Some(d("120.00")));
    assert_eq!(status.bank_reported_on.as_deref(), Some("2026-10-29"));

    db.write(|w, _| {
        crate::bank_sync::save_run(
            w,
            "rabobank",
            &run(vec![balance("USD", "100.00", Some("2026-10-30"))]),
        )?;
        crate::bank_sync::save_run(w, "revolut", &run(Vec::new()))
    })
    .unwrap();
    let status = &statuses(&db)[0];
    assert_eq!(status.bank, None);
    assert_eq!(status.bank_reported_on, None);
}

/// October, settling, with the checking account confirmed, and one planned
/// one-off wish of `amount` euros. A €500 salary leaves room to split.
fn wish_db(amount: &str) -> (crate::writes::fixture::Db, String) {
    let mut db = confirm_db(ClosureStatus::Settling);
    db.store
        .connection()
        .execute_batch(
            "INSERT INTO transactions (id, kind, date, booked_at, description, \
                 source_account_id, destination_account_id, category_id, tags) \
             VALUES (20, 'deposit', '2026-10-24', '2026-10-24T12:00:00', 'Salary', 3, 1, NULL, '[]');
             INSERT INTO postings (transaction_id, account_id, amount, currency) \
             VALUES (20, 3, '-500.00', 'EUR'), (20, 1, '500.00', 'EUR');",
        )
        .unwrap();
    let base = ledger(&db);
    db.write(|w, snap| confirm_balance(w, snap, october(), 1, "firefly:1", base))
        .unwrap();
    let draft = WishDraft {
        wish_id: None,
        parent_id: None,
        name: "Bike".into(),
        amount: d(amount),
        currency: crate::closure::tests::eur(),
        due_on: "2026-12-01".into(),
        source: WishSource::Card,
        kind: Default::default(),
        must: false,
    };
    let wish = db
        .write(|w, snap| crate::writes::wishes::save_wish(w, snap, draft))
        .unwrap();
    (db, wish.wish_id)
}

fn wish(db: &crate::writes::fixture::Db, wish_id: &str) -> Wish {
    load_wishes(db.store.connection())
        .unwrap()
        .into_iter()
        .find(|w| w.wish_id == wish_id)
        .unwrap()
}

/// The checking account's line toward `destination`.
fn checking(id: &str, destination: &str, amount: &str) -> AllocationLine {
    AllocationLine {
        from_account_id: "firefly:1".into(),
        ..line(id, destination, amount)
    }
}

/// October's review as it stands now.
fn october_review(db: &crate::writes::fixture::Db) -> ClosureReview {
    let periods = load_periods(db.store.connection()).unwrap();
    review_month(
        &db.snap(),
        &periods,
        october(),
        &crate::writes::fixture::now(),
    )
}

/// Settle October with `wished` toward the wish and the rest staying.
fn settle_wish(db: &mut crate::writes::fixture::Db, wish_id: &str, wished: &str) -> Result<()> {
    let leftover = october_review(db).accounts[0].leftover;
    assert!(leftover > d("100"), "leftover {leftover}");
    let mut lines = Vec::new();
    if d(wished) > Decimal::ZERO {
        lines.push(checking("w", &format!("wish:{wish_id}"), wished));
    }
    let rest = leftover - d(wished);
    if rest > Decimal::ZERO {
        lines.push(checking("s", "stay", &rest.to_string()));
    }
    let revision = stored(db.store.connection(), "2026-10-01").revision;
    db.write(|w, snap| settle(w, snap, october(), revision, lines))
}

/// Mark October changed after its close, its baseline `less` below today's leftover.
fn changed_after_close(db: &crate::writes::fixture::Db, less: &str) {
    let conn = db.store.connection();
    let mut changed = MonthPeriod {
        status: ClosureStatus::ChangedAfterClose,
        ..stored(conn, "2026-10-01")
    };
    put(conn, &changed);
    let mut baseline = october_review(db);
    baseline.accounts[0].leftover -= d(less);
    changed.settled_snapshot = Some(baseline);
    put(conn, &changed);
}

fn resettle_october(db: &mut crate::writes::fixture::Db, lines: Vec<AllocationLine>) -> Result<()> {
    let revision = stored(db.store.connection(), "2026-10-01").revision;
    db.write(|w, snap| resettle(w, snap, october(), revision, lines))
}

#[test]
fn settle_raises_the_wish_saved_and_sets_its_month() {
    let (mut db, id) = wish_db("900");
    settle_wish(&mut db, &id, "40").unwrap();
    let saved = wish(&db, &id);
    assert_eq!(saved.saved, d("40"));
    assert_eq!(saved.saved_month.as_deref(), Some("2026-10-01"));
}

#[test]
fn resplit_without_the_wish_line_gives_the_money_back() {
    let (mut db, id) = wish_db("900");
    settle_wish(&mut db, &id, "40").unwrap();
    settle_wish(&mut db, &id, "40").unwrap();
    assert_eq!(wish(&db, &id).saved, d("40"));
    settle_wish(&mut db, &id, "0").unwrap();
    assert_eq!(wish(&db, &id).saved, Decimal::ZERO);
}

#[test]
fn manual_lower_then_resplit_clamps_at_zero() {
    let (mut db, id) = wish_db("900");
    settle_wish(&mut db, &id, "40").unwrap();
    db.write(|w, snap| crate::writes::wishes::record_saved(w, snap, &id, d("10")))
        .unwrap();
    settle_wish(&mut db, &id, "40").unwrap();
    assert_eq!(wish(&db, &id).saved, d("10"));
    settle_wish(&mut db, &id, "0").unwrap();
    assert_eq!(wish(&db, &id).saved, Decimal::ZERO);
}

#[test]
fn closure_never_moves_saved_month_backwards() {
    let (mut db, id) = wish_db("900");
    let later = Wish {
        saved_month: Some("2026-11-01".into()),
        ..wish(&db, &id)
    };
    db.write(|w, _| crate::writes::wishes::store(w, &later))
        .unwrap();
    settle_wish(&mut db, &id, "40").unwrap();
    let saved = wish(&db, &id);
    assert_eq!(saved.saved, d("40"));
    assert_eq!(saved.saved_month.as_deref(), Some("2026-11-01"));
}

#[test]
fn a_bought_wish_is_skipped_but_its_line_stays() {
    let bought = || {
        let (mut db, id) = wish_db("900");
        settle_wish(&mut db, &id, "40").unwrap();
        let purchase = WishPurchase {
            amount: d("40"),
            on: "2026-10-20".into(),
            source: WishSource::Account {
                account_id: "firefly:1".into(),
            },
            cards: Vec::new(),
        };
        db.write(|w, snap| crate::writes::wishes::mark_bought(w, snap, &id, purchase, &[]))
            .unwrap();
        assert_eq!(wish(&db, &id).status, WishStatus::Bought);
        (db, id)
    };
    // A re-split without the line would give the money back; a bought wish keeps it.
    let (mut db, id) = bought();
    settle_wish(&mut db, &id, "0").unwrap();
    assert_eq!(wish(&db, &id).saved, d("40"));
    // A resettle keeps the earlier line in the period.
    let (mut db, id) = bought();
    let before = wish(&db, &id);
    changed_after_close(&db, "15");
    resettle_october(&mut db, vec![checking("d", "buffer", "15")]).unwrap();
    assert_eq!(wish(&db, &id).saved, before.saved);
    let wish_line = format!("wish:{id}");
    assert!(
        stored(db.store.connection(), "2026-10-01")
            .allocations
            .iter()
            .any(|l| l.destination == wish_line)
    );
}

#[test]
fn a_resettle_delta_line_adds_to_the_wish() {
    let (mut db, id) = wish_db("60");
    settle_wish(&mut db, &id, "40").unwrap();
    changed_after_close(&db, "15");
    let toward = format!("wish:{id}");
    resettle_october(&mut db, vec![checking("d1", &toward, "15")]).unwrap();
    assert_eq!(wish(&db, &id).saved, d("55"));
    // Earlier lines stay booked: only 60 − 55 is left, not 60 again.
    changed_after_close(&db, "10");
    let refused = resettle_october(&mut db, vec![checking("d2", &toward, "10")]);
    assert_eq!(error(refused), "saves more than the wish still needs");
    assert_eq!(wish(&db, &id).saved, d("55"));
}
