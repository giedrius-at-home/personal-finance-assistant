//! Ports of the stateful cases in Python `tests/unit/test_month_closure.py`,
//! on an in-memory database with the closure tables.

use std::collections::BTreeMap;

use finance_types::report::closure::{
    AccountRoleDraft, AllocationLine, BalanceCheck, ClosureStatus, MonthPeriod, PotDraft,
    TransferStatus,
};
use jiff::civil::date;
use rusqlite::Connection;

use super::*;
use crate::Snapshot;
use crate::closure::tests::{
    account, at, d, goal, line, period, review, role, snapshot, transfer, tx,
};
use crate::model::Kind;

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
fn pot_balance_follows_the_latest_split_and_limits_takes() {
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
    let split = || vec![line("a", "pot:general", "60"), bike.clone()];
    let now = "2026-09-29T10:00:00Z";
    write(&mut conn, now, |w| settle(w, &snap, sept(), 1, split())).unwrap();
    write(&mut conn, now, |w| settle(w, &snap, sept(), 2, split())).unwrap();
    let balances = pot_balances(&load_pot_entries(&conn, None).unwrap(), None);
    assert_eq!(balances["general"]["EUR"], d("60"));
    let plans: Vec<String> = conn
        .prepare("SELECT plan_id FROM planned_payments WHERE archived_at IS NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(plans, ["closure-2026-09-01-p"]);
    // This month's own entries are replaced, so they cannot fund a take.
    let take = vec![line("t", "pot:general", "-50"), line("s", "stay", "150")];
    let refused = write(&mut conn, now, |w| settle(w, &snap, sept(), 3, take));
    assert_eq!(error(refused), "take is larger than the pot balance");
    write(&mut conn, now, |w| {
        settle(w, &snap, sept(), 3, vec![line("s", "stay", "100")])
    })
    .unwrap();
    assert!(pot_balances(&load_pot_entries(&conn, None).unwrap(), None).is_empty());
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
fn pots_are_created_trimmed_and_general_is_never_archived() {
    let mut conn = db();
    let snap = salary_snapshot();
    let draft = |pot_id: Option<&str>, name: &str, archived: bool| PotDraft {
        pot_id: pot_id.map(str::to_owned),
        name: name.into(),
        account_id: None,
        archived,
    };
    let now = "2026-09-29T10:00:00Z";
    let saved = write(&mut conn, now, |w| {
        save_pot(w, &snap, draft(None, " Holiday ", false))
    })
    .unwrap();
    assert_eq!(saved.name, "Holiday");
    assert_eq!(saved.pot_id.len(), 32);
    let general = write(&mut conn, now, |w| {
        save_pot(w, &snap, draft(Some("general"), "x", true))
    });
    assert_eq!(error(general), "general savings cannot be archived");
    let unknown = write(&mut conn, now, |w| {
        save_pot(w, &snap, draft(Some("nope"), "x", false))
    });
    assert_eq!(error(unknown), "unknown pot");
    let listed = crate::closure::pots_view(&conn).unwrap();
    assert_eq!(listed.pots[0].pot_id, "general");
    assert_eq!(listed.pots.len(), 2);
    assert!(listed.entries.is_empty());
    let reopen_open = write(&mut conn, now, |w| reopen(w, &snap, date(2026, 8, 1), 0));
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
