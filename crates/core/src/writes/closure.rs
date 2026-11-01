//! Month closure actions (Python `closure_service.py`, with the request
//! checks of `closure_routes.py`): sync, settle, finalize, resettle, reopen,
//! marking transfers by hand, pots and account roles.
//!
//! "Sync" re-reads the local data and runs the post-sync step (late
//! arrivals flag closed months, late transfers are ticked off); the screen
//! pulls fresh bank data through `bank_sync` before it asks.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};

use finance_types::gateway::{AccountRole, Direction, PlannedPayment, PotRecord};
use finance_types::report::UnitMoney;
use finance_types::report::closure::{
    AccountRoleDraft, AllocationLine, BalanceCheck, ClosureAction, ClosureReview, ClosureStatus,
    LineKind, MonthPeriod, PotDraft, PotView, TransferStatus,
};
use jiff::Timestamp;
use jiff::civil::Date;
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use rust_decimal::Decimal;
use std::sync::LazyLock;

use super::{Writer, invalid};
use crate::closure::transfers::{
    mark_transfer, match_transfers, reroute_pending, unmark_transfer, with_transfer_targets,
};
use crate::closure::{
    BANKS, GENERAL_POT_ID, PLANNED_PURCHASE_CATEGORY, candidates, day_of, general_pot,
    load_periods, load_pot_entries, load_pots, load_roles, normalized_currency, open_pot_balances,
    parse_month, period_or_new, plan_due_month, pot_balances, pot_entries, pot_view, review_month,
    stored_instant, transactions_by_id, validate_allocations,
};
use crate::load::parse_instant;
use crate::model::Kind;
use crate::period::{PeriodResolver, shift_month};
use crate::plans::CLOSURE_PLAN_PREFIX;
use crate::{Error, Result, Snapshot};

use ClosureStatus::{ChangedAfterClose, Closed, Open, Settled, Settling};

/// Lines one split may hold (the web route's limit).
const MAX_LINES: usize = 200;
/// Bank transfers one mark may name.
const MAX_MARKED: usize = 20;

const CHANGED: &str = "month closure changed; refresh";

/// The sync step that comes before a `sync` or `finalize` (Python's route
/// refreshed the bank data, then ran `refresh_after_sync`): a stale screen
/// fails first; then closed months that received late transactions are
/// flagged and late transfers ticked off. Commit it on its own, before
/// [`act`], so it holds even when the step itself is refused. Other steps
/// need nothing here.
pub fn prepare(w: &Writer<'_>, snap: &Snapshot, action: &ClosureAction) -> Result<()> {
    let (ClosureAction::Sync {
        month,
        revision,
        missing_banks,
    }
    | ClosureAction::Finalize {
        month,
        revision,
        missing_banks,
    }) = action
    else {
        return Ok(());
    };
    let month = parse_month(month)?;
    check_banks(missing_banks)?;
    require_revision(w, snap, month, *revision)?;
    refresh_after_sync(w, snap)
}

/// Run one closure step (after [`prepare`]); returns the month it acted on.
pub fn act(w: &Writer<'_>, snap: &Snapshot, action: ClosureAction) -> Result<Date> {
    let month = parse_month(action.month())?;
    match action {
        ClosureAction::Sync {
            revision,
            missing_banks,
            ..
        } => sync(w, snap, month, revision, &missing_banks),
        ClosureAction::Settle {
            revision, lines, ..
        } => settle(w, snap, month, revision, lines),
        ClosureAction::Finalize {
            revision,
            missing_banks,
            ..
        } => finalize(w, snap, month, revision, &missing_banks),
        ClosureAction::Resettle {
            revision, lines, ..
        } => resettle(w, snap, month, revision, lines),
        ClosureAction::Reopen { revision, .. } => reopen(w, snap, month, revision),
        ClosureAction::Mark {
            revision,
            line_id,
            hop,
            transaction_ids,
            ..
        } => mark(w, snap, month, revision, &line_id, hop, &transaction_ids),
        ClosureAction::Unmark {
            revision,
            line_id,
            hop,
            ..
        } => unmark(w, month, revision, &line_id, hop),
        ClosureAction::ConfirmBalance {
            revision,
            account_id,
            balance,
            ..
        } => confirm_balance(w, snap, month, revision, &account_id, balance),
    }?;
    Ok(month)
}

/// `now` as pydantic wrote it into payloads.
fn stamp(w: &Writer<'_>) -> String {
    stored_instant(w.now.timestamp())
}

/// `now` as Python's `isoformat()` wrote it into `updated_at`/`archived_at`.
fn iso_stamp(w: &Writer<'_>) -> String {
    stored_instant(w.now.timestamp()).replace('Z', "+00:00")
}

fn periods(w: &Writer<'_>) -> Result<Vec<MonthPeriod>> {
    load_periods(&w.tx)
}

fn resolver(snap: &Snapshot, periods: &[MonthPeriod]) -> PeriodResolver {
    PeriodResolver::new(periods, snap.first_observed.clone(), snap.zone.clone())
}

/// The stored closure of `month`, or a fresh open one.
fn period(w: &Writer<'_>, snap: &Snapshot, month: Date) -> Result<(Vec<MonthPeriod>, MonthPeriod)> {
    let all = periods(w)?;
    let found = period_or_new(&all, month, &resolver(snap, &all));
    Ok((all, found))
}

fn require(period: &MonthPeriod, allowed: &[ClosureStatus], revision: u32) -> Result<()> {
    if !allowed.contains(&period.status) {
        return Err(invalid(format!("month is {}", period.status.as_str())));
    }
    if period.revision != revision {
        return Err(invalid(CHANGED));
    }
    Ok(())
}

fn require_no_later_closure(periods: &[MonthPeriod], month: Date) -> Result<()> {
    let later = periods
        .iter()
        .any(|p| p.cutoff_at.is_some() && day_of(&p.month).is_some_and(|m| m > month));
    if later {
        return Err(invalid("a later month is already closed"));
    }
    Ok(())
}

/// Save `period` if nobody saved it since its revision; `None` on a conflict.
fn try_save(w: &Writer<'_>, period: MonthPeriod) -> Result<Option<MonthPeriod>> {
    let saved = MonthPeriod {
        revision: period.revision + 1,
        ..period
    };
    let json = serde_json::to_string(&saved).map_err(|e| Error::Invalid(e.to_string()))?;
    let changed = w.tx.execute(
        "INSERT INTO month_periods(month, revision, updated_at, payload_json) \
         SELECT ?1, ?2, ?3, ?4 \
         WHERE COALESCE((SELECT revision FROM month_periods WHERE month = ?1), 0) = ?5 \
         ON CONFLICT(month) DO UPDATE SET revision = excluded.revision, \
         updated_at = excluded.updated_at, payload_json = excluded.payload_json",
        params![
            saved.month,
            saved.revision,
            iso_stamp(w),
            json,
            saved.revision - 1
        ],
    )?;
    Ok((changed == 1).then_some(saved))
}

fn save(w: &Writer<'_>, period: MonthPeriod) -> Result<MonthPeriod> {
    try_save(w, period)?.ok_or_else(|| invalid(CHANGED))
}

fn check_banks(missing: &[String]) -> Result<Vec<String>> {
    let mut names: Vec<String> = missing.iter().filter(|b| !b.is_empty()).cloned().collect();
    names.sort();
    names.dedup();
    if missing.len() > 50 || names.iter().any(|b| !BANKS.contains(&b.as_str())) {
        return Err(invalid("invalid missing_banks"));
    }
    Ok(names)
}

/// Cheap early check so a stale screen fails before the sync step.
fn require_revision(w: &Writer<'_>, snap: &Snapshot, month: Date, revision: u32) -> Result<()> {
    if period(w, snap, month)?.1.revision != revision {
        return Err(invalid(CHANGED));
    }
    Ok(())
}

fn chars_within(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.chars().count())
}

fn optional_within(text: &Option<String>, max: usize) -> bool {
    text.as_deref().is_none_or(|t| chars_within(t, max))
}

/// pydantic's `max_digits=16, decimal_places=2` on a normalized decimal.
fn cents(amount: Decimal) -> bool {
    let n = amount.normalize();
    let scale = n.scale();
    let digits = n.mantissa().unsigned_abs().to_string().len();
    let integer = digits.saturating_sub(scale as usize);
    scale <= 2 && integer + scale as usize <= 16 && integer <= 14
}

/// The field rules the Python model enforced on every line a screen sends.
fn checked_lines(lines: Vec<AllocationLine>) -> Result<Vec<AllocationLine>> {
    static DESTINATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(goal:[^\s]{1,128}|pot:[^\s]{1,64}|plan|buffer|stay)$").expect("valid regex")
    });
    let bad = || invalid("invalid lines");
    if lines.len() > MAX_LINES {
        return Err(bad());
    }
    lines
        .into_iter()
        .map(|mut line| {
            let valid = chars_within(&line.line_id, 64)
                && chars_within(&line.from_account_id, 128)
                && DESTINATION.is_match(&line.destination)
                && cents(line.amount)
                && optional_within(&line.to_account_id, 128)
                && optional_within(&line.plan_name, 128)
                && optional_within(&line.plan_match_category, 128)
                && optional_within(&line.plan_match_description, 128)
                && optional_within(&line.plan_id, 64)
                && optional_within(&line.transfer_source_account_id, 128)
                && line.transfer_hops.iter().all(|h| {
                    chars_within(&h.from_account_id, 128)
                        && chars_within(&h.to_account_id, 128)
                        && h.amount > Decimal::ZERO
                        && cents(h.amount)
                        && h.transfer_status != TransferStatus::NotNeeded
                });
            let currency = normalized_currency(&line.currency);
            let due = match line.plan_due_on.as_deref() {
                None => Some(None),
                Some(text) => text.parse::<Date>().ok().map(Some),
            };
            match (valid, currency, due) {
                (true, Some(currency), Some(due)) => {
                    line.currency = currency;
                    line.plan_due_on = due.map(|d| d.to_string());
                    Ok(line)
                }
                _ => Err(bad()),
            }
        })
        .collect()
}

/// Drop server-owned transfer fields a client may have sent.
fn client_lines(lines: Vec<AllocationLine>, kind: LineKind) -> Vec<AllocationLine> {
    lines
        .into_iter()
        .map(|line| AllocationLine {
            kind,
            transfer_status: TransferStatus::NotNeeded,
            matched_transaction_ids: Vec::new(),
            to_account_id: None,
            transfer_source_account_id: None,
            transfer_hops: Vec::new(),
            manual: false,
            moved_amount: None,
            plan_id: None,
            ..line
        })
        .collect()
}

fn plan_prefix(month: Date) -> String {
    format!("{CLOSURE_PLAN_PREFIX}{month}-")
}

fn truncated(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn with_plan_ids(
    month: Date,
    lines: Vec<AllocationLine>,
    taken: &HashSet<String>,
) -> Result<Vec<AllocationLine>> {
    let mut used = taken.clone();
    lines
        .into_iter()
        .map(|line| {
            if line.destination != "plan" {
                return Ok(line);
            }
            let plan_id = truncated(&format!("{}{}", plan_prefix(month), line.line_id), 64);
            if !used.insert(plan_id.clone()) {
                return Err(invalid("duplicate planned purchase"));
            }
            Ok(AllocationLine {
                plan_id: Some(plan_id),
                ..line
            })
        })
        .collect()
}

fn goal_ids(snap: &Snapshot) -> HashSet<&str> {
    snap.goals.iter().map(|g| g.goal.goal_id.as_str()).collect()
}

/// The general pot exists from the first closure read on.
fn ensure_general_pot(w: &Writer<'_>) -> Result<()> {
    w.put_json(
        "INSERT OR IGNORE INTO pots(pot_id, payload_json) VALUES (?2, ?1)",
        &general_pot(w.now.timestamp()),
        &[&GENERAL_POT_ID],
    )
}

fn active_payments(w: &Writer<'_>) -> Result<Vec<PlannedPayment>> {
    crate::load::payloads(
        &w.tx,
        "SELECT payload_json FROM planned_payments WHERE archived_at IS NULL ORDER BY plan_id",
    )
}

/// What a plan line's planned payment is built from.
fn plan_fields(line: &AllocationLine) -> impl PartialEq + '_ {
    (
        &line.from_account_id,
        line.amount,
        &line.currency,
        &line.plan_name,
        &line.plan_due_on,
        &line.plan_match_category,
        &line.plan_match_description,
    )
}

/// Rewrite the month's pot entries; create or update the planned purchase of
/// every plan line that is new or changed since `previous`. Unchanged lines
/// leave their plan alone, so a plan the owner archived stays archived.
fn record_side_effects(
    w: &Writer<'_>,
    period: &MonthPeriod,
    previous: &[AllocationLine],
    sync_plans: bool,
) -> Result<()> {
    w.tx.execute("DELETE FROM pot_entries WHERE month = ?1", [&period.month])?;
    for entry in pot_entries(period, &stamp(w)) {
        w.put_json(
            "INSERT INTO pot_entries(entry_id, pot_id, month, payload_json) VALUES (?2, ?3, ?4, ?1)",
            &entry,
            &[&entry.entry_id, &entry.pot_id, &entry.month],
        )?;
    }
    if !sync_plans {
        return Ok(());
    }
    let prefix = format!("{CLOSURE_PLAN_PREFIX}{}-", period.month);
    let active: HashMap<String, PlannedPayment> = active_payments(w)?
        .into_iter()
        .filter(|p| p.plan_id.starts_with(&prefix))
        .map(|p| (p.plan_id.clone(), p))
        .collect();
    let before: HashMap<&str, &AllocationLine> = previous
        .iter()
        .filter(|l| l.destination == "plan")
        .filter_map(|l| Some((l.plan_id.as_deref()?, l)))
        .collect();
    let mut wanted: HashSet<&str> = HashSet::new();
    for line in period
        .allocations
        .iter()
        .filter(|l| l.destination == "plan")
    {
        let (Some(plan_id), Some(name), Some(due_on)) =
            (line.plan_id.as_deref(), &line.plan_name, &line.plan_due_on)
        else {
            continue;
        };
        wanted.insert(plan_id);
        if before
            .get(plan_id)
            .is_some_and(|old| plan_fields(old) == plan_fields(line))
        {
            continue;
        }
        let plan = PlannedPayment {
            plan_id: plan_id.to_owned(),
            name: name.clone(),
            match_category: line
                .plan_match_category
                .clone()
                .unwrap_or_else(|| PLANNED_PURCHASE_CATEGORY.to_owned()),
            direction: Direction::Outflow,
            amount: UnitMoney {
                amount: line.amount,
                currency: line.currency.clone(),
            },
            due_on: due_on.clone(),
            created_at: active
                .get(plan_id)
                .map_or_else(|| stamp(w), |p| p.created_at.clone()),
            reimbursement_for: None,
            pending: false,
            account_id: Some(line.from_account_id.clone()),
            match_description: line.plan_match_description.clone(),
        };
        w.put_json(
            "INSERT INTO planned_payments(plan_id, payload_json) VALUES (?2, ?1) \
             ON CONFLICT(plan_id) DO UPDATE SET payload_json = excluded.payload_json, archived_at = NULL",
            &plan,
            &[&plan.plan_id],
        )?;
    }
    let mut stale: Vec<&String> = active
        .keys()
        .filter(|id| !wanted.contains(id.as_str()))
        .collect();
    stale.sort();
    for plan_id in stale {
        w.tx.execute(
            "UPDATE planned_payments SET archived_at = ?1 WHERE plan_id = ?2 AND archived_at IS NULL",
            params![iso_stamp(w), plan_id],
        )?;
    }
    Ok(())
}

/// Flag closed months that received late transactions and tick off late
/// transfers (Python `refresh_after_sync`, run after every bank sync).
pub fn refresh_after_sync(w: &Writer<'_>, snap: &Snapshot) -> Result<()> {
    let all = periods(w)?;
    let resolver = resolver(snap, &all);
    let transactions = transactions_by_id(&snap.transactions);
    for period in all {
        // Settled months are matched by finalize itself; touching them here
        // would bump their revision under the wizard.
        let Some(settled_at) = period.settled_at.as_deref().and_then(parse_instant) else {
            continue;
        };
        if !matches!(period.status, Closed | ChangedAfterClose) {
            continue;
        }
        let allocations =
            match_transfers(&period.allocations, &transactions, settled_at, &snap.zone);
        let mut status = period.status;
        // A month already flagged only has its transfers ticked off.
        if let (Some(cutoff), Closed) =
            (period.cutoff_at.as_deref().and_then(parse_instant), status)
        {
            let reviewed = cutoff.max(
                period
                    .reviewed_through
                    .as_deref()
                    .and_then(parse_instant)
                    .unwrap_or(cutoff),
            );
            let month = day_of(&period.month);
            // Transfers move money without changing leftover, so a late
            // transfer leg alone never reopens a closed month.
            let late = snap.transactions.iter().any(|t| {
                t.kind != Kind::Transfer
                    && Some(resolver.month_of(t)) == month
                    && snap
                        .first_observed
                        .get(&t.transaction_id)
                        .is_some_and(|seen| *seen > reviewed)
            });
            if late {
                status = ChangedAfterClose;
            }
        }
        if allocations != period.allocations || status != period.status {
            // A conflict means a screen write won; the next sync revisits it.
            try_save(
                w,
                MonthPeriod {
                    allocations,
                    status,
                    ..period
                },
            )?;
        }
    }
    Ok(())
}

/// Start settling (from the 25th). Run [`prepare`] first: it re-reads the
/// data the bank sync just wrote.
pub fn sync(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    missing_banks: &[String],
) -> Result<()> {
    let missing = check_banks(missing_banks)?;
    let (all, period) = period(w, snap, month)?;
    require(&period, &[Open, Settling], revision)?;
    require_no_later_closure(&all, month)?;
    if w.now.with_time_zone(snap.zone.clone()).date()
        < month.with().day(25).build().unwrap_or(month)
    {
        return Err(invalid("closing opens from the 25th"));
    }
    save(
        w,
        MonthPeriod {
            status: Settling,
            missing_banks: missing,
            ..period
        },
    )?;
    Ok(())
}

/// Save (or replace) the split. A re-split replaces the old one and its
/// transfer state from scratch but keeps the first settle time, so
/// transfers made in between can still match.
pub fn settle(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    lines: Vec<AllocationLine>,
) -> Result<()> {
    let lines = checked_lines(lines)?;
    let (all, period) = period(w, snap, month)?;
    require(&period, &[Settling, Settled], revision)?;
    let roles = load_roles(&w.tx)?;
    let missing = crate::closure::unconfirmed_names(&crate::closure::balance_statuses(
        &w.tx, snap, &period, &roles,
    )?);
    if !missing.is_empty() {
        return Err(invalid(format!(
            "confirm every account balance first: {}",
            missing.join(", ")
        )));
    }
    require_no_later_closure(&all, month)?;
    ensure_general_pot(w)?;
    let review = review_month(snap, &all, month, &w.now);
    let lines = client_lines(lines, LineKind::Split);
    let pots = load_pots(&w.tx, true)?;
    let entries = load_pot_entries(&w.tx, None)?;
    validate_allocations(
        &review,
        &lines,
        &goal_ids(snap),
        None,
        &open_pot_balances(&pots, &entries, Some(month)),
    )
    .map_err(invalid)?;
    let lines = with_plan_ids(month, lines, &HashSet::new())?;
    let previous = period.allocations.clone();
    let settled_at = match (&period.status, &period.settled_at) {
        (Settled, Some(at)) => at.clone(),
        _ => stamp(w),
    };
    let saved = save(
        w,
        MonthPeriod {
            status: Settled,
            settled_at: Some(settled_at),
            settled_snapshot: Some(review),
            allocations: with_transfer_targets(lines, &load_roles(&w.tx)?, &pots),
            ..period
        },
    )?;
    record_side_effects(w, &saved, &previous, true)
}

/// Buffer the leftover change since the baseline, clamped like resettle.
/// Line ids never reuse one in `taken`.
fn delta_lines(
    review: &ClosureReview,
    baseline: &ClosureReview,
    prefix: &str,
    taken: &HashSet<String>,
) -> Vec<AllocationLine> {
    let mut used = taken.clone();
    let mut line_id = |account_id: &str| {
        let base = format!("{prefix}-{account_id}");
        let mut candidate = truncated(&base, 64);
        let mut counter = 1;
        while used.contains(&candidate) {
            let suffix = format!("-{counter}");
            candidate = truncated(&base, 64 - suffix.len()) + &suffix;
            counter += 1;
        }
        used.insert(candidate.clone());
        candidate
    };
    let before: HashMap<&str, Decimal> = baseline
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.leftover))
        .collect();
    // Accounts that dropped out of the review (e.g. now ignored) count as 0.
    let mut current: Vec<(&str, Decimal, &finance_types::report::Currency)> = review
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.leftover, &a.currency))
        .collect();
    for a in &baseline.accounts {
        if !current.iter().any(|(id, _, _)| *id == a.account_id) {
            current.push((&a.account_id, Decimal::ZERO, &a.currency));
        }
    }
    let mut lines = Vec::new();
    for (account_id, leftover, currency) in current {
        let change = leftover.max(Decimal::ZERO)
            - before
                .get(account_id)
                .copied()
                .unwrap_or_default()
                .max(Decimal::ZERO);
        if !change.is_zero() {
            lines.push(AllocationLine {
                line_id: line_id(account_id),
                from_account_id: account_id.to_owned(),
                destination: "buffer".into(),
                amount: change,
                currency: currency.clone(),
                kind: LineKind::Delta,
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
            });
        }
    }
    lines
}

/// Close the month: tick off transfers, buffer any change since the split,
/// and set the cutoff (later bookings belong to the next month). Run
/// [`prepare`] first.
pub fn finalize(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    missing_banks: &[String],
) -> Result<()> {
    let missing = check_banks(missing_banks)?;
    let (all, period) = period(w, snap, month)?;
    require(&period, &[Settled], revision)?;
    require_no_later_closure(&all, month)?;
    let (Some(settled_at), Some(baseline)) = (
        period.settled_at.as_deref().and_then(parse_instant),
        period.settled_snapshot.as_ref(),
    ) else {
        return Err(invalid("settled month is missing its snapshot"));
    };
    let review = review_month(snap, &all, month, &w.now);
    let mut allocations = match_transfers(
        &period.allocations,
        &transactions_by_id(&snap.transactions),
        settled_at,
        &snap.zone,
    );
    let taken: HashSet<String> = allocations.iter().map(|l| l.line_id.clone()).collect();
    // Unique per finalize: a reopened month is finalized again.
    allocations.extend(delta_lines(
        &review,
        baseline,
        &format!("delta{}", period.revision),
        &taken,
    ));
    let mut banks: Vec<String> = period
        .missing_banks
        .iter()
        .chain(&missing)
        .cloned()
        .collect();
    banks.sort();
    banks.dedup();
    let previous = period.allocations.clone();
    let saved = save(
        w,
        MonthPeriod {
            status: Closed,
            cutoff_at: Some(stamp(w)),
            settled_snapshot: Some(review),
            allocations,
            missing_banks: banks,
            ..period
        },
    )?;
    // Finalize adds only buffer deltas; planned purchases stay as the owner left them.
    record_side_effects(w, &saved, &previous, false)
}

/// Assign the leftover change of a month that changed after its close.
pub fn resettle(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    lines: Vec<AllocationLine>,
) -> Result<()> {
    let lines = checked_lines(lines)?;
    let (all, period) = period(w, snap, month)?;
    require(&period, &[ChangedAfterClose], revision)?;
    let Some(baseline) = period.settled_snapshot.as_ref() else {
        return Err(invalid("closed month is missing its snapshot"));
    };
    ensure_general_pot(w)?;
    let review = review_month(snap, &all, month, &w.now);
    let lines = client_lines(lines, LineKind::Delta);
    let pots = load_pots(&w.tx, true)?;
    let entries = load_pot_entries(&w.tx, None)?;
    validate_allocations(
        &review,
        &lines,
        &goal_ids(snap),
        Some(baseline),
        // Resettle appends to the month's entries, so takes draw on the full balance.
        &open_pot_balances(&pots, &entries, None),
    )
    .map_err(invalid)?;
    let taken: HashSet<String> = period
        .allocations
        .iter()
        .filter_map(|l| l.plan_id.clone())
        .collect();
    let lines = with_plan_ids(month, lines, &taken)?;
    let marked = with_transfer_targets(lines, &load_roles(&w.tx)?, &pots);
    let previous = period.allocations.clone();
    let mut allocations = period.allocations.clone();
    allocations.extend(marked.iter().cloned());
    let saved = save(
        w,
        MonthPeriod {
            status: Closed,
            settled_snapshot: Some(review),
            allocations,
            reviewed_through: Some(stamp(w)),
            ..period
        },
    )?;
    record_side_effects(w, &saved, &previous, true)?;
    // Buffer lines feed the next month's carry-in; plan lines their due month's.
    let mut affected: HashSet<Date> = marked
        .iter()
        .filter(|l| l.destination == "buffer" && !l.amount.is_zero())
        .map(|_| shift_month(month, 1))
        .collect();
    affected.extend(
        marked
            .iter()
            .filter(|l| l.destination == "plan")
            .filter_map(plan_due_month),
    );
    for later in periods(w)? {
        if later.status != Closed || !day_of(&later.month).is_some_and(|m| affected.contains(&m)) {
            continue;
        }
        try_save(
            w,
            MonthPeriod {
                status: ChangedAfterClose,
                ..later
            },
        )?;
    }
    Ok(())
}

/// Accounts a month's balance check covers, in review order: active, role spending or savings.
pub(crate) fn checked_accounts<'a>(
    snap: &'a Snapshot,
    roles: &[AccountRole],
) -> Vec<&'a crate::model::Account> {
    snap.accounts
        .iter()
        .filter(|a| a.active)
        .filter(|a| {
            roles.iter().any(|r| {
                r.account_id == a.account_id && matches!(r.role.as_str(), "spending" | "savings")
            })
        })
        .collect()
}

/// Confirm what the bank shows for one account: book the difference to the
/// ledger as a reconciliation entry (replacing an earlier confirmation's)
/// and store the check on the month.
pub fn confirm_balance(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    account_id: &str,
    balance: Decimal,
) -> Result<()> {
    let (all, mut period) = period(w, snap, month)?;
    require(&period, &[Open, Settling, Settled], revision)?;
    require_no_later_closure(&all, month)?;
    let roles = load_roles(&w.tx)?;
    let account = checked_accounts(snap, &roles)
        .into_iter()
        .find(|a| a.account_id == account_id)
        .ok_or_else(|| invalid("account cannot be confirmed"))?;
    let currency = account.currency();
    if balance.scale() > u32::from(currency.minor_unit) {
        return Err(invalid("invalid balance"));
    }
    if let Some(earlier) = period
        .balance_checks
        .get(account_id)
        .and_then(|c| c.transaction_id.as_deref())
        .and_then(|id| id.strip_prefix("firefly:"))
        .and_then(|id| id.parse::<i64>().ok())
    {
        // Only ever delete our own reconciliation entry; a stored id that now
        // points at anything else counts as having no entry.
        let kind: Option<String> =
            w.tx.query_row(
                "SELECT kind FROM transactions WHERE id = ?1",
                [earlier],
                |r| r.get(0),
            )
            .optional()?;
        if kind.as_deref() == Some("reconciliation") {
            finance_storage::delete_transaction(&w.tx, earlier)?;
        }
    }
    let description = format!(
        "Balance confirmed at {balance} {} for {} closure",
        currency.code,
        month.strftime("%Y-%m")
    );
    let id = crate::bank_sync::book_reconciliation(
        w,
        account.ledger_id,
        &account.name,
        &currency.code,
        balance,
        &description,
    )?;
    period.balance_checks.insert(
        account_id.to_owned(),
        BalanceCheck {
            balance,
            ledger_balance: balance,
            confirmed_at: stamp(w),
            transaction_id: id.map(|i| format!("firefly:{i}")),
        },
    );
    save(w, period)?;
    Ok(())
}

/// Undo the final close of the latest closed month; its split stays.
pub fn reopen(w: &Writer<'_>, snap: &Snapshot, month: Date, revision: u32) -> Result<()> {
    let (all, period) = period(w, snap, month)?;
    require(&period, &[Closed, ChangedAfterClose], revision)?;
    require_no_later_closure(&all, month)?;
    let due_months: HashSet<Date> = period
        .allocations
        .iter()
        .filter(|l| l.destination == "plan")
        .filter_map(plan_due_month)
        .collect();
    for later in &all {
        let Some(later_month) = day_of(&later.month) else {
            continue;
        };
        if due_months.contains(&later_month) && !matches!(later.status, Open | Settling) {
            return Err(invalid(format!(
                "{} already uses a planned purchase from this month",
                later_month.strftime("%B %Y")
            )));
        }
    }
    let next = shift_month(month, 1).to_string();
    if let Some(following) = all.iter().find(|p| p.month == next)
        && !matches!(following.status, Open | Settling)
    {
        if period
            .allocations
            .iter()
            .any(|l| l.destination == "buffer" && !l.amount.is_zero())
        {
            return Err(invalid("next month already uses this buffer"));
        }
        return Err(invalid("next month is already settled"));
    }
    save(
        w,
        MonthPeriod {
            status: Settled,
            cutoff_at: None,
            reopened_at: Some(stamp(w)),
            ..period
        },
    )?;
    Ok(())
}

fn check_line_id(line_id: &str) -> Result<()> {
    if chars_within(line_id, 64) {
        Ok(())
    } else {
        Err(invalid("invalid line_id"))
    }
}

/// The owner says these bank transfers made a hop (or a direct transfer).
pub fn mark(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    revision: u32,
    line_id: &str,
    hop: Option<u32>,
    transaction_ids: &[String],
) -> Result<()> {
    check_line_id(line_id)?;
    let mut ids: Vec<String> = Vec::new();
    for id in transaction_ids.iter().filter(|i| !i.is_empty()) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    if ids.is_empty() || ids.len() > MAX_MARKED || ids.iter().any(|i| i.chars().count() > 128) {
        return Err(invalid("invalid transaction_ids"));
    }
    let (_, period) = period(w, snap, month)?;
    require(&period, &[Settled, Closed, ChangedAfterClose], revision)?;
    if period.settled_at.is_none() {
        return Err(invalid("month has no split yet"));
    }
    let allocations = mark_transfer(
        &period.allocations,
        line_id,
        hop.map(|h| h as usize),
        &ids,
        &candidates(snap, &period),
    )
    .map_err(invalid)?;
    save(
        w,
        MonthPeriod {
            allocations,
            ..period
        },
    )?;
    Ok(())
}

/// Undo a transfer marked by hand.
pub fn unmark(
    w: &Writer<'_>,
    month: Date,
    revision: u32,
    line_id: &str,
    hop: Option<u32>,
) -> Result<()> {
    check_line_id(line_id)?;
    let all = periods(w)?;
    let text = month.to_string();
    let Some(period) = all.into_iter().find(|p| p.month == text) else {
        return Err(invalid("month is open"));
    };
    require(&period, &[Settled, Closed, ChangedAfterClose], revision)?;
    let allocations =
        unmark_transfer(&period.allocations, line_id, hop.map(|h| h as usize)).map_err(invalid)?;
    save(
        w,
        MonthPeriod {
            allocations,
            ..period
        },
    )?;
    Ok(())
}

/// A fresh pot id (Python used `uuid4().hex`).
fn new_pot_id(now: Timestamp) -> String {
    (0u8..2)
        .map(|part| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u8(part);
            hasher.write_i128(now.as_nanosecond());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// Create or change a pot. Only an empty pot can be archived, and never
/// the general one.
pub fn save_pot(w: &Writer<'_>, snap: &Snapshot, draft: PotDraft) -> Result<PotView> {
    if !chars_within(&draft.name, 128) || !optional_within(&draft.account_id, 128) {
        return Err(invalid("invalid pot"));
    }
    ensure_general_pot(w)?;
    let pots = load_pots(&w.tx, true)?;
    if let Some(account) = &draft.account_id
        && !snap.accounts.iter().any(|a| &a.account_id == account)
    {
        return Err(invalid("unknown account"));
    }
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(invalid("pot name is required"));
    }
    let current = match &draft.pot_id {
        None => PotRecord {
            pot_id: new_pot_id(w.now.timestamp()),
            name: name.to_owned(),
            account_id: None,
            archived: false,
            created_at: stamp(w),
        },
        Some(id) => pots
            .iter()
            .find(|p| &p.pot_id == id)
            .cloned()
            .ok_or_else(|| invalid("unknown pot"))?,
    };
    let balances = pot_balances(&load_pot_entries(&w.tx, None)?, None);
    if draft.archived && !current.archived {
        if current.pot_id == GENERAL_POT_ID {
            return Err(invalid("general savings cannot be archived"));
        }
        let own = balances.get(&current.pot_id);
        if own.is_some_and(|b| b.values().any(|v| !v.is_zero())) {
            return Err(invalid("only an empty pot can be archived"));
        }
    }
    let saved = PotRecord {
        name: name.to_owned(),
        account_id: draft.account_id,
        archived: draft.archived,
        ..current
    };
    w.put_json(
        "INSERT INTO pots(pot_id, payload_json) VALUES (?2, ?1) \
         ON CONFLICT(pot_id) DO UPDATE SET payload_json = excluded.payload_json",
        &saved,
        &[&saved.pot_id],
    )?;
    Ok(pot_view(&saved, &balances))
}

/// Apply a changed via route to transfers that have not been made yet.
pub fn reroute_open_transfers(w: &Writer<'_>) -> Result<()> {
    let roles = load_roles(&w.tx)?;
    for period in periods(w)? {
        if !matches!(period.status, Settled | Closed | ChangedAfterClose) {
            continue;
        }
        let allocations = reroute_pending(&period.allocations, &roles);
        if allocations == period.allocations {
            continue;
        }
        // A conflict means a screen write won; its next save re-reads the roles.
        try_save(
            w,
            MonthPeriod {
                allocations,
                ..period
            },
        )?;
    }
    Ok(())
}

/// Save an account's closure role (and route), then re-plan the transfers
/// not made yet.
pub fn save_account_role(w: &Writer<'_>, snap: &Snapshot, draft: AccountRoleDraft) -> Result<()> {
    let accounts: HashSet<&str> = snap
        .accounts
        .iter()
        .map(|a| a.account_id.as_str())
        .collect();
    if draft.account_id.chars().count() > 128 || !accounts.contains(draft.account_id.as_str()) {
        return Err(invalid("unknown account"));
    }
    let goal = draft.goal_id.filter(|g| !g.is_empty());
    if let Some(goal) = &goal
        && !snap.goals.iter().any(|g| &g.goal.goal_id == goal)
    {
        return Err(invalid("unknown goal"));
    }
    let roles = load_roles(&w.tx)?;
    if let Some(goal) = &goal
        && roles.iter().any(|r| {
            r.goal_id.as_ref() == Some(goal)
                && r.role == "savings"
                && r.account_id != draft.account_id
        })
    {
        return Err(invalid("goal already linked to another savings account"));
    }
    let saved_role = roles.iter().find(|r| r.account_id == draft.account_id);
    // Left out, the saved route stays; null clears it.
    let via = match draft.via_account_id {
        Some(via) => via,
        None => saved_role.and_then(|r| r.via_account_id.clone()),
    };
    if let Some(via) = &via {
        if via.chars().count() > 128 || *via == draft.account_id || !accounts.contains(via.as_str())
        {
            return Err(invalid("invalid via account"));
        }
        // One hop at most: the via account is reached directly, and nothing
        // already routes through this account.
        if roles
            .iter()
            .any(|r| &r.account_id == via && r.via_account_id.is_some())
        {
            return Err(invalid("via account is itself reached via another"));
        }
        if roles
            .iter()
            .any(|r| r.via_account_id.as_ref() == Some(&draft.account_id))
        {
            return Err(invalid("other accounts are reached via this one"));
        }
    }
    let known_role = matches!(draft.role.as_str(), "spending" | "savings" | "ignored");
    if !known_role || (goal.is_some() && draft.role != "savings") {
        return Err(invalid("invalid role"));
    }
    let role = AccountRole {
        account_id: draft.account_id,
        role: draft.role,
        goal_id: goal,
        via_account_id: via,
        confirmed_at: stamp(w),
    };
    w.put_json(
        "INSERT INTO account_roles(account_id, payload_json) VALUES (?2, ?1) \
         ON CONFLICT(account_id) DO UPDATE SET payload_json = excluded.payload_json",
        &role,
        &[&role.account_id],
    )?;
    // A new route applies at once to transfers not made yet.
    reroute_open_transfers(w)
}

#[cfg(test)]
mod tests;
