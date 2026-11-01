//! Ports of the pure cases in Python `tests/unit/test_month_closure.py`,
//! plus the fixtures the write tests share.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

use finance_types::gateway::{AccountRole, OwnerSettings, SavingsGoal, SavingsGoalRecord};
use finance_types::report::closure::{
    AccountReview, AccountRoleKind, AllocationLine, ClosureReview, ClosureStatus, LineKind,
    MonthPeriod, TransferHop, TransferStatus,
};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use rust_decimal::Decimal;

use super::transfers::{
    mark_transfer, match_transfers, own_transfers, reroute_pending, unmark_transfer,
    with_transfer_targets,
};
use super::*;
use crate::Snapshot;
use crate::model::{Account, Kind, Tx};

pub(crate) const TZ: &str = "Europe/Amsterdam";

pub(crate) fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(crate) fn zone() -> TimeZone {
    TimeZone::get(TZ).unwrap()
}

pub(crate) fn at(text: &str) -> Zoned {
    text.parse::<Timestamp>().unwrap().to_zoned(zone())
}

pub(crate) fn eur() -> Currency {
    Currency::eur()
}

pub(crate) fn account(id: &str) -> Account {
    Account {
        account_id: id.to_owned(),
        ledger_id: 0,
        name: id.to_owned(),
        balance: UnitMoney {
            amount: d("100"),
            currency: eur(),
        },
        include_net_worth: true,
        active: true,
        iban: None,
    }
}

pub(crate) fn tx(id: &str, account: &str, kind: Kind, amount: &str, when: &str) -> Tx {
    Tx {
        transaction_id: id.to_owned(),
        ledger_id: 0,
        account_id: account.to_owned(),
        counterparty_account_id: None,
        kind,
        amount: d(amount),
        currency: eur(),
        occurred_at: at(when),
        purchased_on: None,
        description: None,
        counterparty_name: None,
        group_id: None,
        category: None,
        subscription_id: None,
        source_country: None,
        destination_country: None,
        transfer_evidence: None,
    }
}

/// A move between own accounts the bank booked as one transfer.
pub(crate) fn transfer(id: &str, from: &str, to: &str, amount: &str, when: &str) -> Tx {
    Tx {
        counterparty_account_id: Some(to.to_owned()),
        ..tx(id, from, Kind::Transfer, amount, when)
    }
}

pub(crate) fn role(id: &str, role: &str, goal: Option<&str>, via: Option<&str>) -> AccountRole {
    AccountRole {
        account_id: id.to_owned(),
        role: role.to_owned(),
        goal_id: goal.map(str::to_owned),
        via_account_id: via.map(str::to_owned),
        confirmed_at: "2026-09-01T00:00:00Z".into(),
    }
}

pub(crate) fn goal(id: &str) -> SavingsGoalRecord {
    let zero = UnitMoney {
        amount: Decimal::ZERO,
        currency: eur(),
    };
    SavingsGoalRecord {
        goal: SavingsGoal {
            goal_id: id.to_owned(),
            name: "Goal".into(),
            target: UnitMoney {
                amount: d("1000"),
                currency: eur(),
            },
            current: zero.clone(),
            started_on: "2026-01-01".into(),
            target_date: "2027-01-01".into(),
        },
        priority: 1,
        protected_amount: zero,
    }
}

/// A snapshot holding only what a closure reads.
pub(crate) fn snapshot(
    accounts: Vec<Account>,
    transactions: Vec<Tx>,
    roles: &[AccountRole],
) -> Snapshot {
    Snapshot {
        settings: OwnerSettings {
            timezone: TZ.into(),
            ..serde_json::from_str("{}").unwrap()
        },
        zone: zone(),
        accounts,
        raw_transactions: transactions.clone(),
        transactions,
        goals: Vec::new(),
        bills: Vec::new(),
        details: HashMap::new(),
        group_mappings: Vec::new(),
        custom_groups: Vec::new(),
        budget_plans: Vec::new(),
        planned_payments: Vec::new(),
        archived_payments: Vec::new(),
        spending_limits: Vec::new(),
        subscription_rules: Vec::new(),
        declined_bill_matches: HashSet::new(),
        deferred_sources: HashSet::new(),
        account_roles: roles
            .iter()
            .map(|r| (r.account_id.clone(), r.clone()))
            .collect(),
        month_periods: Vec::new(),
        first_observed: HashMap::new(),
        daily_review_policy: None,
        daily_review_entries: Vec::new(),
        daily_review_baseline: HashSet::new(),
        sync_health: (None, None, false),
        last_import: None,
        category_proposals: Vec::new(),
    }
}

pub(crate) fn period(month: &str, status: ClosureStatus) -> MonthPeriod {
    MonthPeriod {
        balance_checks: BTreeMap::new(),
        month: month.into(),
        starts_at: "2026-08-31T18:00:00Z".into(),
        cutoff_at: None,
        status,
        settled_at: None,
        settled_snapshot: None,
        missing_banks: Vec::new(),
        allocations: Vec::new(),
        revision: 0,
        reviewed_through: None,
        reopened_at: None,
    }
}

pub(crate) fn line(id: &str, destination: &str, amount: &str) -> AllocationLine {
    AllocationLine {
        line_id: id.into(),
        from_account_id: "abn".into(),
        destination: destination.into(),
        amount: d(amount),
        currency: eur(),
        kind: LineKind::Split,
        transfer_status: TransferStatus::NotNeeded,
        to_account_id: None,
        matched_transaction_ids: Vec::new(),
        plan_name: None,
        plan_due_on: None,
        plan_match_category: None,
        plan_match_description: None,
        plan_id: None,
        transfer_source_account_id: None,
        transfer_hops: Vec::new(),
        manual: false,
        moved_amount: None,
    }
}

pub(crate) fn review(leftover: &str) -> ClosureReview {
    ClosureReview {
        month: "2026-09-01".into(),
        accounts: vec![AccountReview {
            account_id: "abn".into(),
            name: "ABN".into(),
            currency: eur(),
            role: AccountRoleKind::Spending,
            closing_balance: d("1000"),
            carry_in: Decimal::ZERO,
            income_planned: Decimal::ZERO,
            income_actual: Decimal::ZERO,
            fixed_planned: Decimal::ZERO,
            fixed_actual: Decimal::ZERO,
            other_spent: Decimal::ZERO,
            groups: Vec::new(),
            leftover: d(leftover),
            skipped_foreign: 0,
        }],
        unassigned_groups: Vec::new(),
        unassigned_payments: Vec::new(),
        unclassified_accounts: Vec::new(),
    }
}

fn august() -> MonthPeriod {
    MonthPeriod {
        balance_checks: BTreeMap::new(),
        cutoff_at: Some("2026-08-31T18:00:00Z".into()),
        ..period("2026-08-01", ClosureStatus::Closed)
    }
}

fn resolver(periods: &[MonthPeriod], observed: HashMap<String, Timestamp>) -> PeriodResolver {
    PeriodResolver::new(periods, observed, zone())
}

#[test]
fn cutoff_day_is_split_by_first_observation() {
    let r = resolver(&[august()], HashMap::new());
    let before: Timestamp = "2026-08-31T17:00:00Z".parse().unwrap();
    let after: Timestamp = "2026-08-31T19:00:00Z".parse().unwrap();
    assert_eq!(
        r.period_for(date(2026, 8, 31), Some(before)),
        date(2026, 8, 1)
    );
    assert_eq!(
        r.period_for(date(2026, 8, 31), Some(after)),
        date(2026, 9, 1)
    );
    assert_eq!(r.period_for(date(2026, 9, 5), None), date(2026, 9, 1));
    assert_eq!(r.period_for(date(2026, 11, 5), None), date(2026, 11, 1));
}

#[test]
fn late_cutoff_does_not_pull_next_month_bookings_back() {
    let september = MonthPeriod {
        balance_checks: BTreeMap::new(),
        // Oct 2 10:00 Amsterdam local time.
        cutoff_at: Some("2026-10-02T08:00:00Z".into()),
        ..period("2026-09-01", ClosureStatus::Closed)
    };
    let r = resolver(&[september], HashMap::new());
    assert_eq!(r.period_for(date(2026, 10, 1), None), date(2026, 10, 1));
    assert_eq!(r.period_for(date(2026, 9, 30), None), date(2026, 9, 1));
}

#[test]
fn settle_requires_every_euro_assigned_to_known_destinations() {
    let goals: HashSet<&str> = HashSet::from(["emergency"]);
    let pots = PotBalances::new();
    let check = |leftover: &str, lines: &[AllocationLine]| {
        validate_allocations(&review(leftover), lines, &goals, None, &pots)
    };
    assert!(
        check(
            "400",
            &[
                line("a", "goal:emergency", "300"),
                line("b", "buffer", "100")
            ]
        )
        .is_ok()
    );
    assert!(
        check("400", &[line("a", "stay", "300")])
            .unwrap_err()
            .contains("unassigned")
    );
    assert!(
        check("100", &[line("a", "goal:trip", "100")])
            .unwrap_err()
            .contains("goal")
    );
    assert!(
        check("-50", &[line("a", "stay", "10")])
            .unwrap_err()
            .contains("deficit")
    );
    assert_eq!(
        check("100", &[line("a", "plan", "100")]).unwrap_err(),
        "a planned purchase needs a name and a due date"
    );
}

#[test]
fn delta_mode_assigns_the_leftover_change() {
    let goals = HashSet::new();
    let pots = PotBalances::new();
    let baseline = review("100");
    let check = |lines: &[AllocationLine]| {
        validate_allocations(&review("40"), lines, &goals, Some(&baseline), &pots)
    };
    assert!(check(&[line("d", "buffer", "-60")]).is_ok());
    assert!(check(&[line("d", "buffer", "-50")]).is_err());
}

fn pending(id: &str, amount: &str) -> AllocationLine {
    AllocationLine {
        transfer_status: TransferStatus::Pending,
        to_account_id: Some("savings".into()),
        ..line(id, "goal:g", amount)
    }
}

fn paired(id: &str, amount: &str, when: &str) -> Tx {
    Tx {
        transfer_evidence: Some("paired_entries"),
        ..transfer(id, "abn", "savings", amount, when)
    }
}

#[test]
fn transfer_matching_uses_each_bank_transfer_once() {
    let settled: Timestamp = "2026-09-30T10:00:00Z".parse().unwrap();
    let lines = [pending("a", "300"), pending("b", "300")];
    let all = [
        paired("old", "300", "2026-09-01T00:00:00Z"),
        paired("old2", "300", "2026-09-01T00:00:00Z"),
        paired("debit", "300", "2026-09-30T12:00:00Z"),
        paired("credit", "300.00", "2026-09-30T12:00:00Z"),
    ];
    let refs: Vec<&Tx> = all.iter().collect();
    let matched = match_transfers(&lines, &refs, settled, &zone());
    let statuses: Vec<_> = matched.iter().map(|m| m.transfer_status).collect();
    assert_eq!(statuses, [TransferStatus::Matched, TransferStatus::Pending]);
    assert_eq!(matched[0].matched_transaction_ids, ["debit", "credit"]);

    let cross = [
        paired("d2", "300", "2026-09-30T10:00:00Z"),
        paired("c2", "300", "2026-10-01T09:00:00Z"),
    ];
    let refs: Vec<&Tx> = cross.iter().collect();
    let matched = match_transfers(&lines, &refs, settled, &zone());
    let statuses: Vec<_> = matched.iter().map(|m| m.transfer_status).collect();
    assert_eq!(statuses, [TransferStatus::Matched, TransferStatus::Pending]);
}

#[test]
fn planned_purchase_money_carries_into_its_due_month() {
    let september = MonthPeriod {
        balance_checks: BTreeMap::new(),
        allocations: vec![AllocationLine {
            plan_name: Some("Laptop".into()),
            plan_due_on: Some("2026-11-01".into()),
            ..line("p", "plan", "300")
        }],
        ..period("2026-09-01", ClosureStatus::Closed)
    };
    let periods = [september];
    assert!(carry_in_for(date(2026, 10, 1), &periods).is_empty());
    let carried = carry_in_for(date(2026, 11, 1), &periods);
    assert_eq!(carried, HashMap::from([("abn".to_owned(), d("300"))]));
    let accounts = [account("abn")];
    let refs: Vec<&Account> = accounts.iter().collect();
    let laptop = [tx(
        "laptop",
        "abn",
        Kind::Outflow,
        "300",
        "2026-11-10T00:00:00Z",
    )];
    let r = resolver(&periods, HashMap::new());
    let november = closure_review(&ReviewInputs {
        month: date(2026, 11, 1),
        plan: None,
        accounts: &refs,
        transactions: &laptop,
        resolver: &r,
        mappings: &[],
        payments: &[],
        roles: &BTreeMap::new(),
        carry_in: &carried,
        fixed_ids: &HashSet::new(),
    });
    // Paying for the purchase uses the money set aside, not November's own.
    assert_eq!(november.accounts[0].leftover, Decimal::ZERO);
    assert_eq!(november.unclassified_accounts, ["abn"]);
}

#[test]
fn account_roles_list_ignored_and_unclassified_accounts() {
    let roles = [
        role("abn", "ignored", None, None),
        role("gone", "spending", None, None),
    ];
    let accounts = [account("abn"), account("rev")];
    let refs: Vec<&Account> = accounts.iter().collect();
    let views = account_role_views(&refs, &roles);
    let listed: Vec<_> = views
        .iter()
        .map(|v| (v.account_id.as_str(), v.name.as_str(), v.role))
        .collect();
    assert_eq!(
        listed,
        [
            ("abn", "abn", AccountRoleKind::Ignored),
            ("rev", "rev", AccountRoleKind::Unclassified),
            ("gone", "gone", AccountRoleKind::Spending),
        ]
    );
}

#[test]
fn transfers_into_a_pocket_route_via_the_main_account() {
    let roles = [
        role("abn", "spending", None, None),
        role("revolut", "spending", None, None),
        role("vault", "savings", Some("g"), Some("revolut")),
    ];
    let lines = with_transfer_targets(
        vec![
            line("a", "goal:g", "300"),
            AllocationLine {
                from_account_id: "revolut".into(),
                ..line("r", "goal:g", "50")
            },
        ],
        &roles,
        &[],
    );
    let hops: Vec<_> = lines[0]
        .transfer_hops
        .iter()
        .map(|h| (h.from_account_id.as_str(), h.to_account_id.as_str()))
        .collect();
    assert_eq!(hops, [("abn", "revolut"), ("revolut", "vault")]);
    assert_eq!(lines[0].to_account_id.as_deref(), Some("vault"));
    assert!(lines[1].transfer_hops.is_empty());
    assert_eq!(lines[1].to_account_id.as_deref(), Some("vault"));

    let moves = [
        transfer("in", "abn", "revolut", "300", "2026-10-01T09:00:00Z"),
        transfer("v", "revolut", "vault", "50", "2026-10-01T09:00:00Z"),
    ];
    let refs: Vec<&Tx> = moves.iter().collect();
    let settled: Timestamp = "2026-09-30T10:00:00Z".parse().unwrap();
    let matched = match_transfers(&lines, &refs, settled, &zone());
    let hop_status: Vec<_> = matched[0]
        .transfer_hops
        .iter()
        .map(|h| h.transfer_status)
        .collect();
    assert_eq!(
        hop_status,
        [TransferStatus::Matched, TransferStatus::Pending]
    );
    let statuses: Vec<_> = matched.iter().map(|m| m.transfer_status).collect();
    assert_eq!(statuses, [TransferStatus::Pending, TransferStatus::Matched]);
}

#[test]
fn a_new_via_route_replans_only_transfers_not_made_yet() {
    let direct = with_transfer_targets(
        vec![line("a", "goal:g", "300"), line("b", "goal:g", "40")],
        &[role("vault", "savings", Some("g"), None)],
        &[],
    );
    assert!(direct.iter().all(|l| l.transfer_hops.is_empty()));
    let routed = [role("vault", "savings", Some("g"), Some("revolut"))];
    let mut started = reroute_pending(&direct, &routed);
    started[1].transfer_hops[0] = TransferHop {
        transfer_status: TransferStatus::Matched,
        matched_transaction_ids: vec!["in".into()],
        ..started[1].transfer_hops[0].clone()
    };
    let hops: Vec<_> = started[0]
        .transfer_hops
        .iter()
        .map(|h| (h.from_account_id.as_str(), h.to_account_id.as_str()))
        .collect();
    assert_eq!(hops, [("abn", "revolut"), ("revolut", "vault")]);
    let moved_back = reroute_pending(&started, &[role("vault", "savings", Some("g"), None)]);
    // Clearing the route re-plans the untouched line but keeps the half-made one.
    assert!(moved_back[0].transfer_hops.is_empty());
    assert_eq!(moved_back[1], started[1]);
}

#[test]
fn manual_mark_covers_a_hop_at_another_amount() {
    let roles = [role("vault", "savings", Some("g"), Some("revolut"))];
    let hopped = with_transfer_targets(vec![line("a", "goal:g", "500")], &roles, &[]);
    let moves = [
        transfer("topup", "abn", "revolut", "450", "2026-10-01T09:00:00Z"),
        transfer("other", "abn", "swed", "500", "2026-10-01T09:00:00Z"),
    ];
    let refs: Vec<&Tx> = moves.iter().collect();
    let settled: Timestamp = "2026-09-30T10:00:00Z".parse().unwrap();
    let candidates = own_transfers(&refs, settled, &zone());
    assert_eq!(candidates.len(), 2);
    let ids = |id: &str| vec![id.to_owned()];
    assert_eq!(
        mark_transfer(&hopped, "a", Some(0), &ids("other"), &candidates).unwrap_err(),
        "transfer is between other accounts"
    );
    assert_eq!(
        mark_transfer(&hopped, "a", None, &ids("topup"), &candidates).unwrap_err(),
        "this transfer is routed; mark each hop"
    );
    let marked = mark_transfer(&hopped, "a", Some(0), &ids("topup"), &candidates).unwrap();
    let (first, second) = (&marked[0].transfer_hops[0], &marked[0].transfer_hops[1]);
    assert_eq!(first.transfer_status, TransferStatus::Matched);
    assert!(first.manual);
    assert_eq!(first.moved_amount, Some(d("450")));
    assert_eq!(second.transfer_status, TransferStatus::Pending);
    assert_eq!(marked[0].transfer_status, TransferStatus::Pending);
    let cleared = unmark_transfer(&marked, "a", Some(0)).unwrap();
    assert_eq!(cleared, hopped);
    assert_eq!(
        unmark_transfer(&cleared, "a", Some(0)).unwrap_err(),
        "only a transfer marked by hand can be unmarked"
    );
}

fn payment(
    plan_id: &str,
    direction: finance_types::gateway::Direction,
    amount: &str,
    due: &str,
) -> PlannedPaymentView {
    PlannedPaymentView {
        plan_id: plan_id.into(),
        name: plan_id.into(),
        match_category: "Other".into(),
        direction,
        amount: UnitMoney {
            amount: d(amount),
            currency: eur(),
        },
        due_on: due.into(),
        created_at: "2026-08-31T18:00:00Z".into(),
        reimbursement_for: None,
        pending: false,
        account_id: None,
        match_description: None,
        status: PlanStatus::Upcoming,
        paid_on: None,
        transaction_id: None,
        candidate_count: 0,
        candidates: Vec::new(),
    }
}

#[test]
fn suggestion_starts_from_overview_and_reserves_what_is_still_to_come() {
    use finance_types::gateway::Direction::{Inflow, Outflow};
    let allocation: CurrencyAllocation = serde_json::from_value(serde_json::json!({
        "currency": {"code": "EUR", "minor_unit": 2},
        "current_month": "2026-10-01", "funded_month": "2026-11-01",
        "bank_balance": "1681.93", "protected_savings": "0", "salary_reserved": "0",
        "salary_received_on": null, "current_month_funds": "1681.93",
        "current_fixed_remaining": "301.23", "current_plans_remaining": "0",
        "current_groups_remaining": "384.32", "current_wiggle_room": "996.38",
        "current_expected_income": "500", "current_planned_wiggle_room": "1496.38",
        "salary_at_risk": "0", "next_fixed": "0", "next_goals": "0", "next_plans": "0",
        "next_groups": "0", "next_wiggle_room": "0", "next_shortfall": "0", "groups": []
    }))
    .unwrap();
    let payments = [
        payment("laptop", Outflow, "900", "2026-11-20"),
        payment("insurance", Outflow, "300", "2027-01-31"),
        // This month's plan, beyond the horizon, recurring, a closure plan
        // or income: not reserved again.
        payment("rent", Outflow, "100", "2026-10-03"),
        payment("trip", Outflow, "700", "2027-02-01"),
        payment("recurring-salary", Outflow, "50", "2026-12-01"),
        payment("closure-2026-09-01-bike", Outflow, "999", "2026-12-09"),
        payment("deposit", Inflow, "400", "2026-12-01"),
    ];
    let forecast = EverydayForecast {
        model: "timesfm".into(),
        remaining: d("800"),
        excluded: None,
    };
    let found = savings_suggestion(
        &allocation,
        date(2026, 10, 2),
        &payments,
        d("300"),
        Decimal::ZERO,
        Some(forecast),
    );
    assert_eq!(
        found.projected_available,
        found.bank_balance - found.bills_remaining - found.plans_remaining - found.groups_remaining
            + found.expected_income
    );
    assert_eq!(found.everyday_remaining, d("800"));
    let known: Vec<_> = found
        .known_payments
        .iter()
        .map(|k| (k.name.as_str(), k.amount))
        .collect();
    assert_eq!(known, [("laptop", d("900")), ("insurance", d("300"))]);
    assert_eq!(found.known_future_payments, d("1200"));
    let unforecast = savings_suggestion(
        &allocation,
        date(2026, 10, 2),
        &[],
        d("300"),
        Decimal::ZERO,
        None,
    );
    assert!(!unforecast.forecast_available);
    assert_eq!(unforecast.everyday_remaining, Decimal::ZERO);
}

#[test]
fn savings_memos_are_suspected_transfers() {
    let mut vault = tx("1", "abn", Kind::Outflow, "10", "2026-10-01T09:00:00Z");
    vault.description = Some("To Spaarrekening".into());
    let mut coffee = tx("2", "abn", Kind::Outflow, "3", "2026-10-01T09:00:00Z");
    coffee.description = Some("Coffee".into());
    let all = [vault, coffee];
    let refs: Vec<&Tx> = all.iter().collect();
    assert_eq!(
        suspected_savings_transfers(&refs, &HashSet::from(["abn"])),
        ["1"]
    );
}

#[test]
fn stored_instants_drop_zero_microseconds() {
    let whole: Timestamp = "2026-10-26T20:00:00Z".parse().unwrap();
    let fine: Timestamp = "2026-10-26T20:00:00.123456789Z".parse().unwrap();
    assert_eq!(stored_instant(whole), "2026-10-26T20:00:00Z");
    assert_eq!(stored_instant(fine), "2026-10-26T20:00:00.123456Z");
    assert!(parse_month("2026-10-02").is_err());
    assert_eq!(parse_month("2026-10-01").unwrap(), date(2026, 10, 1));
}

#[test]
fn pot_entries_name_the_month_and_skip_other_lines() {
    let september = MonthPeriod {
        balance_checks: BTreeMap::new(),
        allocations: vec![
            line("s", "stay", "5"),
            line("g", "pot:general", "60"),
            line("z", "pot:general", "0"),
        ],
        ..period("2026-09-01", ClosureStatus::Settled)
    };
    let entries = pot_entries(&september, "2026-09-30T10:00:00Z");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_id, "2026-09-01:1:g");
    assert_eq!(entries[0].note.as_deref(), Some("September 2026 closure"));
}
