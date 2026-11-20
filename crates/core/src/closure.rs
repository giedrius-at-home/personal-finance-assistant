//! Month closure (Python `domain/month_closure.py` and the read side of
//! `closure_service.py`): the per-account review of a month, split
//! validation, savings pots, the savings suggestion and the screen's view.
//! Transfer routing and matching live in [`transfers`]; the actions that
//! change a closure live in `writes::closure`.

pub mod plan;
pub mod transfers;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use finance_storage::Store;
use finance_types::gateway::{
    AccountRole, BudgetPlan, Direction, GroupMapping, PotEntryRecord, PotRecord, Wish, WishKind,
    WishStatus,
};
use finance_types::report::Currency;
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::closure::{
    AccountReview, AccountRoleKind, AccountRoleView, AllocationLine, BalanceState, BalanceStatus,
    ClosureReview, ClosureStatus, ClosureView, GroupReview, KnownPayment, MoneyPlan, MonthPeriod,
    NamedAccount, PotView, SavingsSuggestion, TransferStatus, UnclassifiedAccounts, WishOption,
};
use finance_types::report::outlook::{DailySpendingForecast, ForecastExclusions};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use regex::Regex;
use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::budget::budget_owner;
use crate::groups::{fixed_category_names, fixed_transaction_ids};
use crate::load::{parse_instant, payloads};
use crate::model::{Account, Kind, Tx};
use crate::period::{PeriodResolver, shift_month};
use crate::plans::{CLOSURE_PLAN_PREFIX, RECURRING_PLAN_PREFIX, planned_payment_views_since};
use crate::{Ctx, Error, Result, Snapshot, dashboard};

/// The built-in pot every split can save into; it cannot be archived.
pub const GENERAL_POT_ID: &str = "general";
pub const GENERAL_POT_NAME: &str = "General savings";
/// Manual one-off payments due this many months after the current one are reserved.
pub const KNOWN_PAYMENT_MONTHS: i32 = 3;
/// Banks a closure can report as not synced (Python `bank_ids.BANKS`).
pub const BANKS: [&str; 3] = ["abnamro", "revolut", "swedbank"];
/// The category a planned purchase is matched on when the owner names none.
pub const PLANNED_PURCHASE_CATEGORY: &str = "Planned purchase";

/// `YYYY-MM-01`, or the owner sees "invalid month".
pub fn parse_month(text: &str) -> Result<Date> {
    text.parse::<Date>()
        .ok()
        .filter(|d| d.day() == 1 && text.len() == 10)
        .ok_or_else(|| Error::Rejected("invalid month".into()))
}

/// A stored `YYYY-MM-DD…` date.
pub(crate) fn day_of(text: &str) -> Option<Date> {
    text.get(..10)?.parse().ok()
}

/// An instant the way pydantic wrote `datetime.now(UTC)`: `…T20:00:00Z`, or
/// with six fractional digits when there are microseconds.
pub fn stored_instant(t: Timestamp) -> String {
    let micros = t.subsec_microsecond();
    let base = t.strftime("%Y-%m-%dT%H:%M:%S");
    if micros == 0 {
        format!("{base}Z")
    } else {
        format!("{base}.{micros:06}Z")
    }
}

/// Every month closure, oldest first.
pub fn load_periods(conn: &Connection) -> Result<Vec<MonthPeriod>> {
    payloads(
        conn,
        "SELECT payload_json FROM month_periods ORDER BY month",
    )
}

/// Saved account roles in the order they were first saved.
pub fn load_roles(conn: &Connection) -> Result<Vec<AccountRole>> {
    payloads(
        conn,
        "SELECT payload_json FROM account_roles ORDER BY rowid",
    )
}

/// Pots by creation; the general pot is always there (a read-only store
/// that lacks it gets one in memory).
pub fn load_pots(conn: &Connection, include_archived: bool) -> Result<Vec<PotRecord>> {
    let mut pots: Vec<PotRecord> = payloads(conn, "SELECT payload_json FROM pots ORDER BY rowid")?;
    if !pots.iter().any(|p| p.pot_id == GENERAL_POT_ID) {
        pots.insert(
            0,
            PotRecord {
                pot_id: GENERAL_POT_ID.to_owned(),
                name: GENERAL_POT_NAME.to_owned(),
                account_id: None,
                archived: false,
                created_at: stored_instant(Timestamp::now()),
            },
        );
    }
    pots.retain(|p| include_archived || !p.archived);
    Ok(pots)
}

/// Pot ledger entries, newest month first (at most `limit`).
pub fn load_pot_entries(conn: &Connection, limit: Option<usize>) -> Result<Vec<PotEntryRecord>> {
    let sql = match limit {
        Some(n) => format!(
            "SELECT payload_json FROM pot_entries ORDER BY month DESC, rowid DESC LIMIT {n}"
        ),
        None => "SELECT payload_json FROM pot_entries ORDER BY month DESC, rowid DESC".to_owned(),
    };
    payloads(conn, &sql)
}

/// Pot balance per currency code, the sum of its entries, optionally
/// without one month's entries.
pub type PotBalances = BTreeMap<String, BTreeMap<String, Decimal>>;

pub fn pot_balances(entries: &[PotEntryRecord], exclude_month: Option<Date>) -> PotBalances {
    let excluded = exclude_month.map(|m| m.to_string());
    let mut balances = PotBalances::new();
    for entry in entries {
        if excluded.as_deref() == Some(entry.month.as_str()) {
            continue;
        }
        *balances
            .entry(entry.pot_id.clone())
            .or_default()
            .entry(entry.currency.code.clone())
            .or_default() += entry.amount;
    }
    balances
}

pub fn pot_view(pot: &PotRecord, balances: &PotBalances) -> PotView {
    PotView {
        pot_id: pot.pot_id.clone(),
        name: pot.name.clone(),
        account_id: pot.account_id.clone(),
        archived: pot.archived,
        balances: balances.get(&pot.pot_id).cloned().unwrap_or_default(),
    }
}

/// Accounts in the Python service's order (`ORDER BY canonical_id`).
pub fn accounts_by_id(accounts: &[Account]) -> Vec<&Account> {
    let mut sorted: Vec<&Account> = accounts.iter().collect();
    sorted.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    sorted
}

/// Transactions in the Python service's order (`ORDER BY canonical_id`).
pub fn transactions_by_id(transactions: &[Tx]) -> Vec<&Tx> {
    let mut sorted: Vec<&Tx> = transactions.iter().collect();
    sorted.sort_by(|a, b| a.transaction_id.cmp(&b.transaction_id));
    sorted
}

/// Every known account with its role, so ignored ones can be re-enabled.
pub fn account_role_views(accounts: &[&Account], roles: &[AccountRole]) -> Vec<AccountRoleView> {
    let by_id: HashMap<&str, &AccountRole> =
        roles.iter().map(|r| (r.account_id.as_str(), r)).collect();
    let known: HashSet<&str> = accounts.iter().map(|a| a.account_id.as_str()).collect();
    accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.name.as_str()))
        .chain(
            roles
                .iter()
                .filter(|r| !known.contains(r.account_id.as_str()))
                .map(|r| (r.account_id.as_str(), r.account_id.as_str())),
        )
        .map(|(id, name)| {
            let role = by_id.get(id);
            AccountRoleView {
                account_id: id.to_owned(),
                name: name.to_owned(),
                role: role.map_or(AccountRoleKind::Unclassified, |r| {
                    AccountRoleKind::parse(&r.role)
                }),
                goal_id: role.and_then(|r| r.goal_id.clone()),
                via_account_id: role.and_then(|r| r.via_account_id.clone()),
            }
        })
        .collect()
}

/// The month a plan line's purchase is due in.
pub fn plan_due_month(line: &AllocationLine) -> Option<Date> {
    line.plan_due_on
        .as_deref()
        .and_then(day_of)
        .map(|d| d.first_of_month())
}

/// The previous month's buffer lines and moves into a spending account (to
/// that target), plus money any month set aside for a planned purchase due
/// this month (it pays for that purchase here).
pub fn carry_in_for(
    month: Date,
    periods: &[MonthPeriod],
    roles: &[AccountRole],
) -> HashMap<String, Decimal> {
    let spends = |id: &str| {
        roles.iter().any(|r| {
            r.account_id == id && AccountRoleKind::parse(&r.role) == AccountRoleKind::Spending
        })
    };
    let mut carried: HashMap<String, Decimal> = HashMap::new();
    let previous = shift_month(month, -1).to_string();
    if let Some(previous) = periods.iter().find(|p| p.month == previous) {
        for line in &previous.allocations {
            if line.destination == "buffer" {
                *carried.entry(line.from_account_id.clone()).or_default() += line.amount;
            } else if let Some(target) = line.target_account_id().filter(|t| spends(t)) {
                *carried.entry(target.to_owned()).or_default() += line.amount;
            }
        }
    }
    for period in periods {
        for line in &period.allocations {
            if line.destination == "plan" && plan_due_month(line) == Some(month) {
                *carried.entry(line.from_account_id.clone()).or_default() += line.amount;
            }
        }
    }
    carried
}

/// What [`closure_review`] reads.
#[derive(Clone, Copy, Debug)]
pub struct ReviewInputs<'a> {
    pub month: Date,
    /// The month's saved EUR budget plan.
    pub plan: Option<&'a BudgetPlan>,
    /// In the Python order ([`accounts_by_id`]).
    pub accounts: &'a [&'a Account],
    pub transactions: &'a [Tx],
    pub resolver: &'a PeriodResolver,
    pub mappings: &'a [GroupMapping],
    pub payments: &'a [PlannedPaymentView],
    pub roles: &'a BTreeMap<String, AccountRole>,
    pub carry_in: &'a HashMap<String, Decimal>,
    /// The budget's fixed transactions (subscriptions and fixed categories);
    /// paid planned outflows due this month count as fixed too.
    pub fixed_ids: &'a HashSet<String>,
}

fn sum(values: impl Iterator<Item = Decimal>) -> Decimal {
    values.fold(Decimal::ZERO, |a, b| a + b)
}

/// The per-account review of a month (Python `closure_review`).
pub fn closure_review(input: &ReviewInputs) -> ClosureReview {
    let month = input.month;
    let group_of: HashMap<&str, Option<&str>> = input
        .mappings
        .iter()
        .map(|m| (m.category.as_str(), m.group.as_deref()))
        .collect();
    let in_month: Vec<&Tx> = input
        .transactions
        .iter()
        .filter(|t| input.resolver.month_of(t) == month)
        .collect();
    let due: Vec<&PlannedPaymentView> = input
        .payments
        .iter()
        .filter(|p| day_of(&p.due_on).map(|d| d.first_of_month()) == Some(month))
        .collect();
    let mut fixed: HashSet<&str> = input.fixed_ids.iter().map(String::as_str).collect();
    fixed.extend(
        due.iter()
            .filter(|p| p.direction == Direction::Outflow && p.status == PlanStatus::Paid)
            .filter_map(|p| p.transaction_id.as_deref()),
    );
    // A received reimbursement nets against the group of the spend it repays.
    let reimbursement_origins: HashMap<&str, &str> = input
        .payments
        .iter()
        .filter(|p| p.status == PlanStatus::Paid)
        .filter_map(|p| {
            Some((
                p.transaction_id.as_deref()?,
                p.reimbursement_for.as_deref()?,
            ))
        })
        .collect();
    let mut group_spent: HashMap<String, BTreeMap<String, Decimal>> = HashMap::new();
    let mut rows: Vec<AccountReview> = Vec::new();
    for account in input.accounts {
        let role = input.roles.get(&account.account_id);
        if role.is_some_and(|r| r.role == "ignored") {
            continue;
        }
        let mine: Vec<&Tx> = in_month
            .iter()
            .copied()
            .filter(|t| t.account_id == account.account_id)
            .collect();
        let own: Vec<&Tx> = mine
            .iter()
            .copied()
            .filter(|t| &t.currency == account.currency())
            .collect();
        let total = |kind: Kind| sum(own.iter().filter(|t| t.kind == kind).map(|t| t.amount));
        let signed = |t: &Tx| {
            if t.kind == Kind::Outflow {
                t.amount
            } else {
                -t.amount
            }
        };
        let spending = || {
            own.iter()
                .copied()
                .filter(|t| matches!(t.kind, Kind::Outflow | Kind::Refund))
        };
        let fixed_spent = sum(spending()
            .filter(|t| fixed.contains(t.transaction_id.as_str()))
            .map(signed));
        let mut other = Decimal::ZERO;
        for t in spending() {
            if fixed.contains(t.transaction_id.as_str()) {
                continue;
            }
            let owner = input.plan.and_then(|plan| {
                budget_owner(
                    &plan.groups,
                    t.category.as_deref(),
                    group_of
                        .get(t.category.as_deref().unwrap_or(""))
                        .copied()
                        .flatten(),
                    Some(
                        reimbursement_origins
                            .get(t.transaction_id.as_str())
                            .copied()
                            .unwrap_or(&t.transaction_id),
                    ),
                )
            });
            match (owner, input.plan) {
                (None, _) => other += signed(t),
                (Some(owner), Some(plan)) if t.currency == plan.currency => {
                    *group_spent
                        .entry(owner.to_owned())
                        .or_default()
                        .entry(account.account_id.clone())
                        .or_default() += signed(t);
                }
                _ => {}
            }
        }
        let (inflow, refunds, outflow) = (
            total(Kind::Inflow),
            total(Kind::Refund),
            total(Kind::Outflow),
        );
        let carried = input
            .carry_in
            .get(&account.account_id)
            .copied()
            .unwrap_or_default();
        let planned = |direction: Direction| {
            sum(due
                .iter()
                .filter(|p| {
                    p.direction == direction
                        && p.account_id.as_deref() == Some(account.account_id.as_str())
                        && &p.amount.currency == account.currency()
                })
                .map(|p| p.amount.amount))
        };
        rows.push(AccountReview {
            account_id: account.account_id.clone(),
            name: account.name.clone(),
            currency: account.currency().clone(),
            role: role.map_or(AccountRoleKind::Unclassified, |r| {
                AccountRoleKind::parse(&r.role)
            }),
            closing_balance: account.balance.amount,
            carry_in: carried,
            income_planned: planned(Direction::Inflow),
            income_actual: inflow,
            fixed_planned: planned(Direction::Outflow),
            fixed_actual: fixed_spent,
            other_spent: other,
            groups: Vec::new(),
            // Own transfers are excluded: they move money, not earn or spend it.
            leftover: inflow + refunds - outflow + carried,
            skipped_foreign: mine.len() - own.len(),
        });
    }
    let mut unassigned = Vec::new();
    for group in input.plan.map(|p| p.groups.as_slice()).unwrap_or(&[]) {
        let spent = group_spent.get(&group.id);
        let home = group
            .account_id
            .as_deref()
            .and_then(|id| rows.iter_mut().find(|r| r.account_id == id));
        let Some(home) = home else {
            unassigned.push(group.id.clone());
            continue;
        };
        let values = || spent.into_iter().flat_map(|s| s.iter());
        let spent_elsewhere = sum(values()
            .filter(|(k, _)| **k != home.account_id)
            .map(|(_, v)| *v));
        home.groups.push(GroupReview {
            group_id: group.id.clone(),
            name: group.name.clone(),
            planned: group.amount,
            spent: sum(values().map(|(_, v)| *v)),
            spent_elsewhere,
        });
    }
    ClosureReview {
        month: month.to_string(),
        accounts: rows,
        unassigned_groups: unassigned,
        unassigned_payments: due
            .iter()
            .filter(|p| p.account_id.is_none())
            .map(|p| p.plan_id.clone())
            .collect(),
        unclassified_accounts: input
            .accounts
            .iter()
            .filter(|a| !input.roles.contains_key(&a.account_id))
            .map(|a| a.account_id.clone())
            .collect(),
    }
}

/// The review of `month` as the closure service built it: the saved EUR
/// plan, reconciled planned payments (salary from the closing month on) and
/// the budget's fixed rule, over `periods` as they stand.
pub fn review_month(
    snap: &Snapshot,
    periods: &[MonthPeriod],
    month: Date,
    now: &Zoned,
) -> ClosureReview {
    let resolver = PeriodResolver::new(periods, snap.first_observed.clone(), snap.zone.clone());
    let ctx = Ctx {
        snap,
        now: now.with_time_zone(snap.zone.clone()),
        resolver,
        events: Vec::new(),
    };
    // The closing month's payday decides whether next month's salary still
    // counts, even when the closure is reviewed after month end.
    let payments = planned_payment_views_since(&ctx, Some(month));
    let month_text = month.to_string();
    let plan = snap
        .budget_plans
        .iter()
        .find(|p| p.month == month_text && p.currency.code == "EUR");
    let categories =
        fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    let fixed_ids = fixed_transaction_ids(&snap.transactions, &categories);
    let accounts = accounts_by_id(&snap.accounts);
    closure_review(&ReviewInputs {
        month,
        plan,
        accounts: &accounts,
        transactions: &snap.transactions,
        resolver: &ctx.resolver,
        mappings: &snap.group_mappings,
        payments: &payments,
        roles: &snap.account_roles,
        carry_in: &carry_in_for(
            month,
            periods,
            &snap.account_roles.values().cloned().collect::<Vec<_>>(),
        ),
        fixed_ids: &fixed_ids,
    })
}

/// What a one-off planned wish can still take this month.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WishCap {
    pub currency: Currency,
    pub room: Decimal,
}

/// What each one-off planned wish can still take this month, keyed by wish id.
pub type WishCaps = HashMap<String, WishCap>;

/// Caps from the wishes and the period's previous lines (earlier
/// contribution added back). A parent with sub-wishes is capped on its rest,
/// as the wish schedule counts it (`plan_items`); every other wish on its
/// own amount. Only one-off planned wishes take a line.
pub fn wish_caps(wishes: &[Wish], previous: &[AllocationLine]) -> WishCaps {
    let one_off = |w: &Wish| w.kind == WishKind::OneOff;
    let mut children: HashMap<&str, Vec<&Wish>> = HashMap::new();
    for w in wishes {
        if let Some(p) = w.parent_id.as_deref() {
            children.entry(p).or_default().push(w);
        }
    }
    // As `earmarked`: a part of a parent that is no longer planned (or gone)
    // takes no line.
    let parent_planned = |id: &str| {
        wishes
            .iter()
            .any(|p| p.wish_id == id && p.status == WishStatus::Planned)
    };
    wishes
        .iter()
        .filter(|w| w.status == WishStatus::Planned && one_off(w))
        .filter(|w| w.parent_id.as_deref().is_none_or(parent_planned))
        .map(|w| {
            let subs: Vec<&Wish> = children
                .get(w.wish_id.as_str())
                .map(|v| v.iter().copied().filter(|s| one_off(s)).collect())
                .unwrap_or_default();
            let need = if subs.is_empty() {
                w.amount
            } else {
                let mut rest = w.amount;
                for sub in subs {
                    match sub.status {
                        WishStatus::Planned => rest -= sub.amount,
                        WishStatus::Bought => {
                            rest -= sub.bought.as_ref().map_or(sub.amount, |b| b.amount);
                        }
                        WishStatus::Dropped | WishStatus::Started => {}
                    }
                }
                rest.max(Decimal::ZERO)
            };
            let earlier = sum(previous
                .iter()
                .filter(|l| l.wish_id() == Some(w.wish_id.as_str()))
                .map(|l| l.amount));
            (
                w.wish_id.clone(),
                WishCap {
                    currency: w.currency.clone(),
                    room: (need - w.saved + earlier).max(Decimal::ZERO),
                },
            )
        })
        .collect()
}

/// `€12.00`: the amount at the currency's minor unit, `USD 12.00` for a
/// currency without a sign here.
fn short_money(value: Decimal, currency: &Currency) -> String {
    let mut value = value.round_dp(u32::from(currency.minor_unit));
    value.rescale(u32::from(currency.minor_unit));
    let text = value.abs().to_string();
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    // Thousands separators, as the UI's `money` writes them.
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if value.is_sign_negative() && !value.is_zero() {
        "-"
    } else {
        ""
    };
    let value = match fraction {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    };
    match currency.code.as_str() {
        "EUR" => format!("€{value}"),
        "GBP" => format!("£{value}"),
        "USD" => format!("${value}"),
        code => format!("{code} {value}"),
    }
}

/// The wishes a split line can save toward, as the Wishes page orders them:
/// top-level wishes by due date, each followed by its sub-wishes. A parent
/// with sub-wishes takes money toward its rest. A wish with no room left is
/// not offered.
pub fn wish_options(wishes: &[Wish], previous: &[AllocationLine]) -> Vec<WishOption> {
    let caps = wish_caps(wishes, previous);
    let by_due = |list: &mut Vec<&Wish>| {
        list.sort_by(|a, b| (&a.due_on, &a.created_at).cmp(&(&b.due_on, &b.created_at)));
    };
    let ids: HashSet<&str> = wishes.iter().map(|w| w.wish_id.as_str()).collect();
    let mut tops: Vec<&Wish> = wishes
        .iter()
        .filter(|w| w.parent_id.as_deref().is_none_or(|p| !ids.contains(p)))
        .collect();
    by_due(&mut tops);
    let mut out = Vec::new();
    for top in tops {
        let mut subs: Vec<&Wish> = wishes
            .iter()
            .filter(|w| w.parent_id.as_deref() == Some(top.wish_id.as_str()))
            .collect();
        by_due(&mut subs);
        let parent = subs.iter().any(|s| s.kind == WishKind::OneOff);
        for wish in std::iter::once(top).chain(subs) {
            // A fully saved wish stays in the caps (a stored line to it still
            // validates and labels) but is not offered.
            let Some(cap) = caps.get(&wish.wish_id).filter(|c| c.room > Decimal::ZERO) else {
                continue;
            };
            let label = if parent && wish.wish_id == top.wish_id {
                format!("Wish: {} rest", wish.name)
            } else {
                format!(
                    "Wish: {} ({} of {})",
                    wish.name,
                    short_money(wish.saved, &wish.currency),
                    short_money(wish.amount, &wish.currency)
                )
            };
            out.push(WishOption {
                wish_id: wish.wish_id.clone(),
                label,
                currency: cap.currency.clone(),
                room: cap.room,
            });
        }
    }
    out
}

/// The wish options for the period's editor, with room as the server will
/// check it: a split or re-split replaces the period's lines, so their
/// earlier contribution counts back; a resettle appends delta lines, so it
/// does not.
pub fn period_wish_options(wishes: &[Wish], period: &MonthPeriod) -> Vec<WishOption> {
    if period.status == ClosureStatus::ChangedAfterClose {
        wish_options(wishes, &[])
    } else {
        wish_options(wishes, &period.allocations)
    }
}

/// `(wish_id, label)` of every wish for naming lines: `Wish: <name>` while
/// it is offered, else what became of it.
pub fn wish_labels(wishes: &[Wish], options: &[WishOption]) -> Vec<(String, String)> {
    let has_subs = |id: &str| {
        wishes
            .iter()
            .any(|s| s.parent_id.as_deref() == Some(id) && s.kind == WishKind::OneOff)
    };
    wishes
        .iter()
        .map(|w| {
            let offered = options.iter().any(|o| o.wish_id == w.wish_id);
            let label = match w.status {
                _ if offered && has_subs(&w.wish_id) => format!("Wish: {} rest", w.name),
                _ if offered => format!("Wish: {}", w.name),
                WishStatus::Bought => format!("Wish (bought): {}", w.name),
                WishStatus::Dropped => format!("Wish (dropped): {}", w.name),
                // Planned or started but not one-off (repeating, bill change):
                // it never takes a line, so an old line to it is legacy.
                WishStatus::Planned | WishStatus::Started => {
                    format!("Wish (no longer planned): {}", w.name)
                }
            };
            (w.wish_id.clone(), label)
        })
        .collect()
}

/// Split mode: every positive leftover fully assigned with positive lines.
/// Delta mode (`baseline` given): lines sum to the leftover change; negatives
/// allowed, except on a wish line. Pot lines are legacy and refused in both.
/// `wishes` holds what each planned wish can still take; `inactive` lists the
/// accounts a move cannot target.
pub fn validate_allocations(
    review: &ClosureReview,
    lines: &[AllocationLine],
    goal_ids: &HashSet<&str>,
    baseline: Option<&ClosureReview>,
    wishes: &WishCaps,
    inactive: &HashSet<&str>,
) -> std::result::Result<(), String> {
    let before: HashMap<&str, Decimal> = baseline
        .map(|b| {
            b.accounts
                .iter()
                .map(|a| (a.account_id.as_str(), a.leftover))
                .collect()
        })
        .unwrap_or_default();
    let ids: HashSet<&str> = lines.iter().map(|l| l.line_id.as_str()).collect();
    if ids.len() != lines.len() {
        return Err("duplicate allocation line".into());
    }
    let mut accounts: Vec<AccountReview> = review.accounts.clone();
    if let Some(baseline) = baseline {
        // Accounts that dropped out of the review (e.g. now ignored) count as 0.
        let current: HashSet<String> = accounts.iter().map(|a| a.account_id.clone()).collect();
        accounts.extend(
            baseline
                .accounts
                .iter()
                .filter(|a| !current.contains(&a.account_id))
                .map(|a| AccountReview {
                    leftover: Decimal::ZERO,
                    ..a.clone()
                }),
        );
    }
    let known: HashSet<&str> = accounts.iter().map(|a| a.account_id.as_str()).collect();
    let mut to_wish: HashMap<&str, Decimal> = HashMap::new();
    let next_month = day_of(&review.month).map(|m| shift_month(m, 1));
    for item in lines {
        if !known.contains(item.from_account_id.as_str()) {
            return Err("allocation from an unknown account".into());
        }
        if item.goal_id().is_some_and(|g| !goal_ids.contains(g)) {
            return Err("unknown savings goal".into());
        }
        if let Some(wish) = item.wish_id() {
            let Some(cap) = wishes.get(wish) else {
                return Err("unknown wish".into());
            };
            if item.amount <= Decimal::ZERO {
                return Err("a wish line must be positive".into());
            }
            if item.currency != cap.currency {
                return Err("wish currency does not match".into());
            }
            let total = to_wish.entry(wish).or_default();
            *total += item.amount;
            if *total > cap.room {
                return Err("saves more than the wish still needs".into());
            }
        } else if let Some(target) = item.target_account_id() {
            if target == item.from_account_id {
                return Err("cannot move money to the same account".into());
            }
            let Some(to) = review.accounts.iter().find(|a| {
                a.account_id == target
                    && !inactive.contains(target)
                    && matches!(a.role, AccountRoleKind::Spending | AccountRoleKind::Savings)
            }) else {
                return Err("move to an unknown account".into());
            };
            if item.amount <= Decimal::ZERO {
                return Err("a move must be positive".into());
            }
            if to.currency != item.currency {
                return Err("move currency does not match".into());
            }
        } else if item.pot_id().is_some() || (item.amount < Decimal::ZERO && baseline.is_none()) {
            return Err("pots are no longer available".into());
        }
        if item.destination == "plan" {
            let (Some(_), Some(due)) = (&item.plan_name, item.plan_due_on.as_deref()) else {
                return Err("a planned purchase needs a name and a due date".into());
            };
            if day_of(due).zip(next_month).is_some_and(|(d, n)| d < n) {
                return Err("a planned purchase must be due after this month".into());
            }
            if item.amount <= Decimal::ZERO {
                return Err("a planned purchase must be positive".into());
            }
        }
    }
    for account in &accounts {
        let own: Vec<&AllocationLine> = lines
            .iter()
            .filter(|l| l.from_account_id == account.account_id)
            .collect();
        if own.iter().any(|l| l.currency != account.currency) {
            return Err("allocation currency must match the account".into());
        }
        let assigned = sum(own.iter().map(|l| l.amount));
        let target = if baseline.is_some() {
            account.leftover.max(Decimal::ZERO)
                - before
                    .get(account.account_id.as_str())
                    .copied()
                    .unwrap_or_default()
                    .max(Decimal::ZERO)
        } else {
            if account.leftover <= Decimal::ZERO && !own.is_empty() {
                return Err(format!("{} has a deficit; nothing to split", account.name));
            }
            if own.iter().any(|l| l.amount <= Decimal::ZERO) {
                return Err("allocation amounts must be positive".into());
            }
            account.leftover.max(Decimal::ZERO)
        };
        if assigned != target {
            return Err(format!("{} has unassigned leftover", account.name));
        }
    }
    Ok(())
}

/// Everyday habit spending still expected this month, outside every group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EverydayForecast {
    pub model: String,
    pub remaining: Decimal,
    pub excluded: Option<ForecastExclusions>,
}

/// Overview's EUR habit forecast from today to `month`'s end, less what its
/// budget groups are expected to take (those are in groups_remaining).
pub fn everyday_forecast(
    forecasts: &[DailySpendingForecast],
    month: &str,
) -> Option<EverydayForecast> {
    forecasts
        .iter()
        .filter(|f| f.currency.code == "EUR")
        .find_map(|f| {
            let found = f.months.iter().find(|m| m.month == month)?;
            let grouped = sum(found.groups.iter().map(|g| g.remaining));
            Some(EverydayForecast {
                model: f.model.clone(),
                remaining: (found.remaining - grouped).max(Decimal::ZERO),
                excluded: f.excluded.clone(),
            })
        })
}

/// What is left now once this month's remaining needs are paid (EUR).
///
/// Starts from Overview's projected available for the month being funded
/// (money in the bank less bills, plans and group budgets still to pay, plus
/// income still expected), then reserves the everyday spending still
/// forecast this month and one-off payments due in the next months. Next
/// month's salary funds the month after, so it is never counted.
pub fn savings_suggestion(
    allocation: &CurrencyAllocation,
    as_of: Date,
    payments: &[PlannedPaymentView],
    safety_buffer: Decimal,
    forecast: Option<EverydayForecast>,
) -> SavingsSuggestion {
    let month = day_of(&allocation.current_month).unwrap_or(as_of.first_of_month());
    let (from, to) = (
        shift_month(month, 1),
        shift_month(month, KNOWN_PAYMENT_MONTHS + 1),
    );
    let mut sorted: Vec<&PlannedPaymentView> = payments.iter().collect();
    sorted.sort_by(|a, b| (&a.due_on, &a.name).cmp(&(&b.due_on, &b.name)));
    let known: Vec<KnownPayment> = sorted
        .into_iter()
        .filter(|p| {
            p.direction == Direction::Outflow
                && p.status != PlanStatus::Paid
                && p.amount.currency.code == "EUR"
                && day_of(&p.due_on).is_some_and(|d| from <= d && d < to)
                && !p.plan_id.starts_with(CLOSURE_PLAN_PREFIX)
                && !p.plan_id.starts_with(RECURRING_PLAN_PREFIX)
        })
        .map(|p| KnownPayment {
            name: p.name.clone(),
            due_on: p.due_on.clone(),
            amount: p.amount.amount,
        })
        .collect();
    SavingsSuggestion {
        as_of: as_of.to_string(),
        month: month.to_string(),
        projected_available: allocation.current_planned_wiggle_room,
        bank_balance: allocation.bank_balance,
        protected_savings: allocation.protected_savings,
        salary_reserved: allocation.salary_reserved,
        bills_remaining: allocation.current_fixed_remaining,
        plans_remaining: allocation.current_plans_remaining,
        groups_remaining: allocation.current_groups_remaining,
        expected_income: allocation.current_expected_income,
        everyday_remaining: forecast.as_ref().map_or(Decimal::ZERO, |f| f.remaining),
        forecast_available: forecast.is_some(),
        forecast_model: forecast.as_ref().map(|f| f.model.clone()),
        forecast_excluded: forecast.and_then(|f| f.excluded),
        known_future_payments: sum(known.iter().map(|k| k.amount)),
        known_payments: known,
        safety_buffer,
    }
}

/// Outflows from known accounts whose memo sounds like saving money.
pub fn suspected_savings_transfers(
    transactions: &[&Tx],
    known_accounts: &HashSet<&str>,
) -> Vec<String> {
    static SAVINGS_HINT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(savings?|vault|pocket|spaar\w*)\b").expect("valid regex")
    });
    transactions
        .iter()
        .filter(|t| {
            t.kind == Kind::Outflow
                && known_accounts.contains(t.account_id.as_str())
                && SAVINGS_HINT.is_match(t.description.as_deref().unwrap_or(""))
        })
        .map(|t| t.transaction_id.clone())
        .collect()
}

/// Accounts without a role and suspected savings transfers (Python
/// `/v1/accounts/unclassified`).
pub fn unclassified_accounts(snap: &Snapshot) -> UnclassifiedAccounts {
    let accounts = accounts_by_id(&snap.accounts);
    let known: HashSet<&str> = accounts.iter().map(|a| a.account_id.as_str()).collect();
    UnclassifiedAccounts {
        accounts: accounts
            .iter()
            .filter(|a| !snap.account_roles.contains_key(&a.account_id))
            .map(|a| NamedAccount {
                account_id: a.account_id.clone(),
                name: a.name.clone(),
            })
            .collect(),
        suspected: suspected_savings_transfers(&transactions_by_id(&snap.transactions), &known),
    }
}

/// The stored closure of `month`, or a fresh open one starting where the
/// periods say.
pub fn period_or_new(
    periods: &[MonthPeriod],
    month: Date,
    resolver: &PeriodResolver,
) -> MonthPeriod {
    let text = month.to_string();
    periods
        .iter()
        .find(|p| p.month == text)
        .cloned()
        .unwrap_or_else(|| MonthPeriod {
            month: text,
            starts_at: resolver.period_start_stamp(month),
            cutoff_at: None,
            status: Default::default(),
            settled_at: None,
            settled_snapshot: None,
            missing_banks: Vec::new(),
            allocations: Vec::new(),
            revision: 0,
            reviewed_through: None,
            reopened_at: None,
            balance_checks: BTreeMap::new(),
        })
}

/// Own-account transfers since the split, when the split has any transfer.
pub fn candidates(
    snap: &Snapshot,
    period: &MonthPeriod,
) -> Vec<finance_types::report::closure::TransferCandidate> {
    let Some(settled_at) = period.settled_at.as_deref().and_then(parse_instant) else {
        return Vec::new();
    };
    if period
        .allocations
        .iter()
        .all(|l| l.transfer_status == TransferStatus::NotNeeded)
    {
        return Vec::new();
    }
    transfers::own_transfers(
        &transactions_by_id(&snap.transactions),
        settled_at,
        &snap.zone,
    )
}

/// Computed as of now from the figures Overview shows, never from the
/// closing month's leftover or its settled snapshot, so anything paid since
/// (including after a reopen) has already left the bank balance.
fn suggestion_and_plan(
    store: &Store,
    snap: &Snapshot,
    now: &Zoned,
    forecaster: Option<&dyn crate::daily::SeriesForecaster>,
) -> Result<(Option<SavingsSuggestion>, Option<MoneyPlan>)> {
    let overview = dashboard::dashboard(store, snap, now, forecaster)?;
    let Some(allocation) = overview
        .monthly_allocation
        .as_ref()
        .and_then(|a| a.currencies.iter().find(|c| c.currency.code == "EUR"))
    else {
        return Ok((None, None));
    };
    let as_of = now.with_time_zone(snap.zone.clone()).date();
    let suggestion = savings_suggestion(
        allocation,
        as_of,
        &overview.planned_payments,
        snap.settings.safety_buffer_eur,
        everyday_forecast(&overview.daily_forecasts, &allocation.current_month),
    );
    Ok((Some(suggestion), overview.money_plan))
}

/// Everything the closure screen shows for `month` (Python `closure_view`).
pub fn closure_view(
    store: &Store,
    snap: &Snapshot,
    month: Date,
    now: &Zoned,
    forecaster: Option<&dyn crate::daily::SeriesForecaster>,
) -> Result<ClosureView> {
    let conn = store.connection();
    let periods = &snap.month_periods;
    let resolver = PeriodResolver::new(periods, snap.first_observed.clone(), snap.zone.clone());
    let mut period = period_or_new(periods, month, &resolver);
    // starts_at depends on the previous month's cutoff, which may have moved
    // since this period was saved, so it is always derived on read.
    period.starts_at = resolver.period_start_stamp(month);
    let entries = load_pot_entries(conn, None)?;
    let balances = pot_balances(&entries, None);
    let roles = load_roles(conn)?;
    let all_wishes = crate::wishes::store::load_wishes(conn)?;
    let wishes = period_wish_options(&all_wishes, &period);
    let (suggestion, plan) = suggestion_and_plan(store, snap, now, forecaster)?;
    Ok(ClosureView {
        balances: balance_statuses(conn, snap, &period, &roles)?,
        plan,
        review: review_month(snap, periods, month, now),
        goals: snap
            .goals
            .iter()
            .map(|g| (g.goal.goal_id.clone(), g.goal.name.clone()))
            .collect(),
        pots: load_pots(conn, true)?
            .iter()
            .map(|p| pot_view(p, &balances))
            .collect(),
        wish_labels: wish_labels(&all_wishes, &wishes),
        wishes,
        suggestion,
        accounts: account_role_views(&accounts_by_id(&snap.accounts), &roles),
        transfer_candidates: candidates(snap, &period),
        period,
    })
}

/// One status per checked account, in review order.
pub fn balance_statuses(
    conn: &Connection,
    snap: &Snapshot,
    period: &MonthPeriod,
    roles: &[AccountRole],
) -> Result<Vec<BalanceStatus>> {
    let accounts = crate::writes::closure::checked_accounts(snap, roles);
    if accounts.is_empty() {
        return Ok(Vec::new());
    }
    let runs = crate::bank_sync::runs(conn)?;
    accounts
        .into_iter()
        .map(|account| {
            let currency = account.currency().clone();
            let ledger = account.balance.amount;
            // The newest bank figure across every bank's last run (no date
            // sorts oldest).
            let reported = runs
                .values()
                .flat_map(|run| &run.balances)
                .filter(|b| b.account_id == account.ledger_id && b.currency == currency.code)
                .max_by(|a, b| a.reference_date.cmp(&b.reference_date));
            let check = period.balance_checks.get(&account.account_id).cloned();
            let state = match &check {
                None => BalanceState::Unconfirmed,
                Some(c) if c.ledger_balance == ledger => BalanceState::Confirmed,
                Some(_) => BalanceState::Stale,
            };
            let booked = check
                .as_ref()
                .and_then(|c| c.transaction_id.as_deref())
                .and_then(|id| id.strip_prefix("firefly:"))
                .and_then(|id| id.parse::<i64>().ok());
            let adjusted_by = match booked {
                Some(id) => conn
                    .query_row(
                        "SELECT p.amount FROM postings p JOIN transactions t ON t.id = p.transaction_id
                         WHERE p.transaction_id = ?1 AND p.account_id = ?2
                           AND t.kind = 'reconciliation'",
                        rusqlite::params![id, account.ledger_id],
                        |r| r.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|text| text.parse::<Decimal>().ok()),
                None => None,
            };
            Ok(BalanceStatus {
                account_id: account.account_id.clone(),
                name: account.name.clone(),
                currency,
                ledger,
                bank: reported.and_then(|b| b.bank.parse().ok()),
                bank_reported_on: reported.and_then(|b| b.reference_date.clone()),
                check,
                state,
                adjusted_by,
            })
        })
        .collect()
}

/// Names of checked accounts whose status is not Confirmed (empty = may settle).
pub(crate) fn unconfirmed_names(statuses: &[BalanceStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|s| s.state != BalanceState::Confirmed)
        .map(|s| s.name.clone())
        .collect()
}

/// A currency the closure screen sends: three ASCII letters, any case.
pub fn normalized_currency(currency: &Currency) -> Option<Currency> {
    let code = currency.code.to_ascii_uppercase();
    let valid = code.len() == 3
        && code.chars().all(|c| c.is_ascii_alphabetic())
        && currency.minor_unit <= 18;
    valid.then_some(Currency {
        code,
        minor_unit: currency.minor_unit,
    })
}

#[cfg(test)]
pub(crate) mod tests;
