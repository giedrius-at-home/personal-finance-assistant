//! Planned purchases and whether the budget can afford them.

pub mod baseline;
pub mod goals;
pub mod ladder;
pub mod schedule;
pub mod store;

use finance_types::gateway::{
    BudgetPlan, Repeat, Wish, WishKind, WishPurchase, WishSettings, WishSource, WishStatus,
};
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::wishes::{
    ChangeableBill, Cut, EventOption, GoalOutlook, GoalStatus, Verdict, WishMonth, WishRow,
    WishesView,
};
use finance_types::report::{Currency, Dashboard};
use jiff::civil::Date;
use rust_decimal::Decimal;
use schedule::{BillNow, Family, Planned, Saver};

use crate::events::{EventNeed, GroupUsual};
use finance_types::report::adjustments::Adjustments;
use finance_types::report::events::{EventPhase, EventRow, EventSpend, EventsView};
use std::collections::HashMap;

use crate::load::Bill;

/// The wishes screen, judged against the dashboard's allocation. `bills`
/// are the snapshot's, for the schedules the dashboard rows leave out.
pub fn wishes_view(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    wishes: &[Wish],
    settings: &WishSettings,
) -> WishesView {
    let (plans, card_bills, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        today: today(dashboard),
        plans,
        card_bills,
        bills: &bills,
        habits: goals::habit_rows(
            &dashboard.adjustments,
            &allocation(dashboard).map_or_else(Currency::eur, |a| a.currency.clone()),
            &settings.essential_groups,
        ),
        events: event_needs(dashboard, wishes),
    }
    .view(wishes, settings)
}

/// What the plan sets aside per wish over its months, in the order the
/// money reaches them: `(name, total)`.
pub fn set_aside_order(months: &[WishMonth], wishes: &[Wish]) -> Vec<(String, Decimal)> {
    let mut out: Vec<(String, Decimal)> = Vec::new();
    for v in months.iter().flat_map(|m| &m.verdicts) {
        if !v.set_aside || v.skipped || v.amount <= Decimal::ZERO {
            continue;
        }
        let name = v
            .wish_id
            .as_deref()
            .and_then(|id| wishes.iter().find(|w| w.wish_id == id))
            .map_or(v.label.as_str(), |w| w.name.as_str());
        match out.iter_mut().find(|(n, _)| n == name) {
            Some((_, total)) => *total += v.amount,
            None => out.push((name.to_owned(), v.amount)),
        }
    }
    out
}

/// What each event still asks of the months, from the dashboard's ledger
/// reading; empty without an allocation to judge in.
pub fn event_needs(dashboard: &Dashboard, wishes: &[Wish]) -> Vec<EventNeed> {
    allocation(dashboard).map_or_else(Vec::new, |alloc| {
        event_needs_for(
            alloc,
            &dashboard.adjustments,
            &dashboard.event_spends,
            wishes,
            today(dashboard),
        )
    })
}

/// The budgeted groups as an event's later months see them.
fn group_usuals(adjustments: &[Adjustments], currency: &Currency) -> Vec<GroupUsual> {
    adjustments
        .iter()
        .filter(|a| a.currency == *currency)
        .flat_map(|a| &a.rows)
        .filter(|row| row.budgeted)
        .filter_map(|row| {
            Some(GroupUsual {
                usual: goals::usual(&row.row.history)?,
                limit: row.row.planned,
            })
        })
        .collect()
}

/// [`event_needs`] from the dashboard's parts, for while it is being built.
pub fn event_needs_for(
    alloc: &CurrencyAllocation,
    adjustments: &[Adjustments],
    spends: &[EventSpend],
    wishes: &[Wish],
    today: Option<Date>,
) -> Vec<EventNeed> {
    let Some(month0) = baseline::current_month(alloc) else {
        return Vec::new();
    };
    let groups = group_usuals(adjustments, &alloc.currency);
    crate::events::claims(wishes)
        .iter()
        .filter(|c| c.currency == alloc.currency)
        .map(|claim| {
            let spend = spends.iter().find(|s| s.wish_id == claim.wish_id);
            crate::events::need(claim, spend, &groups, month0, today.unwrap_or(month0))
        })
        .collect()
}

/// The largest amount the event could plan without a trade-off: its amount
/// plus the least free money any month up to its last is left with (money
/// is saved for it ahead, so an earlier month counts too), or, when it already
/// costs something, its amount less what cuts, savings, cards and a short
/// had to find. `None` when it was not judged.
pub fn fits_up_to(row: &WishRow, months: &[WishMonth], event_months: &[Date]) -> Option<Decimal> {
    if row.not_judged.is_some() {
        return None;
    }
    let id = row.wish.wish_id.as_str();
    if row.short == Decimal::ZERO && !row.tradeoffs {
        let spare = months
            .iter()
            .filter(|m| {
                m.month
                    .parse::<Date>()
                    .is_ok_and(|month| event_months.last().is_some_and(|last| month <= *last))
            })
            .map(|m| m.free_left)
            .min()
            .unwrap_or_default();
        return Some(row.wish.amount + spare.max(Decimal::ZERO));
    }
    let found: Decimal = months
        .iter()
        .flat_map(|m| &m.verdicts)
        .filter(|v| v.wish_id.as_deref() == Some(id) && !v.set_aside)
        .map(|v| {
            let cuts: Decimal = v.cuts.iter().map(|c| c.from - c.to).sum();
            cuts + v.from_savings + v.on_card + v.short
        })
        .sum();
    Some((row.wish.amount - found).max(Decimal::ZERO))
}

/// The events screen, judged with the wishes against the dashboard.
pub fn events_view(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    wishes: &[Wish],
    settings: &WishSettings,
) -> EventsView {
    let (plans, card_bills, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    let alloc = allocation(dashboard);
    let currency = alloc.map_or_else(Currency::eur, |a| a.currency.clone());
    let needs = event_needs(dashboard, wishes);
    let (rows, months) = Inputs {
        alloc,
        today: today(dashboard),
        plans,
        card_bills,
        bills: &bills,
        habits: Vec::new(),
        events: needs.clone(),
    }
    .event_rows(wishes, settings);
    let claims = crate::events::claims(wishes);
    let month0 = alloc.and_then(baseline::current_month);
    let day = today(dashboard).or(month0);
    let groups = alloc.map_or_else(Vec::new, |a| {
        group_usuals(&dashboard.adjustments, &a.currency)
    });
    let mut events: Vec<EventRow> = rows
        .into_iter()
        .filter_map(|mut row| {
            let claim = claims.iter().find(|c| c.wish_id == row.wish.wish_id)?;
            let spend = dashboard
                .event_spends
                .iter()
                .find(|s| s.wish_id == claim.wish_id);
            // What the running month's groups handed over is on the budget's
            // event row; later months are estimated from the groups' usual.
            let money = crate::events::days_money(claim, spend);
            let usual: Vec<(Date, Decimal)> = crate::events::shares(claim, money)
                .into_iter()
                .map(|(month, share)| {
                    let handed = if Some(month) == month0 {
                        dashboard
                            .monthly_budgets
                            .iter()
                            .filter(|b| b.plan.currency == claim.currency)
                            .filter(|b| b.plan.month.parse::<Date>().ok() == Some(month))
                            .flat_map(|b| &b.events)
                            .filter(|e| e.id == claim.key)
                            .flat_map(|e| &e.expected)
                            .map(|(_, n)| *n)
                            .sum()
                    } else if month0.is_some_and(|m| month > m) {
                        crate::events::usual_estimate(
                            &groups,
                            month,
                            crate::events::days_in(claim, month),
                            share,
                        )
                    } else {
                        Decimal::ZERO
                    };
                    (month, handed)
                })
                .collect();
            let bought: Decimal = row
                .children
                .iter()
                .filter(|c| c.wish.status == WishStatus::Bought)
                .map(|c| c.wish.bought.as_ref().map_or(c.wish.amount, |b| b.amount))
                .sum();
            let facts = crate::events::facts(claim, spend, bought, &usual, day?);
            let asks_nothing = needs
                .iter()
                .find(|n| n.wish_id == claim.wish_id)
                .is_some_and(|n| n.months.iter().all(|(_, need)| need.is_zero()));
            if facts.phase == EventPhase::Past || asks_nothing {
                row.not_judged = None;
            }
            let event_months = crate::events::months(claim);
            Some(EventRow {
                fits_up_to: fits_up_to(&row, &months, &event_months),
                row,
                phase: facts.phase,
                usual: facts.usual,
                extra: facts.extra,
                paid_ahead: facts.paid_ahead,
                spent: facts.spent,
                left: facts.left,
                days: facts.days,
                days_left: facts.days_left,
                per_day: facts.per_day,
                pace: facts.pace,
                months: facts.months,
                transactions: spend.map(|s| s.transactions.clone()).unwrap_or_default(),
            })
        })
        .collect();
    // Running first, then upcoming by start, then past, the newest first.
    let starts = |e: &EventRow| {
        e.row
            .wish
            .event
            .as_ref()
            .map(|d| d.starts_on.clone())
            .unwrap_or_default()
    };
    let rank = |e: &EventRow| match e.phase {
        EventPhase::Running => 0,
        EventPhase::Upcoming => 1,
        EventPhase::Past => 2,
    };
    events.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| match a.phase {
            EventPhase::Past => starts(b).cmp(&starts(a)),
            _ => starts(a).cmp(&starts(b)),
        })
    });
    EventsView {
        currency,
        events,
        months,
    }
}

/// The cuts buying `wish_id` with `purchase` needs in month 0: the unsaved
/// gap judged as a purchase of its own, after everything else planned.
pub fn purchase_cuts(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    wishes: &[Wish],
    settings: &WishSettings,
    wish_id: &str,
    purchase: &WishPurchase,
) -> Vec<Cut> {
    let (plans, card_bills, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        today: today(dashboard),
        plans,
        card_bills,
        bills: &bills,
        habits: Vec::new(),
        events: event_needs(dashboard, wishes),
    }
    .purchase_cuts(wishes, settings, wish_id, purchase)
}

/// The plans the judging reads: the dashboard's months (stored or default)
/// plus every other stored plan, so a default month inherits the ranking
/// of the latest stored plan that ranks a group.
pub fn view_plans(dashboard: Vec<BudgetPlan>, stored: &[BudgetPlan]) -> Vec<BudgetPlan> {
    let mut plans = dashboard;
    for plan in stored {
        if !plans
            .iter()
            .any(|p| p.month == plan.month && p.currency == plan.currency)
        {
            plans.push(plan.clone());
        }
    }
    plans
}

/// The owner's day the dashboard was built on.
fn today(dashboard: &Dashboard) -> Option<Date> {
    dashboard.generated_at.get(..10)?.parse().ok()
}

fn allocation(dashboard: &Dashboard) -> Option<&CurrencyAllocation> {
    dashboard.monthly_allocation.as_ref().and_then(|m| {
        m.currencies
            .iter()
            .find(|c| c.currency.code == "EUR")
            .or_else(|| m.currencies.first())
    })
}

/// Plans, each card's linked bill amount, and the bills the judging knows.
type DashboardParts = (
    Vec<BudgetPlan>,
    Vec<(String, Decimal)>,
    HashMap<String, BillNow>,
);

/// The budget plans, each card's bill amount and the bills as they stand.
fn dashboard_parts(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    settings: &WishSettings,
) -> DashboardParts {
    let plans = view_plans(
        dashboard
            .monthly_budgets
            .iter()
            .map(|b| b.plan.clone())
            .collect(),
        stored_plans,
    );
    // Each card's bill is already counted in next month's fixed costs.
    let rows = &dashboard
        .next_month_subscriptions
        .as_ref()
        .unwrap_or(&dashboard.subscriptions)
        .rows;
    let card_bills = settings
        .cards
        .iter()
        .filter_map(|card| {
            let bill_id = card.bill_id.as_deref()?;
            let row = rows.iter().find(|row| row.bill_id == bill_id)?;
            Some((card.card_id.clone(), row.amount_max))
        })
        .collect();
    (plans, card_bills, bills_now(dashboard, bills))
}

/// The schedule the judging knows a bill by (Firefly's frequency and skip);
/// `None` when it cannot judge it.
pub fn repeat_of(frequency: &str, skip: u32) -> Option<Repeat> {
    match (frequency.to_lowercase().as_str(), skip) {
        ("weekly", 0) => Some(Repeat::Weekly),
        ("weekly", 1) => Some(Repeat::Biweekly),
        ("monthly", 0) => Some(Repeat::Monthly),
        ("yearly", 0) => Some(Repeat::Yearly),
        _ => None,
    }
}

/// Every bill the judging knows the schedule of, at its highest amount.
fn bills_now(dashboard: &Dashboard, bills: &[Bill]) -> HashMap<String, BillNow> {
    dashboard
        .next_month_subscriptions
        .as_ref()
        .unwrap_or(&dashboard.subscriptions)
        .rows
        .iter()
        .filter_map(|row| {
            let skip = bills
                .iter()
                .find(|b| b.id == row.bill_id)
                .map_or(0, |b| b.skip);
            let every = repeat_of(&row.frequency, skip)?;
            let bill = BillNow {
                amount: row.amount_max,
                every,
            };
            Some((row.bill_id.clone(), bill))
        })
        .collect()
}

/// What the judging reads besides the wishes.
#[derive(Debug)]
pub struct Inputs<'a> {
    pub alloc: Option<&'a CurrencyAllocation>,
    /// The day judged on, for which card statement is open; `None` reads
    /// the first of month 0, so month 0's statement is open and usage typed
    /// for the next statement counts as stale.
    pub today: Option<Date>,
    pub plans: Vec<BudgetPlan>,
    pub card_bills: Vec<(String, Decimal)>,
    pub bills: &'a HashMap<String, BillNow>,
    /// The spending groups at their typical month; empty leaves the goals
    /// unjudged against habits.
    pub habits: Vec<goals::HabitRow>,
    /// What each event still asks, month by month; an event without an
    /// entry is judged as the plain wish it also is.
    pub events: Vec<EventNeed>,
}

impl Inputs<'_> {
    /// Each month's starting position on the plan, and the stale cards.
    fn bases(
        &self,
        alloc: &CurrencyAllocation,
        month0: Date,
        wishes: &[Wish],
        settings: &WishSettings,
        months: usize,
    ) -> (Vec<ladder::MonthBaseline>, Vec<String>) {
        baseline::baselines(
            alloc,
            self.today.unwrap_or(month0),
            &self.plans,
            &self.card_bills,
            settings,
            schedule::earmarked_with(wishes, &self.events),
            months,
        )
    }

    /// Run the ladder over `bases`.
    fn run(
        &self,
        bases: &[ladder::MonthBaseline],
        month0: Date,
        items: &[Planned],
    ) -> Vec<WishMonth> {
        let today = self.today.unwrap_or(month0);
        let months = bases.len();
        let mut saver = Saver::new(items, month0, months);
        let mut repeats = schedule::repeat_demands(items, month0, months);
        // A closed statement takes no more card costs: the repeats that fell
        // on it are already in its usage (or the bill).
        if schedule::statement_month(today) > month0
            && let Some(first) = repeats.first_mut()
        {
            first.retain(|d| !d.on_card);
        }
        let mut judged = ladder::judge(bases, &repeats, &mut saver);
        // An event's month shares are verdicts of the event itself.
        for verdict in judged.iter_mut().flat_map(|m| &mut m.verdicts) {
            if let Some(id) = &mut verdict.wish_id {
                id.truncate(schedule::base_id(id).len());
            }
        }
        judged
    }

    /// Judge `items` from month 0 on.
    fn judge(
        &self,
        alloc: &CurrencyAllocation,
        month0: Date,
        items: &[Planned],
        wishes: &[Wish],
        settings: &WishSettings,
    ) -> (Vec<WishMonth>, Vec<String>) {
        let months = schedule::horizon(items, month0);
        let (bases, stale_cards) = self.bases(alloc, month0, wishes, settings, months);
        (self.run(&bases, month0, items), stale_cards)
    }

    /// The same wishes judged with spending as usual, and the emergency
    /// fund that takes what they leave. `plan_rows` are the view's rows.
    fn goals(
        &self,
        wishes: &[Wish],
        settings: &WishSettings,
        plan_rows: &[WishRow],
    ) -> Option<GoalOutlook> {
        let alloc = self.alloc.filter(|_| !self.habits.is_empty())?;
        let month0 = baseline::current_month(alloc)?;
        let (mut items, families) =
            schedule::plan_items_with(wishes, month0, self.bills, &self.events);
        self.open_statement(&mut items, month0);
        let months = schedule::horizon(&items, month0);
        let (plan_bases, _) = self.bases(alloc, month0, wishes, settings, months);
        let habits = goals::with_next_budgets(&self.habits, alloc);
        let habit_bases = goals::habit_baselines(&plan_bases, &habits);
        let habit_months = self.run(&habit_bases, month0, &items);
        let ctx = RowContext {
            wishes,
            months: &habit_months,
            families: &families,
            bills: self.bills,
        };
        let mut tops: Vec<&Wish> = wishes
            .iter()
            .filter(|w| w.parent_id.is_none() && w.status == WishStatus::Planned)
            .collect();
        sort_by_due(&mut tops);
        let statuses = tops
            .into_iter()
            .map(|w| {
                // A purchase the cards cover is paid; what the charge then
                // lacks belongs to the month the card is charged.
                let card_short = habit_months
                    .iter()
                    .flat_map(|m| &m.verdicts)
                    .filter(|v| {
                        v.wish_id.as_deref().is_some_and(|id| {
                            id == w.wish_id
                                || wishes.iter().any(|c| {
                                    c.wish_id == id && c.parent_id.as_deref() == Some(&w.wish_id)
                                })
                        })
                    })
                    .map(|v| v.next_short)
                    .sum::<Decimal>()
                    .round_dp(2);
                let short = ctx.row(w).short.round_dp(2);
                let card_short = card_short.min(short);
                GoalStatus {
                    wish_id: w.wish_id.clone(),
                    name: w.name.clone(),
                    due_on: w.due_on.clone(),
                    amount: w.amount,
                    must: w.must,
                    short: short - card_short,
                    card_short,
                    on_card: habit_months
                        .iter()
                        .flat_map(|m| &m.verdicts)
                        .filter(|v| {
                            // Only what lands on a card the owner did not choose.
                            v.wish_id.as_deref().is_some_and(|id| {
                                wishes.iter().any(|c| {
                                    c.wish_id == id
                                        && c.source != WishSource::Card
                                        && (id == w.wish_id
                                            || c.parent_id.as_deref() == Some(&w.wish_id))
                                })
                            })
                        })
                        .map(|v| v.on_card)
                        .sum::<Decimal>()
                        .round_dp(2),
                    plan_short: plan_rows
                        .iter()
                        .find(|r| r.wish.wish_id == w.wish_id)
                        .map_or(Decimal::ZERO, |r| r.short),
                }
            })
            .collect();
        // Each later purchase spread over the months left until it is due,
        // plus the planned repeating costs.
        let ongoing = goals::ongoing_monthly(&items);
        // The savings pot pays the soonest purchases first; only what it
        // cannot cover is asked of the coming months.
        // (The pot as this month's own purchases leave it; this month's
        // set-asides are savings for those same later purchases.)
        let mut pot = habit_months.first().map_or(Decimal::ZERO, |m| {
            let spent: Decimal = m
                .verdicts
                .iter()
                .filter(|v| !v.set_aside)
                .map(|v| v.from_savings)
                .sum();
            (m.savings_before - spent).max(Decimal::ZERO)
        });
        let mut later: Vec<&Planned> = items
            .iter()
            .filter(|i| i.per_month.is_none() && i.remaining > Decimal::ZERO)
            // What is due this month is this month's business.
            .filter(|i| i.months_from(month0) >= 1)
            .collect();
        later.sort_by_key(|i| i.due);
        let purchases: Decimal = later
            .into_iter()
            .map(|i| {
                let from_pot = pot.min(i.remaining);
                pot -= from_pot;
                (i.remaining - from_pot) / Decimal::from(i.months_from(month0))
            })
            .sum();
        let needed_monthly = (purchases + goals::repeating_monthly(&items))
            .max(Decimal::ZERO)
            .round_dp(2);
        let plan_free = plan_bases.get(1)?.free.round_dp(2);
        let habit_free = habit_bases.get(1)?.free.round_dp(2);
        let coverage = (needed_monthly > Decimal::ZERO).then(|| {
            let share = (habit_free.max(Decimal::ZERO) / needed_monthly * Decimal::ONE_HUNDRED)
                .floor()
                .min(Decimal::ONE_HUNDRED);
            u32::try_from(share.mantissa()).unwrap_or(0)
        });
        let steady = habit_free - ongoing;
        let gap = (needed_monthly - habit_free).max(Decimal::ZERO);
        let (cuts, uncovered) = goals::advice(gap, &habits, &self.plans);
        Some(GoalOutlook {
            needed_monthly,
            habit_free,
            plan_free,
            gap,
            coverage,
            horizon_months: habit_months.len(),
            goals: statuses,
            cuts,
            uncovered,
            fund: goals::fund(
                settings.emergency_months,
                goals::fixed_monthly(
                    alloc,
                    &baseline::counted_bills(&self.card_bills, settings, month0),
                    &items,
                ),
                &habits,
                &habit_months,
                steady,
            ),
        })
    }

    /// Card one-offs due before today go on the statement open today: a
    /// closed statement takes nothing more.
    fn open_statement(&self, items: &mut [Planned], month0: Date) {
        let today = self.today.unwrap_or(month0);
        for item in items
            .iter_mut()
            .filter(|i| i.on_card && i.per_month.is_none())
        {
            item.due = item.due.max(today);
        }
    }

    /// The wishes screen: the events are on their own screen.
    pub fn view(&self, wishes: &[Wish], settings: &WishSettings) -> WishesView {
        let mut view = self.view_all(wishes, settings);
        view.events = view
            .wishes
            .iter()
            .map(|row| &row.wish)
            .filter(|w| w.status == WishStatus::Planned)
            .filter_map(|w| {
                let ends = w.event.as_ref()?.ends_on.parse::<Date>().ok()?;
                Some(EventOption {
                    wish_id: w.wish_id.clone(),
                    name: w.name.clone(),
                    currency: w.currency.clone(),
                    over: self.today.is_some_and(|today| ends < today),
                })
            })
            .collect();
        view.wishes.retain(|row| row.wish.event.is_none());
        view
    }

    /// The events' rows, soonest first, and the months they were judged in.
    pub fn event_rows(
        &self,
        wishes: &[Wish],
        settings: &WishSettings,
    ) -> (Vec<WishRow>, Vec<WishMonth>) {
        let mut view = self.view_all(wishes, settings);
        view.wishes.retain(|row| row.wish.event.is_some());
        (view.wishes, view.months)
    }

    /// Every top-level wish, events included, judged together.
    fn view_all(&self, wishes: &[Wish], settings: &WishSettings) -> WishesView {
        let judged = self.alloc.and_then(|a| {
            let month0 = baseline::current_month(a)?;
            let (mut items, families) =
                schedule::plan_items_with(wishes, month0, self.bills, &self.events);
            self.open_statement(&mut items, month0);
            let (judged, stale) = self.judge(a, month0, &items, wishes, settings);
            Some((judged, families, stale))
        });
        let currency = self.alloc.map_or_else(
            || {
                wishes
                    .first()
                    .map_or_else(Currency::eur, |w| w.currency.clone())
            },
            |a| a.currency.clone(),
        );
        let (months, families, stale_cards) = judged.unwrap_or_default();
        let ctx = RowContext {
            wishes,
            months: &months,
            families: &families,
            bills: self.bills,
        };
        let mut tops: Vec<&Wish> = wishes.iter().filter(|w| w.parent_id.is_none()).collect();
        sort_by_due(&mut tops);
        let mut changeable_bills: Vec<ChangeableBill> = self
            .bills
            .iter()
            .map(|(id, bill)| ChangeableBill {
                bill_id: id.clone(),
                every: bill.every,
            })
            .collect();
        changeable_bills.sort_by(|a, b| a.bill_id.cmp(&b.bill_id));
        let must_keep = crate::closure::plan::wishes_must_keep(wishes, &currency.code)
            .iter()
            .map(|(_, v)| *v)
            .sum();
        let rows: Vec<WishRow> = tops.into_iter().map(|w| ctx.row(w)).collect();
        WishesView {
            goals: self.goals(wishes, settings, &rows),
            currency,
            settings: settings.clone(),
            card_stale: !stale_cards.is_empty(),
            stale_cards,
            wishes: rows,
            months,
            changeable_bills,
            savings_must_keep: must_keep,
            events: Vec::new(),
        }
    }

    /// See [`purchase_cuts`].
    pub fn purchase_cuts(
        &self,
        wishes: &[Wish],
        settings: &WishSettings,
        wish_id: &str,
        purchase: &WishPurchase,
    ) -> Vec<Cut> {
        let Some(alloc) = self.alloc else {
            return Vec::new();
        };
        let (Some(month0), Some(wish)) = (
            baseline::current_month(alloc),
            wishes.iter().find(|w| w.wish_id == wish_id),
        ) else {
            return Vec::new();
        };
        let gap = purchase.amount - wish.saved;
        if gap <= Decimal::ZERO {
            return Vec::new();
        }
        let (mut items, _) = schedule::plan_items_with(wishes, month0, self.bills, &self.events);
        items.retain(|i| i.wish_id != wish_id);
        let due = purchase.on.parse::<Date>().unwrap_or(month0).max(month0);
        items.push(Planned {
            wish_id: wish_id.to_owned(),
            label: wish.name.clone(),
            due,
            remaining: gap,
            on_card: purchase.source == WishSource::Card,
            created_at: wish.created_at.clone(),
            per_month: None,
            // Already bought: month 0 sets nothing aside for it, even when
            // the card statement makes month 1 pay.
            saved_this_month: true,
            // A sub-wish of a must wish is a must too, as the view judges it.
            must: wish.must
                || wishes
                    .iter()
                    .any(|p| Some(&p.wish_id) == wish.parent_id.as_ref() && p.must),
        });
        self.open_statement(&mut items, month0);
        let landed = items
            .iter()
            .find(|i| i.wish_id == wish_id)
            .map_or(0, |i| i.months_from(month0));
        let (judged, _) = self.judge(alloc, month0, &items, wishes, settings);
        judged
            .get(landed)
            .map(|m| {
                merge_cuts(
                    m.verdicts
                        .iter()
                        .filter(|v| v.wish_id.as_deref() == Some(wish_id) && !v.set_aside),
                )
            })
            .unwrap_or_default()
    }
}

/// The wishes screen, judged against one currency's allocation.
pub fn view_from(
    alloc: Option<&CurrencyAllocation>,
    plans: &[BudgetPlan],
    card_bills: Vec<(String, Decimal)>,
    wishes: &[Wish],
    settings: &WishSettings,
    bills: &HashMap<String, BillNow>,
) -> WishesView {
    Inputs {
        alloc,
        today: None,
        plans: plans.to_vec(),
        card_bills,
        bills,
        habits: Vec::new(),
        events: Vec::new(),
    }
    .view(wishes, settings)
}

fn sort_by_due(wishes: &mut [&Wish]) {
    wishes.sort_by(|a, b| (&a.due_on, &a.created_at).cmp(&(&b.due_on, &b.created_at)));
}

struct RowContext<'a> {
    wishes: &'a [Wish],
    months: &'a [WishMonth],
    families: &'a [Family],
    bills: &'a HashMap<String, BillNow>,
}

impl RowContext<'_> {
    fn verdicts<'v>(&'v self, wish_id: &'v str) -> impl Iterator<Item = &'v Verdict> {
        self.months
            .iter()
            .flat_map(|m| &m.verdicts)
            .filter(move |v| v.wish_id.as_deref() == Some(wish_id))
    }

    /// What month 0 sets aside for `wish_id` (a skipped month sets nothing).
    fn set_aside(&self, wish_id: &str) -> Decimal {
        self.months
            .first()
            .into_iter()
            .flat_map(|m| &m.verdicts)
            .filter(|v| v.wish_id.as_deref() == Some(wish_id) && v.set_aside && !v.skipped)
            .map(|v| v.amount)
            .sum()
    }

    /// Why a planned wish with money still to find has no verdicts.
    fn why_unjudged(
        &self,
        wish: &Wish,
        children: &[WishRow],
        family: Option<&Family>,
    ) -> Option<&'static str> {
        let ids: Vec<&str> = std::iter::once(wish.wish_id.as_str())
            .chain(children.iter().map(|c| c.wish.wish_id.as_str()))
            .collect();
        let judged = self
            .months
            .iter()
            .flat_map(|m| &m.verdicts)
            .any(|v| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id)));
        if judged {
            return None;
        }
        let unknown_bill = match &wish.kind {
            WishKind::BillChange { bill_id, .. } => match self.bills.get(bill_id) {
                Some(bill) if bill.amount == wish.amount => return None,
                Some(_) => false,
                None => true,
            },
            WishKind::Recurring { .. } => false,
            WishKind::OneOff => {
                let own = family.map_or(wish.amount, |f| f.rest) - wish.saved;
                let parts: Decimal = children
                    .iter()
                    .filter(|c| c.wish.status == WishStatus::Planned)
                    .map(|c| (c.wish.amount - c.wish.saved).max(Decimal::ZERO))
                    .sum();
                if own.max(Decimal::ZERO) + parts <= Decimal::ZERO {
                    return None;
                }
                false
            }
        };
        if self.months.is_empty() {
            return Some("no budget data");
        }
        if unknown_bill {
            return Some("bill can't be judged");
        }
        let last = self.months.last()?.month.parse::<Date>().ok()?;
        let due = wish.due_on.parse::<Date>().ok()?;
        (due.first_of_month() > last).then_some("too far ahead")
    }

    fn row(&self, wish: &Wish) -> WishRow {
        let mut subs: Vec<&Wish> = self
            .wishes
            .iter()
            .filter(|w| w.parent_id.as_deref() == Some(wish.wish_id.as_str()))
            .collect();
        sort_by_due(&mut subs);
        let children: Vec<WishRow> = subs.into_iter().map(|w| self.row(w)).collect();
        let family = self.families.iter().find(|f| f.parent_id == wish.wish_id);
        let (mut short, mut tradeoffs, mut monthly_set_aside) =
            (Decimal::ZERO, false, Decimal::ZERO);
        let one_off = wish.kind == WishKind::OneOff;
        if wish.status == WishStatus::Planned {
            for v in self.verdicts(&wish.wish_id) {
                short += v.short + v.next_short;
                tradeoffs |= !v.cuts.is_empty()
                    || v.on_card > Decimal::ZERO
                    || v.from_savings > Decimal::ZERO;
            }
            if one_off {
                monthly_set_aside = self.set_aside(&wish.wish_id);
            }
            for child in &children {
                short += child.short;
                tradeoffs |= child.tradeoffs;
                monthly_set_aside += child.monthly_set_aside;
            }
        }
        let not_judged = (wish.status == WishStatus::Planned)
            .then(|| self.why_unjudged(wish, &children, family))
            .flatten()
            .map(str::to_owned);
        // A parent's set-aside is its sub-wishes'; they carry the suggestion.
        let own_set_aside = if one_off && children.is_empty() {
            monthly_set_aside
        } else {
            Decimal::ZERO
        };
        let recorded_this_month = self
            .months
            .first()
            .is_some_and(|m| wish.saved_month.as_deref() == Some(m.month.as_str()));
        let suggested_saved = if recorded_this_month {
            wish.saved
        } else {
            wish.saved + own_set_aside
        };
        WishRow {
            wish: wish.clone(),
            children,
            rest: family.map_or(Decimal::ZERO, |f| f.rest),
            over_cap: family.is_some_and(|f| f.over_cap),
            monthly_set_aside,
            suggested_saved,
            short,
            tradeoffs,
            not_judged,
        }
    }
}

/// One cut per group, in first-cut order, keeping the first `from` and the
/// lowest `to`.
fn merge_cuts<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for cut in verdicts.flat_map(|v| &v.cuts) {
        match out.iter_mut().find(|c| c.group_id == cut.group_id) {
            Some(seen) => seen.to = seen.to.min(cut.to),
            None => out.push(cut.clone()),
        }
    }
    out
}

/// The cuts month 0 makes for one wish's own demands (not its set-asides).
pub fn month_zero_cuts(view: &WishesView, wish_id: &str) -> Vec<Cut> {
    view.months
        .first()
        .map(|month| {
            merge_cuts(
                month
                    .verdicts
                    .iter()
                    .filter(|v| v.wish_id.as_deref() == Some(wish_id) && !v.set_aside),
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::baseline::tests::{alloc, d, plan, plan_group, row};
    use super::*;
    use finance_types::gateway::{CardSettings, WishPurchase, WishSource, WishStatus};
    use finance_types::report::Currency;
    use jiff::civil::date;

    #[test]
    fn set_asides_are_totalled_per_wish_in_the_order_money_reaches_them() {
        let aside = |id: &str, amount: &str, set_aside: bool, skipped: bool| Verdict {
            wish_id: Some(id.into()),
            label: format!("label {id}"),
            amount: d(amount),
            from_free: Decimal::ZERO,
            from_savings: Decimal::ZERO,
            cuts: Vec::new(),
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
            set_aside,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
            must: false,
            skipped,
        };
        let month = |verdicts: Vec<Verdict>| WishMonth {
            month: String::new(),
            free_before: Decimal::ZERO,
            free_left: Decimal::ZERO,
            savings_before: Decimal::ZERO,
            savings_left: Decimal::ZERO,
            card_charge_in: Decimal::ZERO,
            card_used: Decimal::ZERO,
            cards: Vec::new(),
            verdicts,
        };
        let months = [
            month(vec![
                aside("moving", "600", true, false),
                aside("bike", "50", false, false),
            ]),
            month(vec![
                aside("moving", "400", true, false),
                aside("japan", "300", true, false),
                aside("gone", "70", true, true),
            ]),
        ];
        let wishes = [wish("moving", None, "1000", "2027-03-01")];
        assert_eq!(
            set_aside_order(&months, &wishes),
            [
                ("moving".to_owned(), d("1000")),
                ("label japan".to_owned(), d("300"))
            ]
        );
    }

    fn wish(id: &str, parent: Option<&str>, amount: &str, due: &str) -> Wish {
        Wish {
            wish_id: id.into(),
            parent_id: parent.map(String::from),
            name: id.into(),
            amount: d(amount),
            currency: Currency::eur(),
            due_on: due.into(),
            source: WishSource::Account {
                account_id: "a".into(),
            },
            saved: Decimal::ZERO,
            saved_month: None,
            status: WishStatus::Planned,
            bought: None,
            kind: WishKind::OneOff,
            must: false,
            created_at: format!("2026-01-01T00:00:{id}"),
            event: None,
        }
    }

    fn going_out_plan() -> Vec<BudgetPlan> {
        vec![plan(
            "2026-10-01",
            vec![plan_group("g-out", "Going out", Some(2), None)],
        )]
    }

    #[test]
    fn bill_schedules_the_judging_knows() {
        assert_eq!(repeat_of("Weekly", 0), Some(Repeat::Weekly));
        assert_eq!(repeat_of("weekly", 1), Some(Repeat::Biweekly));
        assert_eq!(repeat_of("weekly", 2), None);
        assert_eq!(repeat_of("monthly", 0), Some(Repeat::Monthly));
        assert_eq!(repeat_of("monthly", 1), None);
        assert_eq!(repeat_of("yearly", 0), Some(Repeat::Yearly));
        assert_eq!(repeat_of("quarterly", 0), None);
    }

    #[test]
    fn default_month_inherits_ranking_from_stored_plans() {
        let a = alloc(
            "0",
            "0",
            vec![row("Going out", "120", "0"), row("Groceries", "450", "0")],
            vec![row("Going out", "120", "0"), row("Groceries", "450", "0")],
        );
        // The dashboard only carries October's unsaved default plan.
        let dashboard = [plan(
            "2026-10-01",
            vec![
                plan_group("going-out", "Going out", None, None),
                plan_group("groceries", "Groceries", None, None),
            ],
        )];
        let stored = [plan(
            "2026-09-01",
            vec![
                plan_group("g-out", "Going out", Some(2), None),
                plan_group("g-groc", "Groceries", Some(1), Some("300")),
            ],
        )];
        let s = no_card_room();
        let ws = [wish("tv", None, "200", "2026-10-20")];
        let alone = view_from(Some(&a), &dashboard, Vec::new(), &ws, &s, &HashMap::new());
        assert!(alone.months[0].verdicts[0].cuts.is_empty());
        let plans = view_plans(dashboard.to_vec(), &stored);
        let view = view_from(Some(&a), &plans, Vec::new(), &ws, &s, &HashMap::new());
        let cuts = &view.months[0].verdicts[0].cuts;
        assert_eq!(cuts[0].group_id, "going-out");
        assert_eq!((cuts[0].from, cuts[0].to), (d("120"), d("0")));
        assert_eq!(cuts[1].group_id, "groceries");
        assert_eq!(cuts[1].to, d("370"));
    }

    #[test]
    fn view_plans_prefer_the_dashboards_month() {
        let dash = plan("2026-10-01", vec![plan_group("a", "A", None, None)]);
        let stored = [
            plan("2026-10-01", vec![plan_group("old", "A", Some(1), None)]),
            plan("2026-09-01", vec![]),
        ];
        let plans = view_plans(vec![dash.clone()], &stored);
        assert_eq!(plans.len(), 2);
        assert!(plans.contains(&dash));
        assert!(plans.iter().any(|p| p.month == "2026-09-01"));
    }

    #[test]
    fn view_end_to_end() {
        let a = alloc(
            "100",
            "0",
            vec![row("Going out", "120", "0")],
            vec![row("Going out", "120", "0")],
        );
        let view = view_from(
            Some(&a),
            &going_out_plan(),
            Vec::new(),
            &[wish("tv", None, "150", "2026-10-20")],
            &WishSettings::default(),
            &HashMap::new(),
        );
        assert_eq!(view.currency, Currency::eur());
        assert_eq!(view.months.len(), 2);
        assert_eq!(view.wishes.len(), 1);
        let r = &view.wishes[0];
        assert!(r.tradeoffs);
        assert_eq!(r.short, Decimal::ZERO);
        let v = &view.months[0].verdicts[0];
        assert_eq!(v.from_free, d("100"));
        assert_eq!(v.cuts.len(), 1);
        assert_eq!(v.cuts[0].group_id, "g-out");
        assert_eq!(v.cuts[0].from, d("120"));
        assert_eq!(v.cuts[0].to, d("70"));
    }

    #[test]
    fn card_wish_next_month_cannot_pay_is_short() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut tv = wish("tv", None, "500", "2026-10-20");
        tv.source = WishSource::Card;
        let view = view_from(
            Some(&a),
            &[],
            Vec::new(),
            &[tv],
            &WishSettings::default(),
            &HashMap::new(),
        );
        let charge = &view.months[1].verdicts[0];
        assert_eq!(charge.label, "Credit card");
        assert_eq!(charge.short, d("500"));
        assert_eq!(view.wishes[0].short, d("500"));
        assert!(view.wishes[0].tradeoffs);
    }

    #[test]
    fn card_wish_after_the_25th_is_next_statement() {
        let a = alloc("0", "1000", vec![], vec![]);
        let mut tv = wish("tv", None, "500", "2026-10-27");
        tv.source = WishSource::Card;
        let judge = |w: &Wish| {
            view_from(
                Some(&a),
                &[],
                Vec::new(),
                std::slice::from_ref(w),
                &WishSettings::default(),
                &HashMap::new(),
            )
        };
        let bought = |m: &WishMonth| {
            m.verdicts
                .iter()
                .find(|v| v.wish_id.as_deref() == Some("tv") && !v.set_aside)
                .cloned()
        };
        // 27 Oct is November's statement, charged in December.
        let view = judge(&tv);
        assert!(bought(&view.months[0]).is_none());
        assert_eq!(bought(&view.months[1]).unwrap().on_card, d("500"));
        assert_eq!(view.months[1].card_used, d("500"));
        assert_eq!(view.months[2].card_charge_in, d("500"));
        // From a bank account it is still October's.
        tv.source = account();
        let view = judge(&tv);
        assert_eq!(bought(&view.months[0]).unwrap().amount, d("500"));
        assert!(bought(&view.months[1]).is_none());
    }

    fn named_card(id: &str, used: &str, used_month: &str) -> CardSettings {
        CardSettings {
            card_id: id.into(),
            name: id.into(),
            used: d(used),
            used_month: used_month.into(),
            ..WishSettings::default().cards[0].clone()
        }
    }

    #[test]
    fn view_reports_stale_cards_and_per_card_payback() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut tv = wish("w", None, "120", "2026-10-20");
        tv.source = WishSource::Card;
        let settings = WishSettings {
            cards: vec![
                named_card("visa", "50", "2026-10-01"),
                named_card("mc", "10", "2026-09-01"),
            ],
            ..WishSettings::default()
        };
        let view = view_from(Some(&a), &[], Vec::new(), &[tv], &settings, &HashMap::new());
        assert_eq!(view.stale_cards, vec!["mc".to_string()]);
        assert!(view.card_stale);
        assert_eq!(
            view.months[0].cards.iter().map(|c| c.used).sum::<Decimal>(),
            d("170")
        );
        assert_eq!(view.months[1].card_charge_in, d("170"));
    }

    fn no_card_room() -> WishSettings {
        WishSettings {
            cards: vec![CardSettings {
                limit: Decimal::ZERO,
                ..WishSettings::default().cards[0].clone()
            }],
            ..WishSettings::default()
        }
    }

    fn buy(amount: &str, on: &str, source: WishSource) -> WishPurchase {
        WishPurchase {
            amount: d(amount),
            on: on.into(),
            source,
            cards: Vec::new(),
            transaction_id: None,
        }
    }

    fn account() -> WishSource {
        WishSource::Account {
            account_id: "a".into(),
        }
    }

    #[test]
    fn early_buy_judges_the_unsaved_gap() {
        // 100 free covers the 100 already saved (earmarked).
        let a = alloc(
            "100",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        let mut tv = wish("tv", None, "900", "2026-12-20");
        tv.saved = d("100");
        let s = no_card_room();
        let wishes = [tv];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: None,
            plans: going_out_plan(),
            card_bills: Vec::new(),
            bills: &bills,
            habits: Vec::new(),
            events: Vec::new(),
        };
        // While planned, month 0 only sets aside a third of the gap, and a
        // set-aside's cuts are not a purchase's.
        let view = inputs.view(&wishes, &s);
        assert!(view.months[0].verdicts[0].set_aside);
        assert!(month_zero_cuts(&view, "tv").is_empty());
        let cuts = inputs.purchase_cuts(&wishes, &s, "tv", &buy("900", "2026-10-15", account()));
        assert_eq!(
            cuts,
            vec![Cut {
                group_id: "g-out".into(),
                name: "Going out".into(),
                from: d("1000"),
                to: d("200"),
            }]
        );
        // On the card, the gap goes on the card and nothing is cut.
        let s = WishSettings::default();
        let card = inputs.purchase_cuts(
            &wishes,
            &s,
            "tv",
            &buy("900", "2026-10-15", WishSource::Card),
        );
        assert!(card.is_empty());
        // Fully saved: nothing to judge.
        assert!(
            inputs
                .purchase_cuts(&wishes, &s, "tv", &buy("100", "2026-10-15", account()))
                .is_empty()
        );
    }

    #[test]
    fn closed_statement_takes_no_card_wishes() {
        // On 27 Oct October's statement is closed: an overdue card wish
        // goes on November's, and a card repeat that fell on October's is
        // already in its usage.
        let a = alloc("0", "1000", vec![], vec![]);
        let mut late = wish("late", None, "200", "2026-10-20");
        late.source = WishSource::Card;
        let mut gym = wish("gym", None, "30", "2026-10-10");
        gym.source = WishSource::Card;
        gym.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: None,
        };
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: Some("2026-10-27".parse().unwrap()),
            plans: Vec::new(),
            card_bills: Vec::new(),
            bills: &bills,
            habits: Vec::new(),
            events: Vec::new(),
        };
        let view = inputs.view(&[late, gym], &WishSettings::default());
        let paid = |m: usize, id: &str| {
            view.months[m]
                .verdicts
                .iter()
                .any(|v| v.wish_id.as_deref() == Some(id) && !v.set_aside)
        };
        assert!(!paid(0, "late") && !paid(0, "gym"));
        assert!(paid(1, "late") && paid(1, "gym"));
        assert_eq!(view.months[1].card_used, d("230"));
    }

    #[test]
    fn card_buy_after_the_25th_records_next_months_cuts() {
        // Bought on the card on 28 Oct: November's statement, so November
        // (month 1) pays for it, and with no card room it cuts going out.
        let a = alloc(
            "0",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        let wishes = [wish("tv", None, "900", "2026-12-20")];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: Some("2026-10-28".parse().unwrap()),
            plans: going_out_plan(),
            card_bills: Vec::new(),
            bills: &bills,
            habits: Vec::new(),
            events: Vec::new(),
        };
        let cuts = inputs.purchase_cuts(
            &wishes,
            &no_card_room(),
            "tv",
            &buy("900", "2026-10-28", WishSource::Card),
        );
        assert_eq!(
            cuts,
            vec![Cut {
                group_id: "g-out".into(),
                name: "Going out".into(),
                from: d("1000"),
                to: d("100"),
            }]
        );
    }

    #[test]
    fn buying_a_sub_of_a_must_wish_is_judged_as_must() {
        let a = alloc(
            "250",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        // The must hotel's set-aside (100) comes first; the optional tv keeps
        // all 600 from the 150 free left plus a 450 cut to going out (set-asides
        // cut groups before they skip, and never use the cards).
        let tv = wish("tv", None, "600", "2026-12-20");
        let mut trip = wish("trip", None, "100", "2026-12-31");
        trip.must = true;
        let hotel = wish("hotel", Some("trip"), "100", "2026-12-31");
        let wishes = [tv, trip, hotel];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: None,
            plans: going_out_plan(),
            card_bills: Vec::new(),
            bills: &bills,
            habits: Vec::new(),
            events: Vec::new(),
        };
        let view = inputs.view(&wishes, &no_card_room());
        let tv: Vec<_> = view.months[0]
            .verdicts
            .iter()
            .filter(|v| v.wish_id.as_deref() == Some("tv"))
            .map(|v| (v.set_aside, v.skipped, v.amount, v.from_free, v.cuts.len()))
            .collect();
        assert_eq!(tv, vec![(true, false, d("600"), d("150"), 1)]);
        // The must purchase is judged before the optional set-aside, so the
        // 250 free covers it and nothing is cut.
        let cuts = inputs.purchase_cuts(
            &wishes,
            &no_card_room(),
            "hotel",
            &buy("100", "2026-10-15", account()),
        );
        assert!(cuts.is_empty(), "{cuts:?}");
    }

    #[test]
    fn unjudged_rows_say_why() {
        let a = alloc("1000", "1000", vec![], vec![]);
        let mut saved = wish("saved", None, "40", "2026-10-20");
        saved.saved = d("40");
        let mut change = wish("change", None, "45", "2026-11-01");
        change.kind = WishKind::BillChange {
            bill_id: "gone".into(),
            ends_on: None,
        };
        let ws = [
            wish("near", None, "50", "2026-10-20"),
            wish("far", None, "50", "2032-10-20"),
            saved,
            change,
        ];
        let view = view_from(
            Some(&a),
            &[],
            Vec::new(),
            &ws,
            &WishSettings::default(),
            &HashMap::new(),
        );
        let why = |id: &str| {
            view.wishes
                .iter()
                .find(|r| r.wish.wish_id == id)
                .unwrap()
                .not_judged
                .clone()
        };
        assert_eq!(view.months.len(), 60);
        assert_eq!(why("near"), None);
        assert_eq!(why("far").as_deref(), Some("too far ahead"));
        assert_eq!(why("saved"), None);
        assert_eq!(why("change").as_deref(), Some("bill can't be judged"));
    }

    #[test]
    fn parent_row_aggregates_children_and_rest() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut gone = wish("gone", None, "10", "2026-09-01");
        gone.status = WishStatus::Dropped;
        let ws = [
            wish("trip", None, "1000", "2026-12-31"),
            wish("hotel", Some("trip"), "400", "2026-11-10"),
            wish("later", None, "60", "2026-12-01"),
            gone,
        ];
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
        let ids: Vec<_> = view
            .wishes
            .iter()
            .map(|r| r.wish.wish_id.as_str())
            .collect();
        assert_eq!(ids, vec!["gone", "later", "trip"]);
        let gone = &view.wishes[0];
        assert_eq!(gone.short, Decimal::ZERO);
        assert!(!gone.tradeoffs);
        let trip = &view.wishes[2];
        assert_eq!(trip.children.len(), 1);
        assert_eq!(trip.rest, d("600"));
        assert!(!trip.over_cap);
        // Nothing is affordable: trip's short is its rest plus its hotel.
        assert_eq!(trip.children[0].short, d("400"));
        assert_eq!(trip.short, d("1000"));
        // Must-wishes addendum: an optional set-aside that does not fit
        // without the cards is skipped, so a month with nothing free sets
        // nothing aside (it would be hotel 400/2, rest 600/3).
        assert_eq!(trip.children[0].monthly_set_aside, Decimal::ZERO);
        assert_eq!(trip.monthly_set_aside, Decimal::ZERO);
        assert_eq!(view.wishes[1].monthly_set_aside, Decimal::ZERO);
    }

    #[test]
    fn wish_paid_from_savings_is_a_tradeoff() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.savings_balance = d("500");
        let ws = [wish("tv", None, "150", "2026-10-20")];
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
        let v = &view.months[0].verdicts[0];
        assert_eq!(v.from_savings, d("150"));
        assert!(view.wishes[0].tradeoffs);
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
    }

    #[test]
    fn parent_row_sums_month_zero_set_asides_that_fit() {
        let a = alloc("1000", "1000", vec![], vec![]);
        let ws = [
            wish("trip", None, "1000", "2026-12-31"),
            wish("hotel", Some("trip"), "400", "2026-11-10"),
            wish("later", None, "60", "2026-12-01"),
        ];
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
        let trip = view
            .wishes
            .iter()
            .find(|r| r.wish.wish_id == "trip")
            .unwrap();
        // Month 0 set-asides, soonest first: hotel (10 Nov) keeps all 400,
        // "later" (1 Dec) its 60, the trip's rest (31 Dec) the 540 left.
        assert_eq!(trip.children[0].monthly_set_aside, d("400"));
        assert_eq!(trip.monthly_set_aside, d("940"));
        assert_eq!(trip.short, Decimal::ZERO);
    }

    #[test]
    fn no_allocation_leaves_wishes_unjudged() {
        let view = view_from(
            None,
            &[],
            Vec::new(),
            &[wish("tv", None, "150", "2026-10-20")],
            &WishSettings::default(),
            &HashMap::new(),
        );
        assert!(view.months.is_empty());
        assert_eq!(view.wishes.len(), 1);
        assert_eq!(view.wishes[0].not_judged.as_deref(), Some("no budget data"));
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
        assert!(!view.wishes[0].tradeoffs);
        assert_eq!(view.currency, Currency::eur());
    }

    #[test]
    fn month_zero_cuts_merge_repeated_groups() {
        let cut = |g: &str, from: &str, to: &str| Cut {
            group_id: g.into(),
            name: g.into(),
            from: d(from),
            to: d(to),
        };
        let verdict = |id: Option<&str>, cuts: Vec<Cut>| Verdict {
            wish_id: id.map(String::from),
            label: String::new(),
            amount: Decimal::ZERO,
            from_free: Decimal::ZERO,
            from_savings: Decimal::ZERO,
            cuts,
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
            set_aside: false,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
            must: false,
            skipped: false,
        };
        let month = |verdicts| WishMonth {
            month: "2026-10-01".into(),
            free_before: Decimal::ZERO,
            free_left: Decimal::ZERO,
            savings_before: Decimal::ZERO,
            savings_left: Decimal::ZERO,
            card_charge_in: Decimal::ZERO,
            card_used: Decimal::ZERO,
            cards: Vec::new(),
            verdicts,
        };
        let view = WishesView {
            currency: Currency::eur(),
            settings: WishSettings::default(),
            card_stale: false,
            stale_cards: Vec::new(),
            changeable_bills: Vec::new(),
            savings_must_keep: Decimal::ZERO,
            events: Vec::new(),
            goals: None,
            months: vec![
                month(vec![
                    verdict(
                        Some("tv"),
                        vec![cut("g1", "120", "90"), cut("g2", "50", "40")],
                    ),
                    verdict(Some("other"), vec![cut("g3", "10", "0")]),
                    verdict(Some("tv"), vec![cut("g1", "90", "70")]),
                ]),
                month(vec![verdict(Some("tv"), vec![cut("g4", "5", "0")])]),
            ],
            wishes: vec![],
        };
        let cuts = month_zero_cuts(&view, "tv");
        assert_eq!(cuts, vec![cut("g1", "120", "70"), cut("g2", "50", "40")]);
        assert!(month_zero_cuts(&view, "none").is_empty());
    }

    fn habit(id: &str, typical: &str, budget: &str, essential: bool) -> goals::HabitRow {
        goals::HabitRow {
            id: id.into(),
            name: id.into(),
            typical: d(typical),
            budget: d(budget),
            typical_visits: Some(d("30")),
            ticket: None,
            essential,
        }
    }

    fn with_habits(
        a: &CurrencyAllocation,
        habits: Vec<goals::HabitRow>,
        wishes: &[Wish],
    ) -> WishesView {
        Inputs {
            alloc: Some(a),
            today: None,
            plans: Vec::new(),
            card_bills: Vec::new(),
            bills: &HashMap::new(),
            habits,
            events: Vec::new(),
        }
        .view(wishes, &WishSettings::default())
    }

    #[test]
    fn an_event_is_judged_on_what_its_months_ask_and_is_not_a_wish_row() {
        // 500 free now and each later month; the event asks 66.67 of
        // November and 283.33 of December.
        let a = alloc("500", "500", vec![], vec![]);
        let parents = Wish {
            event: Some(finance_types::gateway::EventDetails {
                starts_on: "2026-11-30".into(),
                ends_on: "2026-12-05".into(),
                ..Default::default()
            }),
            ..wish("e", None, "400", "2026-11-30")
        };
        let bike = wish("bike", None, "100", "2026-12-20");
        let inputs = Inputs {
            alloc: Some(&a),
            today: None,
            plans: Vec::new(),
            card_bills: Vec::new(),
            bills: &HashMap::new(),
            habits: Vec::new(),
            events: vec![EventNeed {
                wish_id: "e".into(),
                months: vec![
                    (date(2026, 11, 1), d("66.67")),
                    (date(2026, 12, 1), d("283.33")),
                ],
            }],
        };
        let wishes = [parents, bike];
        let view = inputs.view(&wishes, &WishSettings::default());
        let ids: Vec<&str> = view
            .wishes
            .iter()
            .map(|r| r.wish.wish_id.as_str())
            .collect();
        assert_eq!(ids, ["bike"]);
        let (rows, months) = inputs.event_rows(&wishes, &WishSettings::default());
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].short, rows[0].tradeoffs), (Decimal::ZERO, false));
        // The event's verdicts carry its own id, and ask 350 in all.
        let asked: Decimal = months
            .iter()
            .flat_map(|m| &m.verdicts)
            .filter(|v| v.wish_id.as_deref() == Some("e"))
            .map(|v| v.amount)
            .sum();
        assert_eq!(asked, d("350"));
        assert_eq!(
            fits_up_to(&rows[0], &months, &[date(2026, 11, 1), date(2026, 12, 1)]),
            Some(d("400") + free_after(&months))
        );
    }

    fn free_after(months: &[WishMonth]) -> Decimal {
        // October (the fixture's running month) to December.
        months[..3].iter().map(|m| m.free_left).min().unwrap()
    }

    #[test]
    fn habits_above_budget_put_an_affordable_wish_on_the_card() {
        // The plan leaves 500 a month; food out runs 470 above its budget.
        let a = alloc("0", "500", vec![], vec![]);
        let trip = wish("trip", None, "900", "2026-12-20");
        let view = with_habits(&a, vec![habit("Food", "570", "100", false)], &[trip]);
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
        assert!(!view.wishes[0].tradeoffs);
        let goals = view.goals.unwrap();
        assert_eq!((goals.plan_free, goals.habit_free), (d("500"), d("30")));
        // 900 over November and December.
        assert_eq!((goals.needed_monthly, goals.gap), (d("450"), d("420")));
        assert_eq!(goals.coverage, Some(6));
        assert_eq!(goals.horizon_months, 3);
        // Two months of 30 against 900: the rest lands on the card.
        let trip = &goals.goals[0];
        assert_eq!(
            (trip.short, trip.on_card, trip.plan_short),
            (d("0"), d("840"), d("0"))
        );
        let food = &goals.cuts[0];
        assert_eq!((food.monthly, goals.uncovered), (d("420"), d("0")));
    }

    #[test]
    fn essentials_are_not_cut_so_the_wish_is_short_on_habits() {
        let a = alloc("0", "500", vec![], vec![]);
        let trip = wish("trip", None, "900", "2026-12-20");
        let view = with_habits(&a, vec![habit("Groceries", "330", "100", true)], &[trip]);
        let goals = view.goals.unwrap();
        assert!(goals.cuts.is_empty());
        assert_eq!((goals.gap, goals.uncovered), (d("180"), d("180")));
        // Two months of 270 against 900: the rest lands on the card.
        let trip = &goals.goals[0];
        assert_eq!(
            (trip.short, trip.on_card, trip.plan_short),
            (d("0"), d("360"), d("0"))
        );
        let fund = goals.fund.unwrap();
        assert_eq!((fund.months, fund.essentials_monthly), (6, d("330")));
    }

    #[test]
    fn a_goal_the_card_covers_is_not_short_the_card_bill_is() {
        // Nothing free now, so 600 goes on the card; next month's 270 at
        // current habits cannot pay the whole charge.
        let a = alloc("0", "500", vec![], vec![]);
        let tv = wish("tv", None, "600", "2026-10-20");
        let view = with_habits(&a, vec![habit("Groceries", "330", "100", true)], &[tv]);
        let tv = &view.goals.unwrap().goals[0];
        assert_eq!(
            (tv.short, tv.on_card, tv.card_short),
            (d("0"), d("600"), d("330"))
        );
    }

    #[test]
    fn a_wish_paid_from_savings_needs_no_cuts() {
        let mut a = alloc("0", "500", vec![], vec![]);
        a.savings_balance = d("1000");
        let tv = wish("tv", None, "300", "2026-10-20");
        let view = with_habits(&a, vec![habit("Food", "570", "100", false)], &[tv]);
        let goals = view.goals.unwrap();
        assert!(goals.cuts.is_empty());
        assert_eq!(goals.goals[0].short, Decimal::ZERO);
        assert_eq!(goals.fund.unwrap().balance, d("1000"));
        // Due this month: not part of what the coming months need.
        assert_eq!(
            (goals.needed_monthly, goals.coverage),
            (Decimal::ZERO, None)
        );
    }

    #[test]
    fn no_habits_or_no_wishes() {
        let a = alloc("0", "500", vec![], vec![]);
        let tv = wish("tv", None, "300", "2026-10-20");
        assert!(with_habits(&a, Vec::new(), &[tv]).goals.is_none());
        // Without wishes the fund still fills: 6 × 330 at 270 a month.
        let goals = with_habits(&a, vec![habit("Groceries", "330", "100", true)], &[])
            .goals
            .unwrap();
        assert_eq!(
            (goals.needed_monthly, goals.coverage),
            (Decimal::ZERO, None)
        );
        assert!(goals.goals.is_empty());
        let fund = goals.fund.unwrap();
        assert_eq!(fund.target, d("1980"));
        assert_eq!(fund.eta.as_deref(), Some("2027-06-01"));
    }

    #[test]
    fn a_planned_repeating_cost_is_part_of_what_the_goals_need() {
        // A new subscription from December, no dated purchase.
        let a = alloc("0", "500", vec![], vec![]);
        let mut gym = wish("gym", None, "100", "2026-12-01");
        gym.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: None,
        };
        let goals = with_habits(&a, vec![habit("Food", "570", "100", false)], &[gym])
            .goals
            .unwrap();
        assert_eq!(
            (goals.needed_monthly, goals.habit_free, goals.gap),
            (d("100"), d("30"), d("70"))
        );
        assert_eq!(goals.coverage, Some(30));
        assert_eq!(goals.cuts[0].monthly, d("70"));
    }

    #[test]
    fn savings_that_cover_a_later_wish_leave_no_gap() {
        let mut a = alloc("0", "500", vec![], vec![]);
        a.savings_balance = d("1000");
        let tv = wish("tv", None, "300", "2026-12-20");
        let bike = wish("bike", None, "900", "2027-01-20");
        let habits = || vec![habit("Food", "570", "100", false)];
        let covered = with_habits(&a, habits(), std::slice::from_ref(&tv))
            .goals
            .unwrap();
        assert_eq!((covered.needed_monthly, covered.gap), (d("0"), d("0")));
        assert!(covered.cuts.is_empty());
        // The pot pays the TV and 700 of the bike; 200 is left over 3 months.
        let both = with_habits(&a, habits(), &[tv, bike]).goals.unwrap();
        assert_eq!(both.needed_monthly, d("66.67"));
    }

    #[test]
    fn next_months_budget_is_the_one_habits_are_measured_against() {
        // Food out is budgeted 300 this month and 200 the next; free money
        // for next month already paid the 200.
        let a = alloc("0", "500", vec![], vec![row("Food", "200", "0")]);
        let goals = with_habits(&a, vec![habit("Food", "570", "300", false)], &[])
            .goals
            .unwrap();
        assert_eq!(goals.habit_free, d("130"));
    }

    #[test]
    fn a_wish_meant_for_the_card_is_not_a_card_warning() {
        let a = alloc("0", "500", vec![], vec![]);
        let mut trip = wish("trip", None, "900", "2026-12-20");
        trip.source = WishSource::Card;
        let goals = with_habits(&a, vec![habit("Food", "570", "100", false)], &[trip])
            .goals
            .unwrap();
        assert_eq!(goals.goals[0].on_card, d("0"));
    }

    #[test]
    fn a_repeating_cost_with_an_end_is_still_needed_while_it_runs() {
        let a = alloc("0", "500", vec![], vec![]);
        let mut course = wish("course", None, "100", "2026-12-01");
        course.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: Some("2027-11-01".into()),
        };
        let goals = with_habits(&a, vec![habit("Food", "570", "100", false)], &[course])
            .goals
            .unwrap();
        assert_eq!((goals.needed_monthly, goals.gap), (d("100"), d("70")));
    }
}
