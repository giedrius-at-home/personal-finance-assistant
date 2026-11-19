//! Spending → Plan & patterns, "Adjustments": every spending group against
//! the owner's own typical month, what drives a group that runs above it,
//! one budget action, and how often the owner goes where.

use finance_types::gateway::{BudgetGroup, BudgetPlan};
use finance_types::report::adjustments::{AdjustmentRow, Adjustments, Habit};
use finance_types::report::budget::{BudgetRow, MonthlyBudget};
use finance_types::report::{Currency, Dashboard};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

use crate::api::invoke;
use crate::ds::{Card, CardHeader, CardTitle, Eyebrow, Size, Variant, button_class, money};
use crate::pages::overview::visuals::{Pace, month_end, row_color};
use crate::pages::with_report;
use crate::{Tab, format, state};

/// Earlier months with spending a typical month needs.
const TYPICAL_MONTHS: usize = 3;
/// Days of the month and purchases before a pace means anything.
const EARLY_DAYS: usize = 7;
const EARLY_VISITS: usize = 2;
/// Purchases in the month from which an unbudgeted group runs at a daily pace.
const HABIT_VISITS: usize = 5;
/// The least a finding must be worth, in the currency's units.
const WORTH: Decimal = Decimal::from_parts(25, 0, 0, false, 0);
/// Visits lately that put a place in the habits table.
const SHOWN_VISITS: usize = 4;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn ratio(percent: i64) -> Decimal {
    Decimal::new(percent, 2)
}

fn median(mut values: Vec<Decimal>) -> Option<Decimal> {
    values.sort();
    let n = values.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(values[n / 2]),
        _ => Some((values[n / 2 - 1] + values[n / 2]) / Decimal::TWO),
    }
}

/// The owner's typical month for a row: the median of the earlier months
/// that had spending, once there are three.
pub fn typical(row: &BudgetRow) -> Option<Decimal> {
    let earlier = row.history.len().saturating_sub(1);
    let spent: Vec<Decimal> = row.history[..earlier]
        .iter()
        .copied()
        .filter(|v| *v > Decimal::ZERO)
        .collect();
    (spent.len() >= TYPICAL_MONTHS)
        .then(|| median(spent))
        .flatten()
}

/// Too little of the month, or too few purchases, to say where it is heading.
pub fn too_early(row: &AdjustmentRow) -> bool {
    row.row.elapsed_days < EARLY_DAYS || row.habit.visits < EARLY_VISITS
}

/// Where the month is heading at its daily pace (Overview's projection).
pub fn heading(row: &AdjustmentRow, days: usize) -> Option<Decimal> {
    if too_early(row) {
        return None;
    }
    // A handful of purchases is not a daily habit: an unbudgeted group
    // counts what was spent. Budget rows keep Overview's daily pace.
    let pace = if row.budgeted || row.habit.visits >= HABIT_VISITS {
        Pace::Daily
    } else {
        Pace::Lump
    };
    // The division leaves a hair of error; a finding must not hang on it.
    month_end(&row.row, pace, days).map(|end| end.round_dp(2))
}

/// How far above a typical month the row is heading, when that is at least
/// a quarter and worth mentioning.
pub fn above_norm(row: &AdjustmentRow, days: usize) -> Option<Decimal> {
    let typical = typical(&row.row)?;
    let excess = heading(row, days)? - typical;
    (excess >= typical * ratio(25) && excess >= WORTH).then_some(excess)
}

/// A budget that does not fit the owner's habit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetFit {
    /// A fifth or more below a typical month.
    Low,
    /// A fifth or more above it, and worth mentioning.
    High,
}

pub fn budget_fit(row: &AdjustmentRow) -> Option<BudgetFit> {
    if !row.budgeted {
        return None;
    }
    let typical = typical(&row.row)?;
    let budget = row.row.planned;
    if budget <= typical * ratio(80) {
        Some(BudgetFit::Low)
    } else if budget >= typical * ratio(120) && budget - typical >= WORTH {
        Some(BudgetFit::High)
    } else {
        None
    }
}

/// `amount` to the nearest ten, halves up.
pub fn round_to_ten(amount: Decimal) -> Decimal {
    (amount / Decimal::TEN).round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        * Decimal::TEN
}

/// The one change to the budget plan a row suggests, with the new amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Set(Decimal),
    Lower(Decimal),
    Add(Decimal),
}

impl Action {
    pub fn amount(&self) -> Decimal {
        match self {
            Self::Set(amount) | Self::Lower(amount) | Self::Add(amount) => *amount,
        }
    }
}

pub fn action(row: &AdjustmentRow, days: usize) -> Option<Action> {
    let amount = round_to_ten(typical(&row.row)?);
    match budget_fit(row) {
        Some(BudgetFit::Low) => Some(Action::Set(amount)),
        Some(BudgetFit::High) => Some(Action::Lower(amount)),
        None if !row.budgeted && amount >= WORTH => {
            above_norm(row, days).map(|_| Action::Add(amount))
        }
        None => None,
    }
}

/// `Health & personal care` → `health-personal-care`.
pub fn slug(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// `plan` (the saved one, never the shadow plan) with the action applied.
pub fn apply(plan: &BudgetPlan, row: &AdjustmentRow, action: &Action) -> BudgetPlan {
    let mut plan = plan.clone();
    match action {
        Action::Set(amount) | Action::Lower(amount) => {
            for group in plan.groups.iter_mut().filter(|g| g.id == row.row.id) {
                group.amount = *amount;
            }
        }
        Action::Add(amount) => {
            let name = row.group.clone().unwrap_or_else(|| row.row.name.clone());
            let base = slug(&name);
            let taken = |id: &str| plan.groups.iter().any(|g| g.id == id);
            // The plan refuses two groups of one name (case-folded).
            let named = plan
                .groups
                .iter()
                .any(|g| g.name.trim().to_lowercase() == name.to_lowercase());
            let source = name.clone();
            let name = if named {
                format!("{name} (rest)")
            } else {
                name
            };
            let id = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base}-{n}")))
                .find(|id| !taken(id))
                .unwrap_or(base);
            plan.groups.push(BudgetGroup {
                id,
                source_groups: vec![source],
                name,
                amount: *amount,
                categories: Vec::new(),
                transaction_ids: Vec::new(),
                account_id: None,
                importance: None,
                floor: None,
            });
        }
    }
    plan
}

/// A store behind a row's excess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Driver {
    pub category: String,
    pub visits: usize,
    pub spent: Decimal,
    /// The usual `(visits, spending)` of a month, once two earlier months
    /// had a visit.
    pub usual: Option<(Decimal, Decimal)>,
    /// Never visited in the earlier months.
    pub new: bool,
}

/// The up to three stores furthest above their usual spending for this
/// point of the month; a store without history counts all it spent.
pub fn drivers(row: &AdjustmentRow, days: usize) -> Vec<Driver> {
    let days = days.max(row.row.elapsed_days).max(1);
    let progress = Decimal::from(row.row.elapsed_days) / Decimal::from(days);
    let mut ranked: Vec<(Decimal, Driver)> = row
        .stores
        .iter()
        .map(|store| {
            let habit = &store.habit;
            let usual = habit.typical_visits.zip(habit.typical_spent);
            let expected = usual.map_or(Decimal::ZERO, |(_, spent)| spent * progress);
            (
                habit.spent - expected,
                Driver {
                    category: store.category.clone(),
                    visits: habit.visits,
                    spent: habit.spent,
                    usual,
                    new: habit.history_month_count == 0,
                },
            )
        })
        .filter(|(excess, _)| *excess > Decimal::ZERO)
        .collect();
    ranked.sort_by(|a, b| (b.0, &a.1.category).cmp(&(a.0, &b.1.category)));
    ranked
        .into_iter()
        .take(3)
        .map(|(_, driver)| driver)
        .collect()
}

/// The one or two weekdays holding at least a quarter of the visits each.
pub fn mostly(weekday_visits: &[usize; 7]) -> String {
    let total: usize = weekday_visits.iter().sum();
    let mut busy: Vec<usize> = (0..7)
        .filter(|day| total > 0 && weekday_visits[*day] * 4 >= total)
        .collect();
    busy.sort_by_key(|day| std::cmp::Reverse(weekday_visits[*day]));
    busy.truncate(2);
    busy.sort_unstable();
    if busy.is_empty() {
        return "any day".to_owned();
    }
    busy.into_iter()
        .map(|day| WEEKDAYS[day])
        .collect::<Vec<_>>()
        .join(", ")
}

/// At least two visits above the usual week.
pub fn busy_week(habit: &Habit) -> bool {
    Decimal::from(habit.visits_this_week) >= habit.typical_week_visits + Decimal::TWO
}

/// Visited often enough lately to list.
pub fn shown(habit: &Habit) -> bool {
    habit.recent_visits >= SHOWN_VISITS
}

/// One decimal, without a trailing `.0`.
fn one_decimal(value: Decimal) -> String {
    value
        .round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

/// A usual number of visits a week: `0`, `<1`, `2`, `2.5`.
pub fn per_week(value: Decimal) -> String {
    if value > Decimal::ZERO && value < Decimal::ONE {
        "<1".to_owned()
    } else {
        one_decimal(value)
    }
}

pub fn visits_text(visits: usize) -> String {
    format!("{visits} visit{}", if visits == 1 { "" } else { "s" })
}

/// "Coffee: 22 visits × €2.80, usually 12 × €2.90".
pub fn driver_text(driver: &Driver, unit: &Currency) -> String {
    let Driver {
        category,
        visits,
        spent,
        usual,
        new,
    } = driver;
    match usual {
        Some((usual_visits, usual_spent)) if *visits > 0 && *usual_visits > Decimal::ZERO => {
            format!(
                "{category}: {} × {}, usually {} × {}",
                visits_text(*visits),
                money(*spent / Decimal::from(*visits), unit),
                one_decimal(*usual_visits),
                money(*usual_spent / *usual_visits, unit),
            )
        }
        _ if !new => format!(
            "{category}: {}, {}",
            visits_text(*visits),
            money(*spent, unit)
        ),
        _ => format!(
            "{category}: new this month, {}, {}",
            visits_text(*visits),
            money(*spent, unit)
        ),
    }
}

/// Whole units: budgets, typical months and month-end figures.
fn whole(value: Decimal, unit: &Currency) -> String {
    format::money_dp(value, &unit.code, 0)
}

#[derive(Serialize)]
struct PlanArgs {
    plan: BudgetPlan,
}

/// What the card keeps between renders: the row whose change awaits a
/// confirmation, a save in flight, and the last save error by row.
#[derive(Clone, Copy)]
struct CardState {
    confirming: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<(String, String)>>,
}

impl CardState {
    fn save(self, row_id: String, plan: BudgetPlan) {
        self.busy.set(true);
        self.error.set(None);
        let app = state();
        spawn_local(async move {
            match invoke::<BudgetPlan>("save_budget_plan", &PlanArgs { plan }).await {
                Ok(_) => {
                    self.confirming.set(None);
                    app.refresh();
                }
                Err(e) => self.error.set(Some((
                    row_id,
                    if e.is_empty() {
                        "Could not save this budget.".to_owned()
                    } else {
                        e
                    },
                ))),
            }
            self.busy.set(false);
        });
    }
}

/// A row with everything the card says about it.
struct Line {
    row: AdjustmentRow,
    typical: Decimal,
    heading: Option<Decimal>,
    excess: Option<Decimal>,
    fit: Option<BudgetFit>,
    action: Option<Action>,
}

impl Line {
    fn flagged(&self) -> bool {
        self.excess.is_some() || self.fit.is_some()
    }
}

/// The rows that have a typical month, the flagged ones first (the largest
/// excess leading), then the rest by name.
fn lines(report: &Adjustments, days: usize) -> Vec<Line> {
    let mut lines: Vec<Line> = report
        .rows
        .iter()
        .filter_map(|row| {
            Some(Line {
                typical: typical(&row.row)?,
                heading: heading(row, days),
                excess: above_norm(row, days),
                fit: budget_fit(row),
                action: action(row, days),
                row: row.clone(),
            })
        })
        .collect();
    lines.sort_by(|a, b| {
        (!a.flagged(), b.excess, &a.row.row.name).cmp(&(!b.flagged(), a.excess, &b.row.row.name))
    });
    lines
}

fn action_label(action: &Action, unit: &Currency) -> String {
    let amount = whole(action.amount(), unit);
    match action {
        Action::Set(_) => format!("Set budget to {amount}"),
        Action::Lower(_) => format!("Lower budget to {amount}"),
        Action::Add(_) => format!("Add a {amount} budget"),
    }
}

/// The action button, or its confirmation once clicked.
fn action_view(line: &Line, budget: &MonthlyBudget, state: CardState) -> AnyView {
    let Some(action) = line.action.clone() else {
        return ().into_any();
    };
    let unit = budget.plan.currency.clone();
    let id = line.row.row.id.clone();
    let label = action_label(&action, &unit);
    let change = format!(
        "{}: {} → {}",
        line.row.row.name,
        if line.row.budgeted {
            whole(line.row.row.planned, &unit)
        } else {
            "no budget".to_owned()
        },
        whole(action.amount(), &unit)
    );
    let plan = apply(&budget.plan, &line.row, &action);
    let (shown_id, ask_id, save_id, error_id) = (id.clone(), id.clone(), id.clone(), id);
    view! {
        <div class="flex flex-wrap items-center gap-2">
            <Show
                when=move || state.confirming.with(|c| c.as_deref() == Some(shown_id.as_str()))
                fallback={
                    let label = label.clone();
                    let ask_id = ask_id.clone();
                    move || {
                        let ask_id = ask_id.clone();
                        view! {
                            <button
                                type="button"
                                class=button_class(Variant::Secondary, Size::Sm)
                                on:click=move |_| state.confirming.set(Some(ask_id.clone()))
                            >
                                {label.clone()}
                            </button>
                        }
                    }
                }
            >
                <span class="text-sm tabular-nums">{change.clone()}</span>
                <button
                    type="button"
                    class=button_class(Variant::Primary, Size::Sm)
                    disabled=move || state.busy.get()
                    on:click={
                        let (save_id, plan) = (save_id.clone(), plan.clone());
                        move |_| state.save(save_id.clone(), plan.clone())
                    }
                >
                    "Confirm"
                </button>
                <button
                    type="button"
                    class=button_class(Variant::Ghost, Size::Sm)
                    on:click=move |_| state.confirming.set(None)
                >
                    "Cancel"
                </button>
            </Show>
            {move || {
                state
                    .error
                    .with(|e| e.as_ref().filter(|(row, _)| *row == error_id).map(|(_, m)| m.clone()))
                    .map(|message| view! { <p role="alert" class="w-full text-sm text-negative-fg">{message}</p> })
            }}
        </div>
    }
    .into_any()
}

fn line_view(line: &Line, budget: &MonthlyBudget, days: usize, state: CardState) -> AnyView {
    let unit = &budget.plan.currency;
    let row = &line.row;
    let app = crate::state();
    let dot = if row.budgeted {
        row_color(budget, &row.row.id)
    } else {
        "var(--color-fg-muted)".to_owned()
    };
    let budget_text = if row.budgeted {
        format!("budget {}", whole(row.row.planned, unit))
    } else {
        "no budget".to_owned()
    };
    let heading_text = match line.heading {
        Some(end) => format!("heading to {}", whole(end, unit)),
        None => "too early to tell".to_owned(),
    };
    let norm = line.excess.zip(line.heading).map(|(excess, end)| {
        let text = format!(
            "Heading to {}, about {} above a typical month.",
            whole(end, unit),
            whole(excess, unit)
        );
        let link = row.budgeted.then(|| {
            view! {
                <button
                    type="button"
                    class=button_class(Variant::Link, Size::Inline)
                    on:click=move |_| app.go(Tab::Overview)
                >
                    "See this month →"
                </button>
            }
        });
        view! { <p class="flex flex-wrap items-baseline gap-x-3 text-sm">{text}{link}</p> }
    });
    let fit = line.fit.map(|fit| {
        let text = match fit {
            BudgetFit::Low => format!(
                "The budget is {}; a typical month is {}.",
                whole(row.row.planned, unit),
                whole(line.typical, unit)
            ),
            BudgetFit::High => format!(
                "You usually spend {}; {} of this budget is spare.",
                whole(line.typical, unit),
                whole(row.row.planned - line.typical, unit)
            ),
        };
        view! { <p class="text-sm">{text}</p> }
    });
    let found = if line.excess.is_some() {
        drivers(row, days)
    } else {
        Vec::new()
    };
    let drivers = (!found.is_empty()).then(|| {
        view! {
            <details class="text-sm text-fg-muted">
                <summary class="cursor-pointer">"What drives it"</summary>
                <ul class="mt-1 grid gap-1 pl-4">
                    {found
                        .iter()
                        .map(|driver| view! { <li class="tabular-nums">{driver_text(driver, unit)}</li> })
                        .collect_view()}
                </ul>
            </details>
        }
    });
    view! {
        <li class="grid gap-1.5 py-3">
            <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <span
                    aria-hidden="true"
                    class="size-2.5 shrink-0 self-center rounded-full"
                    style=format!("background:{dot}")
                />
                <strong class="font-semibold">{row.row.name.clone()}</strong>
                <span class="text-sm tabular-nums text-fg-muted">
                    {format!("{budget_text} · typical {} · {heading_text}", whole(line.typical, unit))}
                </span>
            </div>
            {norm}
            {fit}
            {action_view(line, budget, state)}
            {drivers}
        </li>
    }
    .into_any()
}

const HABIT_GRID: &str =
    "grid grid-cols-[minmax(9rem,1fr)_repeat(4,4.5rem)_5.5rem_5.5rem] items-baseline gap-x-2";

fn habit_cells(habit: &Habit, unit: &Currency) -> impl IntoView + use<> {
    let week_class = if busy_week(habit) {
        "text-right font-semibold text-accent"
    } else {
        "text-right"
    };
    let ticket = if habit.visits > 0 && habit.spent > Decimal::ZERO {
        money(habit.spent / Decimal::from(habit.visits), unit)
    } else {
        "—".to_owned()
    };
    view! {
        <span class=week_class>{habit.visits_this_week}</span>
        <span class="text-right text-fg-muted">{per_week(habit.typical_week_visits)}</span>
        <span class="text-right">{habit.visits}</span>
        <span class="text-right text-fg-muted">
            {habit.typical_visits.map_or("—".to_owned(), one_decimal)}
        </span>
        <span class="text-right">{ticket}</span>
        <span class="text-right text-fg-muted">{mostly(&habit.weekday_visits)}</span>
    }
}

/// How often the owner goes where: every row visited lately, opening to
/// its stores.
fn habits_view(report: &Adjustments) -> Option<AnyView> {
    let unit = &report.currency;
    let (often, seldom): (Vec<&AdjustmentRow>, Vec<&AdjustmentRow>) =
        report.rows.iter().partition(|row| shown(&row.habit));
    if often.is_empty() {
        return None;
    }
    let rows = often
        .into_iter()
        .map(|row| {
            let stores: Vec<_> = row.stores.iter().filter(|s| shown(&s.habit)).collect();
            view! {
                <details class="border-t border-border py-2">
                    <summary class=format!("{HABIT_GRID} cursor-pointer")>
                        <strong class="font-semibold">{row.row.name.clone()}</strong>
                        {habit_cells(&row.habit, unit)}
                    </summary>
                    {stores
                        .into_iter()
                        .map(|store| {
                            view! {
                                <div class=format!("{HABIT_GRID} pt-1.5 text-fg-muted")>
                                    <span class="pl-4 text-fg">{store.category.clone()}</span>
                                    {habit_cells(&store.habit, unit)}
                                </div>
                            }
                        })
                        .collect_view()}
                </details>
            }
        })
        .collect_view();
    Some(
        view! {
            <section class="mt-5" aria-label="Habits">
                <Eyebrow>"HABITS · HOW OFTEN YOU GO"</Eyebrow>
                <div class="mt-2 overflow-x-auto">
                    <div class="min-w-[44rem] text-sm tabular-nums">
                        <div class=format!("{HABIT_GRID} pb-2 text-xs text-fg-muted")>
                            <span></span>
                            <span class="text-right">"this week"</span>
                            <span class="text-right">"usual/wk"</span>
                            <span class="text-right">"this month"</span>
                            <span class="text-right">"usual/mo"</span>
                            <span class="text-right">"avg ticket"</span>
                            <span class="text-right">"mostly"</span>
                        </div>
                        {rows}
                    </div>
                </div>
                {(!seldom.is_empty())
                    .then(|| {
                        view! {
                            <p class="mt-2 text-xs text-fg-muted">
                                {format!("Less often ({})", seldom.len())}
                            </p>
                        }
                    })}
            </section>
        }
        .into_any(),
    )
}

/// One currency's card; `None` until some row has a typical month.
fn card(report: &Adjustments, dashboard: &Dashboard, state: CardState) -> Option<AnyView> {
    let budget = dashboard
        .monthly_budgets
        .iter()
        .find(|b| b.plan.currency == report.currency && b.plan.month == report.month)?;
    let unit = &report.currency;
    let days = format::days_in_month(&report.month) as usize;
    let lines = lines(report, days);
    if lines.is_empty() {
        return None;
    }
    let app = crate::state();
    let above: Decimal = lines.iter().filter_map(|line| line.excess).sum();
    let near: Vec<String> = lines
        .iter()
        .filter(|line| !line.flagged())
        .map(|line| line.row.row.name.clone())
        .collect();
    let flagged = lines
        .iter()
        .filter(|line| line.flagged())
        .map(|line| line_view(line, budget, days, state))
        .collect_view();
    let unsorted = (report.unsorted > Decimal::ZERO).then(|| {
        view! {
            <li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 py-3">
                <strong class="font-semibold">"Unsorted"</strong>
                <span class="text-sm tabular-nums text-fg-muted">
                    {format!("{} not sorted yet", money(report.unsorted, unit))}
                </span>
                <button
                    type="button"
                    class=button_class(Variant::Link, Size::Inline)
                    on:click=move |_| app.go(Tab::Transactions)
                >
                    "Review"
                </button>
            </li>
        }
    });
    let near = (!near.is_empty()).then(|| {
        view! {
            <li class="py-3 text-sm text-fg-muted">
                {format!("Near your norm ({}): {}", near.len(), near.join(" · "))}
            </li>
        }
    });
    let label = format!("{} adjustments", unit.code);
    let above = (above > Decimal::ZERO).then(|| {
        let text = format!("{} above your norm this month", whole(above, unit));
        view! { <strong class="text-sm font-semibold tabular-nums text-warning-fg">{text}</strong> }
    });
    let habits = habits_view(report);
    Some(
        view! {
            <section class="my-4 w-full min-w-0" aria-label=label>
                <Card>
                    <CardHeader>
                        <div class="grid gap-1">
                            <Eyebrow>"AGAINST YOUR OWN NORM"</Eyebrow>
                            <CardTitle>"Adjustments"</CardTitle>
                        </div>
                        {above}
                    </CardHeader>
                    <ul class="divide-y divide-border">{flagged}{unsorted}{near}</ul>
                    {habits}
                    <p class="mt-4 text-xs text-fg-muted">
                        "Typical is the median of your earlier months with spending. "
                        "Bills and other fixed costs are left out."
                    </p>
                </Card>
            </section>
        }
        .into_any(),
    )
}

/// Spending → Plan & patterns: every group against the owner's own norm.
#[component]
pub fn AdjustmentsCard() -> impl IntoView {
    let state = CardState {
        confirming: RwSignal::new(None),
        busy: RwSignal::new(false),
        error: RwSignal::new(None),
    };
    with_report(move |report| {
        report
            .adjustments
            .iter()
            .filter_map(|adjustments| card(adjustments, report, state))
            .collect_view()
    })
}

#[cfg(test)]
mod tests {
    use finance_types::gateway::{BudgetGroup, BudgetPlan};
    use finance_types::report::Currency;
    use finance_types::report::adjustments::{AdjustmentRow, Habit, StoreHabit};
    use finance_types::report::budget::BudgetRow;
    use rust_decimal::Decimal;

    use super::*;

    const DAYS: usize = 30;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    /// A row named "Shopping" whose earlier months are `earlier`.
    fn row(
        planned: &str,
        spent: &str,
        earlier: &[i64],
        elapsed: usize,
        visits: usize,
        budgeted: bool,
    ) -> AdjustmentRow {
        let spent = dec(spent);
        let mut history: Vec<Decimal> = earlier.iter().map(|n| Decimal::from(*n)).collect();
        history.push(spent);
        AdjustmentRow {
            row: BudgetRow {
                id: if budgeted {
                    "food-out"
                } else {
                    "shadow:Shopping"
                }
                .into(),
                name: if budgeted {
                    "Food & drinks out"
                } else {
                    "Shopping"
                }
                .into(),
                planned: dec(planned),
                spent,
                remaining: dec(planned) - spent,
                reserved: Decimal::ZERO,
                scheduled: Decimal::ZERO,
                categories: Vec::new(),
                history,
                daily_spending: Vec::new(),
                elapsed_days: elapsed,
                expected: Vec::new(),
            },
            budgeted,
            group: (!budgeted).then(|| "Shopping".into()),
            habit: Habit {
                visits,
                spent,
                ..Habit::default()
            },
            stores: Vec::new(),
        }
    }

    fn store(category: &str, visits: usize, spent: i64, typical_spent: Option<i64>) -> StoreHabit {
        StoreHabit {
            category: category.into(),
            habit: Habit {
                visits,
                spent: Decimal::from(spent),
                typical_visits: typical_spent.map(|_| Decimal::TWO),
                typical_spent: typical_spent.map(Decimal::from),
                ..Habit::default()
            },
        }
    }

    fn plan() -> BudgetPlan {
        let group = |id: &str, amount: i64| BudgetGroup {
            id: id.into(),
            name: id.into(),
            amount: Decimal::from(amount),
            source_groups: vec![id.into()],
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance: None,
            floor: None,
        };
        BudgetPlan {
            month: "2026-11-01".into(),
            currency: Currency::eur(),
            revision: 2,
            groups: vec![group("groceries", 300), group("food-out", 100)],
        }
    }

    #[test]
    fn typical_needs_three_earlier_months_with_spending() {
        assert_eq!(
            typical(&row("0", "90", &[0, 400, 430, 0, 460], 15, 2, false).row),
            Some(dec("430"))
        );
        assert_eq!(
            typical(&row("0", "90", &[0, 0, 400, 430], 15, 2, false).row),
            None
        );
    }

    #[test]
    fn above_norm_at_the_boundary() {
        let earlier = [400, 400, 400];
        assert_eq!(
            above_norm(&row("0", "250", &earlier, 15, 5, false), DAYS),
            Some(dec("100"))
        );
        assert_eq!(
            above_norm(&row("0", "249.5", &earlier, 15, 5, false), DAYS),
            None
        );
        // A quarter above, but only 20 in money.
        assert_eq!(
            above_norm(&row("0", "40", &[60, 60, 60], 15, 5, false), DAYS),
            None
        );
    }

    #[test]
    fn too_early_gives_no_heading_or_norm_finding_but_keeps_budget_fit() {
        let earlier = [430, 430, 430];
        for (elapsed, visits) in [(6, 9), (20, 1)] {
            let early = row("100", "600", &earlier, elapsed, visits, true);
            assert!(too_early(&early));
            assert_eq!(heading(&early, DAYS), None);
            assert_eq!(above_norm(&early, DAYS), None);
            assert_eq!(budget_fit(&early), Some(BudgetFit::Low));
        }
        assert!(!too_early(&row("100", "600", &earlier, 7, 2, true)));
    }

    #[test]
    fn budget_fit_boundaries() {
        let fit = |planned: &str, earlier: &[i64], budgeted| {
            budget_fit(&row(planned, "0", earlier, 15, 2, budgeted))
        };
        let usual = [430, 430, 430];
        assert_eq!(fit("344", &usual, true), Some(BudgetFit::Low));
        assert_eq!(fit("345", &usual, true), None);
        assert_eq!(fit("516", &usual, true), Some(BudgetFit::High));
        assert_eq!(fit("515", &usual, true), None);
        // A fifth above, but only 20 in money.
        assert_eq!(fit("120", &[100, 100, 100], true), None);
        assert_eq!(fit("0", &usual, false), None);
        assert_eq!(fit("100", &[430, 430], true), None);
    }

    #[test]
    fn a_row_can_be_above_norm_and_under_budgeted() {
        let hot = row("100", "290", &[430, 430, 430], 15, 9, true);
        assert_eq!(heading(&hot, DAYS), Some(dec("580")));
        assert_eq!(above_norm(&hot, DAYS), Some(dec("150")));
        assert_eq!(action(&hot, DAYS), Some(Action::Set(dec("430"))));
    }

    #[test]
    fn actions_follow_the_findings() {
        let spare = row("600", "100", &[430, 430, 430], 15, 4, true);
        assert_eq!(action(&spare, DAYS), Some(Action::Lower(dec("430"))));
        let drifting = row("0", "200", &[150, 148, 152], 15, 5, false);
        assert_eq!(action(&drifting, DAYS), Some(Action::Add(dec("150"))));
        let steady = row("0", "75", &[150, 150, 150], 15, 4, false);
        assert_eq!(action(&steady, DAYS), None);
        let fitting = row("430", "290", &[430, 430, 430], 15, 9, true);
        assert_eq!(action(&fitting, DAYS), None);
    }

    #[test]
    fn a_refund_heavy_row_has_no_finding() {
        let mut refunded = row("0", "-20", &[150, 150, 150], 15, 2, false);
        refunded.stores = vec![store("Zara", 0, -20, Some(40))];
        assert_eq!(heading(&refunded, DAYS), Some(Decimal::ZERO));
        assert_eq!(above_norm(&refunded, DAYS), None);
        assert_eq!(action(&refunded, DAYS), None);
        assert!(drivers(&refunded, DAYS).is_empty());
    }

    #[test]
    fn round_to_ten_rounds_half_up() {
        for (amount, rounded) in [("425", "430"), ("424.99", "420"), ("4", "0"), ("5", "10")] {
            assert_eq!(round_to_ten(dec(amount)), dec(rounded));
        }
    }

    #[test]
    fn apply_sets_and_adds_without_touching_other_groups() {
        let plan = plan();
        let food = row("100", "290", &[430, 430, 430], 15, 9, true);
        let set = apply(&plan, &food, &Action::Set(dec("430")));
        assert_eq!(set.groups[1].amount, dec("430"));
        assert_eq!((&set.groups[0], set.revision), (&plan.groups[0], 2));
        assert_eq!(set.groups.len(), 2);

        let shopping = row("0", "200", &[150, 150, 150], 15, 4, false);
        let added = apply(&plan, &shopping, &Action::Add(dec("150")));
        assert_eq!(added.groups[..2], plan.groups[..]);
        let new = &added.groups[2];
        assert_eq!(
            (new.id.as_str(), new.name.as_str()),
            ("shopping", "Shopping")
        );
        assert_eq!(
            (new.amount, &new.source_groups),
            (dec("150"), &vec!["Shopping".to_owned()])
        );
    }

    #[test]
    fn an_added_group_avoids_a_taken_id() {
        let mut plan = plan();
        plan.groups[0].id = "shopping".into();
        plan.groups[1].id = "shopping-2".into();
        let shopping = row("0", "200", &[150, 150, 150], 15, 4, false);
        let added = apply(&plan, &shopping, &Action::Add(dec("150")));
        assert_eq!(added.groups[2].id, "shopping-3");
        assert_eq!(slug("Health & personal care"), "health-personal-care");
    }

    #[test]
    fn drivers_rank_by_pace_adjusted_excess_and_include_new_stores() {
        let mut hot = row("0", "170", &[150, 150, 150], 15, 6, false);
        hot.stores = vec![
            store("B", 2, 40, Some(100)),
            store("C", 3, 30, None),
            store("A", 4, 100, Some(60)),
            store("D", 1, 12, None),
            store("E", 1, 11, None),
        ];
        let found = drivers(&hot, DAYS);
        let names: Vec<&str> = found.iter().map(|d| d.category.as_str()).collect();
        assert_eq!(names, ["A", "C", "D"]);
        assert_eq!(found[0].usual, Some((Decimal::TWO, dec("60"))));
        assert_eq!(
            (found[1].visits, found[1].spent, found[1].usual),
            (3, dec("30"), None)
        );
    }

    #[test]
    fn mostly_names_up_to_two_weekdays() {
        assert_eq!(mostly(&[0, 0, 0, 0, 5, 5, 1]), "Fri, Sat");
        assert_eq!(mostly(&[1, 0, 9, 4, 0, 5, 1]), "Wed, Sat");
        assert_eq!(mostly(&[2, 2, 2, 2, 2, 2, 2]), "any day");
        assert_eq!(mostly(&[0; 7]), "any day");
    }

    #[test]
    fn busy_week_and_shown_thresholds() {
        let habit = |week, usual: &str, recent| Habit {
            visits_this_week: week,
            typical_week_visits: dec(usual),
            recent_visits: recent,
            ..Habit::default()
        };
        assert!(busy_week(&habit(4, "2", 0)));
        assert!(!busy_week(&habit(3, "2", 0)));
        assert!(!busy_week(&habit(3, "1.5", 0)));
        assert!(shown(&habit(0, "0", 4)));
        assert!(!shown(&habit(0, "0", 3)));
    }

    #[test]
    fn a_few_purchases_without_a_budget_are_not_extrapolated() {
        // One concert is not a daily habit.
        let lumpy = row("0", "228", &[7, 144, 7], 19, 3, false);
        assert_eq!(heading(&lumpy, DAYS), Some(dec("228")));
        assert_eq!(above_norm(&lumpy, DAYS), Some(dec("221")));
        // Overview projects budget rows at their daily pace; so does the card.
        let budgeted = row("100", "228", &[7, 144, 7], 19, 3, true);
        assert_eq!(heading(&budgeted, DAYS), Some(dec("360")));
    }

    #[test]
    fn a_budget_too_small_to_matter_is_not_offered() {
        let lumpy = row("0", "228", &[7, 144, 7], 19, 3, false);
        assert_eq!(action(&lumpy, DAYS), None);
    }

    #[test]
    fn an_added_group_avoids_a_taken_name() {
        // A real "Shopping" group that claims by category leaves the rest of
        // the group uncovered; the plan rejects two groups of one name.
        let mut plan = plan();
        plan.groups[0].name = "shopping".into();
        let shopping = row("0", "200", &[150, 150, 150], 15, 5, false);
        let added = apply(&plan, &shopping, &Action::Add(dec("150")));
        let new = &added.groups[2];
        assert_eq!(new.name, "Shopping (rest)");
        assert_eq!(new.source_groups, ["Shopping"]);
    }

    #[test]
    fn a_store_seen_once_before_is_not_called_new() {
        let mut hot = row("0", "170", &[150, 150, 150], 15, 6, false);
        let mut once = store("Zara", 2, 80, None);
        once.habit.history_month_count = 1;
        hot.stores = vec![once];
        let found = drivers(&hot, DAYS);
        assert!(!found[0].new);
        assert_eq!(
            driver_text(&found[0], &Currency::eur()),
            "Zara: 2 visits, €80.00"
        );
        hot.stores = vec![store("Zara", 2, 80, None)];
        assert!(drivers(&hot, DAYS)[0].new);
    }
}
