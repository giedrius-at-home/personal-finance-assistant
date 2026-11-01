//! Ports of the pure cases in Python `tests/unit/test_month_closure.py`,
//! plus the fixtures the write tests share.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

use finance_types::gateway::{
    AccountRole, OwnerSettings, PotRecord, Repeat, SavingsGoal, SavingsGoalRecord, Wish, WishKind,
    WishPurchase, WishSource, WishStatus,
};
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
    let wishes = WishCaps::new();
    let check = |leftover: &str, lines: &[AllocationLine]| {
        validate_allocations(
            &review(leftover),
            lines,
            &goals,
            None,
            &wishes,
            &HashSet::new(),
        )
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
    let wishes = WishCaps::new();
    let baseline = review("100");
    let check = |lines: &[AllocationLine]| {
        validate_allocations(
            &review("40"),
            lines,
            &goals,
            Some(&baseline),
            &wishes,
            &HashSet::new(),
        )
    };
    assert!(check(&[line("d", "buffer", "-60")]).is_ok());
    assert!(check(&[line("d", "buffer", "-50")]).is_err());
}

fn wish(id: &str, amount: &str, saved: &str, parent: Option<&str>) -> Wish {
    Wish {
        wish_id: id.into(),
        parent_id: parent.map(str::to_owned),
        name: id.into(),
        amount: d(amount),
        currency: eur(),
        due_on: "2027-01-01".into(),
        source: WishSource::Card,
        saved: d(saved),
        saved_month: None,
        status: WishStatus::Planned,
        bought: None,
        kind: WishKind::OneOff,
        must: false,
        created_at: "2026-09-01T00:00:00Z".into(),
    }
}

fn usd() -> Currency {
    Currency::new("USD")
}

#[test]
fn pot_lines_and_takes_are_refused_on_a_new_split() {
    let goals = HashSet::new();
    let wishes = WishCaps::new();
    let check = |lines: &[AllocationLine]| {
        validate_allocations(&review("10"), lines, &goals, None, &wishes, &HashSet::new())
    };
    assert_eq!(
        check(&[line("p", "pot:general", "10")]).unwrap_err(),
        "pots are no longer available"
    );
    assert_eq!(
        check(&[line("s", "stay", "-10")]).unwrap_err(),
        "pots are no longer available"
    );
    // A resettle may still refuse a pot line, even a positive one.
    let baseline = review("0");
    assert_eq!(
        validate_allocations(
            &review("10"),
            &[line("p", "pot:general", "10")],
            &goals,
            Some(&baseline),
            &wishes,
            &HashSet::new()
        )
        .unwrap_err(),
        "pots are no longer available"
    );
}

#[test]
fn wish_lines_need_a_planned_one_off_wish_with_room() {
    let goals = HashSet::new();
    let bought = Wish {
        status: WishStatus::Bought,
        bought: Some(WishPurchase {
            amount: d("50"),
            on: "2026-09-10".into(),
            source: WishSource::Card,
            cards: Vec::new(),
        }),
        ..wish("bought", "50", "0", None)
    };
    let repeating = Wish {
        kind: WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: None,
        },
        ..wish("repeat", "50", "0", None)
    };
    let caps = wish_caps(&[wish("bike", "100", "90", None), bought, repeating], &[]);
    let check = |leftover: &str, lines: &[AllocationLine]| {
        validate_allocations(
            &review(leftover),
            lines,
            &goals,
            None,
            &caps,
            &HashSet::new(),
        )
    };
    assert_eq!(
        check("10", &[line("w", "wish:nope", "10")]).unwrap_err(),
        "unknown wish"
    );
    assert_eq!(
        check("10", &[line("w", "wish:bought", "10")]).unwrap_err(),
        "unknown wish"
    );
    assert_eq!(
        check("10", &[line("w", "wish:repeat", "10")]).unwrap_err(),
        "unknown wish"
    );
    assert_eq!(
        check("10", &[line("w", "wish:bike", "-5")]).unwrap_err(),
        "a wish line must be positive"
    );
    let baseline = review("100");
    assert_eq!(
        validate_allocations(
            &review("95"),
            &[line("w", "wish:bike", "-5")],
            &goals,
            Some(&baseline),
            &caps,
            &HashSet::new()
        )
        .unwrap_err(),
        "a wish line must be positive"
    );
    assert_eq!(
        check(
            "20",
            &[line("a", "wish:bike", "15"), line("b", "wish:bike", "5")]
        )
        .unwrap_err(),
        "saves more than the wish still needs"
    );
    assert!(check("10", &[line("w", "wish:bike", "10")]).is_ok());
}

#[test]
fn wish_line_currency_must_match() {
    let caps = wish_caps(&[wish("bike", "100", "0", None)], &[]);
    let usd_line = AllocationLine {
        currency: usd(),
        ..line("w", "wish:bike", "10")
    };
    assert_eq!(
        validate_allocations(
            &review("10"),
            &[usd_line],
            &HashSet::new(),
            None,
            &caps,
            &HashSet::new()
        )
        .unwrap_err(),
        "wish currency does not match"
    );
}

/// A review of `abn` (EUR, spending) plus the given other accounts.
fn review_with(leftover: &str, others: &[(&str, Currency, AccountRoleKind)]) -> ClosureReview {
    let mut review = review(leftover);
    let base = review.accounts[0].clone();
    review
        .accounts
        .extend(others.iter().map(|(id, currency, role)| AccountReview {
            account_id: (*id).into(),
            name: (*id).into(),
            currency: currency.clone(),
            role: *role,
            leftover: Decimal::ZERO,
            ..base.clone()
        }));
    review
}

fn move_review() -> ClosureReview {
    review_with(
        "10",
        &[
            ("pocket", eur(), AccountRoleKind::Spending),
            ("vault", eur(), AccountRoleKind::Savings),
            ("skip", eur(), AccountRoleKind::Ignored),
            ("new", eur(), AccountRoleKind::Unclassified),
            ("usd", usd(), AccountRoleKind::Spending),
        ],
    )
}

#[test]
fn a_move_needs_a_known_target_in_the_same_currency() {
    let goals = HashSet::new();
    let wishes = WishCaps::new();
    let review = move_review();
    let check = |lines: &[AllocationLine]| {
        validate_allocations(&review, lines, &goals, None, &wishes, &HashSet::new())
    };
    for target in ["account:nope", "account:skip", "account:new"] {
        assert_eq!(
            check(&[line("m", target, "10")]).unwrap_err(),
            "move to an unknown account"
        );
    }
    assert_eq!(
        check(&[line("m", "account:abn", "10")]).unwrap_err(),
        "cannot move money to the same account"
    );
    // Checked before the target lookup, whatever the account's standing.
    assert_eq!(
        validate_allocations(
            &review,
            &[line("m", "account:abn", "10")],
            &goals,
            None,
            &wishes,
            &HashSet::from(["abn"])
        )
        .unwrap_err(),
        "cannot move money to the same account"
    );
    assert_eq!(
        check(&[line("m", "account:usd", "10")]).unwrap_err(),
        "move currency does not match"
    );
    assert_eq!(
        check(&[line("m", "account:pocket", "-10")]).unwrap_err(),
        "a move must be positive"
    );
    assert_eq!(
        check(&[line("m", "account:pocket", "0")]).unwrap_err(),
        "a move must be positive"
    );
    let baseline = review_with("20", &[("pocket", eur(), AccountRoleKind::Spending)]);
    assert_eq!(
        validate_allocations(
            &review,
            &[line("m", "account:pocket", "-10")],
            &goals,
            Some(&baseline),
            &wishes,
            &HashSet::new()
        )
        .unwrap_err(),
        "a move must be positive"
    );
    assert!(check(&[line("m", "account:pocket", "10")]).is_ok());
    assert_eq!(
        validate_allocations(
            &review,
            &[line("m", "account:pocket", "10")],
            &goals,
            None,
            &wishes,
            &HashSet::from(["pocket"])
        )
        .unwrap_err(),
        "move to an unknown account"
    );
    assert!(
        check(&[
            line("m", "account:pocket", "6"),
            line("v", "account:vault", "4")
        ])
        .is_ok()
    );
}

#[test]
fn a_move_needs_the_same_currency() {
    let review = review_with("10", &[("usd", usd(), AccountRoleKind::Spending)]);
    assert_eq!(
        validate_allocations(
            &review,
            &[line("m", "account:usd", "10")],
            &HashSet::new(),
            None,
            &WishCaps::new(),
            &HashSet::new()
        )
        .unwrap_err(),
        "move currency does not match"
    );
}

fn moves_in_september() -> [MonthPeriod; 1] {
    [MonthPeriod {
        allocations: vec![
            line("a", "account:pocket", "300"),
            line("b", "account:vault", "200"),
        ],
        ..period("2026-09-01", ClosureStatus::Closed)
    }]
}

fn move_roles() -> [AccountRole; 3] {
    [
        role("abn", "spending", None, None),
        role("pocket", "spending", None, None),
        role("vault", "savings", None, None),
    ]
}

#[test]
fn a_move_is_the_targets_carry_in_only_when_it_spends() {
    let carried = carry_in_for(date(2026, 10, 1), &moves_in_september(), &move_roles());
    assert_eq!(carried, HashMap::from([("pocket".to_owned(), d("300"))]));
}

#[test]
fn a_buffer_stays_on_its_source_and_a_move_goes_to_its_target() {
    let periods = [MonthPeriod {
        allocations: vec![
            line("a", "account:pocket", "300"),
            line("b", "buffer", "50"),
        ],
        ..period("2026-09-01", ClosureStatus::Closed)
    }];
    let carried = carry_in_for(date(2026, 10, 1), &periods, &move_roles());
    assert_eq!(
        carried,
        HashMap::from([("pocket".to_owned(), d("300")), ("abn".to_owned(), d("50"))])
    );
}

#[test]
fn two_moves_into_one_account_add_up() {
    let periods = [MonthPeriod {
        allocations: vec![
            line("a", "account:pocket", "300"),
            line("b", "account:pocket", "120"),
        ],
        ..period("2026-09-01", ClosureStatus::Closed)
    }];
    let carried = carry_in_for(date(2026, 10, 1), &periods, &move_roles());
    assert_eq!(carried, HashMap::from([("pocket".to_owned(), d("420"))]));
}

#[test]
fn a_move_to_savings_is_not_carried_in() {
    let carried = carry_in_for(date(2026, 10, 1), &moves_in_september(), &move_roles());
    assert!(!carried.contains_key("vault"));
    assert!(!carried.contains_key("abn"));
}

#[test]
fn parent_cap_is_its_rest_and_earlier_lines_count_back() {
    let wishes = [
        wish("trip", "500", "250", None),
        wish("flight", "200", "0", Some("trip")),
    ];
    let caps = wish_caps(&wishes, &[]);
    assert_eq!(caps["trip"].room, d("50"));
    assert_eq!(caps["trip"].currency, eur());
    assert_eq!(caps["flight"].room, d("200"));
    let earlier = [line("x", "wish:trip", "30"), line("y", "buffer", "70")];
    let caps = wish_caps(&wishes, &earlier);
    assert_eq!(caps["trip"].room, d("80"));
    // Saved past the cap leaves no room rather than a negative one.
    let caps = wish_caps(&[wish("bike", "100", "120", None)], &[]);
    assert_eq!(caps["bike"].room, Decimal::ZERO);
}

#[test]
fn a_sub_wish_needs_a_planned_parent() {
    let planned = [
        wish("trip", "500", "0", None),
        wish("flight", "200", "0", Some("trip")),
    ];
    assert!(wish_caps(&planned, &[]).contains_key("flight"));
    let mut bought_parent = wish("trip", "500", "0", None);
    bought_parent.status = WishStatus::Bought;
    let wishes = [bought_parent, wish("flight", "200", "0", Some("trip"))];
    assert!(!wish_caps(&wishes, &[]).contains_key("flight"));
    // A parent that is gone leaves the sub-wish without a cap too.
    let orphan = [wish("flight", "200", "0", Some("missing"))];
    assert!(wish_caps(&orphan, &[]).is_empty());
    let options = wish_options(&wishes, &[]);
    let names: HashMap<String, String> = wish_labels(&wishes, &options).into_iter().collect();
    assert_eq!(names["flight"], "Wish (no longer planned): flight");
}

#[test]
fn wish_options_list_parents_then_sub_wishes_by_due_date() {
    let mut later = wish("bike", "1200", "300", None);
    later.due_on = "2027-06-01".into();
    later.name = "Bike".into();
    let mut trip = wish("trip", "500", "250", None);
    trip.name = "Trip".into();
    let mut flight = wish("flight", "200", "0", Some("trip"));
    flight.name = "Flight".into();
    let mut bought = wish("tv", "400", "400", None);
    bought.name = "TV".into();
    bought.status = WishStatus::Bought;
    let mut dropped = wish("sofa", "900", "0", None);
    dropped.name = "Sofa".into();
    dropped.status = WishStatus::Dropped;
    let wishes = [later, flight, bought, trip, dropped];
    let options = wish_options(&wishes, &[]);
    let labels: Vec<(&str, &str)> = options
        .iter()
        .map(|o| (o.wish_id.as_str(), o.label.as_str()))
        .collect();
    assert_eq!(
        labels,
        [
            ("trip", "Wish: Trip rest"),
            ("flight", "Wish: Flight (€0.00 of €200.00)"),
            ("bike", "Wish: Bike (€300.00 of €1,200.00)"),
        ]
    );
    assert_eq!(options[0].room, d("50"));
    assert_eq!(options[0].currency, eur());
    let names: HashMap<String, String> = wish_labels(&wishes, &options).into_iter().collect();
    assert_eq!(names["bike"], "Wish: Bike");
    assert_eq!(names["trip"], "Wish: Trip rest");
    assert_eq!(names["flight"], "Wish: Flight");
    assert_eq!(names["tv"], "Wish (bought): TV");
    assert_eq!(names["sofa"], "Wish (dropped): Sofa");
}

#[test]
fn a_fully_saved_wish_is_not_offered_but_still_has_a_cap() {
    let wishes = [
        wish("bike", "100", "100", None),
        wish("tv", "50", "0", None),
    ];
    let ids: Vec<String> = wish_options(&wishes, &[])
        .into_iter()
        .map(|o| o.wish_id)
        .collect();
    assert_eq!(ids, ["tv"]);
    assert_eq!(wish_caps(&wishes, &[])["bike"].room, Decimal::ZERO);
    // An earlier line to it gives room back, so it is offered again.
    let earlier = [line("x", "wish:bike", "30")];
    assert_eq!(wish_options(&wishes, &earlier).len(), 2);
}

#[test]
fn a_resettle_does_not_count_earlier_wish_lines_back() {
    let wishes = [wish("bike", "100", "40", None)];
    let mut settled = period("2026-10-01", ClosureStatus::Settled);
    settled.allocations = vec![line("x", "wish:bike", "40")];
    let changed = MonthPeriod {
        status: ClosureStatus::ChangedAfterClose,
        ..settled.clone()
    };
    assert_eq!(period_wish_options(&wishes, &settled)[0].room, d("100"));
    assert_eq!(period_wish_options(&wishes, &changed)[0].room, d("60"));
}

#[test]
fn legacy_pot_lines_still_route_and_match() {
    let pots = [PotRecord {
        pot_id: "general".into(),
        name: "General savings".into(),
        account_id: Some("savings".into()),
        archived: false,
        created_at: "2026-01-01T00:00:00Z".into(),
    }];
    let lines = with_transfer_targets(vec![line("p", "pot:general", "300")], &[], &pots);
    assert_eq!(lines[0].to_account_id.as_deref(), Some("savings"));
    assert_eq!(lines[0].transfer_status, TransferStatus::Pending);
    let moves = [
        paired("debit", "300", "2026-09-30T12:00:00Z"),
        paired("credit", "300", "2026-09-30T12:00:00Z"),
    ];
    let refs: Vec<&Tx> = moves.iter().collect();
    let settled: Timestamp = "2026-09-30T10:00:00Z".parse().unwrap();
    let matched = match_transfers(&lines, &refs, settled, &zone());
    assert_eq!(matched[0].transfer_status, TransferStatus::Matched);
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
        allocations: vec![AllocationLine {
            plan_name: Some("Laptop".into()),
            plan_due_on: Some("2026-11-01".into()),
            ..line("p", "plan", "300")
        }],
        ..period("2026-09-01", ClosureStatus::Closed)
    };
    let periods = [september];
    assert!(carry_in_for(date(2026, 10, 1), &periods, &[]).is_empty());
    let carried = carry_in_for(date(2026, 11, 1), &periods, &[]);
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
    let unforecast = savings_suggestion(&allocation, date(2026, 10, 2), &[], d("300"), None);
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
fn unconfirmed_names_lists_everything_not_confirmed_in_order() {
    use finance_types::report::closure::{BalanceState, BalanceStatus};
    let status = |name: &str, state| BalanceStatus {
        account_id: format!("firefly:{name}"),
        name: name.into(),
        currency: eur(),
        ledger: d("1"),
        bank: None,
        bank_reported_on: None,
        check: None,
        state,
        adjusted_by: None,
    };
    let all = [
        status("A", BalanceState::Confirmed),
        status("B", BalanceState::Stale),
        status("C", BalanceState::Unconfirmed),
    ];
    assert_eq!(unconfirmed_names(&all), ["B", "C"]);
}

#[test]
fn a_move_gets_a_pending_transfer_to_its_target() {
    let roles = [role("abn", "spending", None, None)];
    let lines = with_transfer_targets(
        vec![
            line("m", "account:vault", "100"),
            AllocationLine {
                from_account_id: "vault".into(),
                ..line("same", "account:vault", "10")
            },
            line("zero", "account:vault", "0"),
        ],
        &roles,
        &[],
    );
    assert_eq!(lines[0].to_account_id.as_deref(), Some("vault"));
    assert_eq!(lines[0].transfer_status, TransferStatus::Pending);
    assert!(lines[0].transfer_hops.is_empty());
    for stay in &lines[1..] {
        assert_eq!(stay.to_account_id, None);
        assert_eq!(stay.transfer_status, TransferStatus::NotNeeded);
    }
}

#[test]
fn a_move_routes_through_the_via_account() {
    let roles = [
        role("abn", "spending", None, None),
        role("revolut", "spending", None, None),
        role("vault", "savings", None, Some("revolut")),
    ];
    let lines = with_transfer_targets(vec![line("m", "account:vault", "100")], &roles, &[]);
    let hops: Vec<_> = lines[0]
        .transfer_hops
        .iter()
        .map(|h| (h.from_account_id.as_str(), h.to_account_id.as_str()))
        .collect();
    assert_eq!(hops, [("abn", "revolut"), ("revolut", "vault")]);
    assert_eq!(lines[0].to_account_id.as_deref(), Some("vault"));
}

#[test]
fn a_move_is_matched_by_a_bank_transfer() {
    let settled: Timestamp = "2026-09-30T10:00:00Z".parse().unwrap();
    let lines = [AllocationLine {
        transfer_status: TransferStatus::Pending,
        to_account_id: Some("revolut".into()),
        ..line("m", "account:revolut", "100")
    }];
    let bank = [Tx {
        transfer_evidence: Some("paired_entries"),
        ..transfer("t", "abn", "revolut", "100", "2026-09-30T12:00:00Z")
    }];
    let refs: Vec<&Tx> = bank.iter().collect();
    let matched = match_transfers(&lines, &refs, settled, &zone());
    assert_eq!(matched[0].transfer_status, TransferStatus::Matched);
}
