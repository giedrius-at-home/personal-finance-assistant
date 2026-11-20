//! How the month is going (or next month's money split) and the spending
//! trajectory (`budget-visuals.tsx`).

use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::budget::{BudgetRow, MonthlyBudget};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{Donut, Line, LineChart, Series, Slice, fmt, series_color};
use crate::ds::{
    CardHeader, CardTitle, Eyebrow, Facts, Heading, METRIC_STRIP, Metric, Money, Select, Stat,
    Tone, cn, money,
};
use crate::format;
use crate::pages::f;

/// The site's `Card` classes without the app's glass layer (these sit flat
/// inside the budget panel).
const PLAIN_CARD: &str = "text-fg";

/// Budget groups cycle through these; the other rows keep colours of their own.
const GROUP_COLORS: [&str; 4] = [
    "var(--ds-chart-1)",
    "var(--ds-chart-2)",
    "var(--ds-chart-3)",
    "var(--ds-chart-4)",
];

/// One colour per budget row, the same in the bars, the donut and the table.
pub fn row_color(budget: &MonthlyBudget, id: &str) -> String {
    match budget.groups.iter().position(|g| g.id == id) {
        Some(index) => GROUP_COLORS[index % GROUP_COLORS.len()].to_owned(),
        None => match id {
            "fixed" => "var(--ds-chart-fixed)",
            "extras" => "var(--ds-chart-5)",
            _ => "var(--ds-chart-6)",
        }
        .to_owned(),
    }
}

/// How a row's spending runs through the month.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// A little most days: the daily average carries on to month end.
    Daily,
    /// One-off payments: counted once, when paid or due, never extrapolated.
    Lump,
}

/// What a row will have cost by month end, `None` before the month starts.
pub fn month_end(row: &BudgetRow, pace: Pace, days: usize) -> Option<Decimal> {
    if row.elapsed_days == 0 {
        return None;
    }
    let known = (row.spent + row.scheduled).max(Decimal::ZERO);
    Some(match pace {
        Pace::Lump => known,
        Pace::Daily => {
            let days = days.max(row.elapsed_days);
            (row.spent / Decimal::from(row.elapsed_days) * Decimal::from(days)).max(known)
        }
    })
}

/// The whole month's estimate: bills and estimates as planned, budget groups
/// at their daily pace, one-off payments once.
pub fn month_end_total(budget: &MonthlyBudget, days: usize) -> Option<Decimal> {
    let unplanned = &budget.unplanned;
    let unplanned_end = if unplanned.expected.is_empty() {
        month_end(unplanned, Pace::Daily, days)?
    } else {
        unplanned.planned.max(unplanned.spent)
    };
    let mut total = budget.fixed.planned.max(budget.fixed.spent)
        + month_end(&budget.extras, Pace::Lump, days)?.max(budget.extras.planned)
        + unplanned_end;
    for group in &budget.groups {
        total += month_end(group, Pace::Daily, days)?;
    }
    Some(total)
}

/// The plan's running total at the end of each day (index 0 is day 1):
/// budgets spread evenly, one-off payments where they fell and the unpaid
/// rest spread over the days left.
pub fn plan_pace(rows: &[(&BudgetRow, Pace)], days: usize) -> Vec<f64> {
    (1..=days)
        .map(|day| {
            rows.iter()
                .map(|(row, pace)| {
                    let planned = f(row.planned);
                    let elapsed = row.elapsed_days.min(days);
                    match pace {
                        Pace::Daily => planned * day as f64 / days as f64,
                        Pace::Lump if day <= elapsed => {
                            f(row.daily_spending.iter().take(day).copied().sum())
                        }
                        Pace::Lump => {
                            let spent = f(row.spent);
                            spent
                                + (planned - spent) * (day - elapsed) as f64
                                    / (days - elapsed) as f64
                        }
                    }
                })
                .sum()
        })
        .collect()
}

/// The month's verdict in numbers.
pub struct Verdict {
    /// Planned less spent; negative once the plan is overspent.
    pub net: Decimal,
    /// The month-end estimate less the plan, `None` before the month starts.
    pub ahead: Option<Decimal>,
    pub tone: Tone,
    pub word: &'static str,
}

pub fn verdict(budget: &MonthlyBudget, days: usize) -> Verdict {
    let net = budget.planned - budget.spent;
    let ahead = month_end_total(budget, days).map(|total| total - budget.planned);
    let (tone, word) = if net < Decimal::ZERO {
        (Tone::Negative, "Over plan")
    } else if ahead.is_some_and(|a| a > Decimal::ZERO) {
        (Tone::Warning, "Over pace")
    } else {
        (Tone::Positive, "On track")
    };
    Verdict {
        net,
        ahead,
        tone,
        word,
    }
}

/// `+€5.00` over the plan, `−€5.00` under it.
pub fn signed(value: Decimal, unit: &finance_types::report::Currency) -> String {
    if value > Decimal::ZERO {
        format!("+{}", money(value, unit))
    } else {
        money(value, unit)
    }
}

/// The current month: one verdict, then every row's spent, left and over.
fn on_track(budget: &MonthlyBudget, days: usize) -> AnyView {
    let unit = budget.plan.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let Verdict {
        net,
        ahead,
        tone,
        word,
    } = verdict(budget, days);
    let headline = if net < Decimal::ZERO {
        "Over plan"
    } else {
        "Left in plan"
    };
    let month_end = ahead.map(|a| {
        let class = if a > Decimal::ZERO {
            "text-negative-fg"
        } else {
            "text-positive-fg"
        };
        let text = signed(a, &unit);
        view! {
            <Metric label="Month end vs plan" size="text-3xl">
                <span class=class>{text}</span>
            </Metric>
        }
    });

    let mut rows: Vec<(&BudgetRow, &str)> = vec![(&budget.fixed, "to pay")];
    rows.extend(budget.groups.iter().map(|g| (g, "left")));
    rows.push((&budget.extras, "still due"));
    rows.push((
        &budget.unplanned,
        if budget.unplanned.expected.is_empty() {
            "left"
        } else {
            "expected"
        },
    ));
    rows.retain(|(r, _)| r.planned > Decimal::ZERO || r.spent > Decimal::ZERO);
    let mut left_total = Decimal::ZERO;
    let mut over_total = Decimal::ZERO;
    let bars = rows
        .iter()
        .map(|(row, word)| {
            let left = (row.planned - row.spent).max(Decimal::ZERO);
            let over = (row.spent - row.planned).max(Decimal::ZERO);
            left_total += left;
            over_total += over;
            let scale = f(row.planned.max(row.spent)).max(f64::EPSILON);
            let within = (f(row.spent.min(row.planned)).max(0.0) / scale * 100.0).clamp(0.0, 100.0);
            let beyond = (f(over) / scale * 100.0).clamp(0.0, 100.0);
            let status = if over > Decimal::ZERO {
                format!("{} over", m(over))
            } else if left.is_zero() {
                if row.id == "extras" { "all paid" } else { "all spent" }.to_owned()
            } else {
                format!("{} {word}", m(left))
            };
            let label = format!(
                "{}: {} spent of {} planned, {status}",
                row.name,
                m(row.spent),
                m(row.planned)
            );
            view! {
                <li class="grid gap-1.5">
                    <div class="flex items-baseline justify-between gap-3 text-sm">
                        <span class="min-w-0 truncate">{row.name.clone()}</span>
                        <span class=cn([
                            "whitespace-nowrap tabular-nums",
                            if over > Decimal::ZERO { "font-medium text-negative-fg" } else { "text-fg-muted" },
                        ])>{status}</span>
                    </div>
                    <div role="img" aria-label=label class="flex h-2 overflow-hidden rounded-pill bg-soft">
                        <i class="block h-full" style:width=format!("{within:.2}%") style:background=row_color(budget, &row.id)></i>
                        <i class="block h-full bg-negative" style:width=format!("{beyond:.2}%")></i>
                    </div>
                </li>
            }
        })
        .collect_view();
    let mut sums = vec![("Still planned".to_owned(), m(left_total))];
    if over_total > Decimal::ZERO {
        sums.push(("Already overspent".to_owned(), m(over_total)));
    }
    view! {
        <div class="grid grid-cols-2 gap-x-6 gap-y-4">
            <Metric label=headline chip=(tone, word.to_owned()) size="text-3xl">
                <Money value=net.abs() unit=unit.clone() neutral=true class=if net < Decimal::ZERO { "text-negative-fg" } else { "" } />
            </Metric>
            {month_end}
        </div>
        <ul class="mt-6 grid list-none gap-3.5 p-0">{bars}</ul>
        <Facts rows=sums class="mt-4 border-t border-line pt-3" />
    }
    .into_any()
}

#[component]
pub fn BudgetVisuals(
    budget: MonthlyBudget,
    allocation: Option<CurrencyAllocation>,
    current_month: String,
    /// The plotted group's id, or `all`.
    selection: RwSignal<String>,
) -> impl IntoView {
    let unit = budget.plan.currency.clone();
    let current = budget.plan.month == current_month;
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };

    let month = budget.plan.month.clone();
    let days = format::days_in_month(&month) as usize;

    // This month: how it is going. Next month: where the money goes.
    let split = if current {
        on_track(&budget, days)
    } else {
        let mut pieces: Vec<(String, f64, String)> = vec![(
            budget.fixed.name.clone(),
            f(budget.fixed.remaining),
            row_color(&budget, "fixed"),
        )];
        pieces.extend(
            budget
                .groups
                .iter()
                .map(|g| (g.name.clone(), f(g.reserved), row_color(&budget, &g.id))),
        );
        if let Some(a) = &allocation {
            pieces.push((
                budget.extras.name.clone(),
                f(a.next_plans),
                row_color(&budget, "extras"),
            ));
            pieces.push((
                "Savings goals".into(),
                f(a.next_goals),
                "var(--ds-chart-6)".into(),
            ));
            pieces.push((
                "Unallocated".into(),
                f(a.next_wiggle_room).max(0.0),
                series_color(0, Some("free")),
            ));
        }
        pieces.retain(|p| p.1 > 0.0);
        let total: f64 = pieces.iter().map(|p| p.1).sum();
        let fmt_money = {
            let unit = unit.clone();
            fmt(move |n| money(Decimal::try_from(n).unwrap_or_default(), &unit))
        };
        if pieces.is_empty() {
            view! { <p role="img" aria-label="No money allocated" class="py-10 text-center text-sm text-fg-muted">"No money allocated"</p> }
                .into_any()
        } else {
            let label = pieces
                .iter()
                .map(|(l, v, _)| format!("{l}: {}", fmt_money(*v)))
                .collect::<Vec<_>>()
                .join(", ");
            let slices = pieces
                .iter()
                .enumerate()
                .map(|(i, (l, v, c))| Slice {
                    key: format!("piece-{i}"),
                    label: l.clone(),
                    value: *v,
                    color: c.clone(),
                })
                .collect();
            let list = pieces
                .iter()
                .map(|(l, v, _)| {
                    view! {
                        <li class="flex items-baseline justify-between gap-3">
                            <span class="min-w-0 text-fg-muted">{l.clone()}</span>
                            <strong class="font-medium tabular-nums">{fmt_money(*v)}</strong>
                        </li>
                    }
                })
                .collect_view();
            let center_unit = unit.clone();
            let total_value = Decimal::try_from(total).unwrap_or_default();
            view! {
                <Donut
                    height=320
                    data=slices
                    format=fmt_money.clone()
                    label=label
                    center=Box::new(move || {
                        view! {
                            <span class="grid gap-0.5">
                                <small class="text-[0.65rem] font-normal leading-tight text-fg-muted">"Planned allocation"</small>
                                <Money value=total_value unit=center_unit.clone() neutral=true class="text-lg" />
                            </span>
                        }
                        .into_any()
                    })
                />
                <ul class="mt-4 grid gap-2 text-sm">{list}</ul>
            }
            .into_any()
        }
    };

    // Spending trajectory.
    let mut plotted: Vec<(BudgetRow, Pace)> = budget
        .groups
        .iter()
        .map(|g| (g.clone(), Pace::Daily))
        .collect();
    let has_extras = budget.extras.planned > Decimal::ZERO || budget.extras.spent > Decimal::ZERO;
    if has_extras {
        plotted.push((budget.extras.clone(), Pace::Lump));
    }
    let all_label = if has_extras {
        "Budgets & extra payments"
    } else {
        "All budget groups"
    };
    let mut options = vec![("all".to_owned(), all_label.to_owned())];
    options.extend(plotted.iter().map(|(g, _)| (g.id.clone(), g.name.clone())));
    let plotted = StoredValue::new(plotted);
    let trajectory = move || {
        let selected_id = selection.get();
        plotted.with_value(|plotted| {
            let selected = plotted.iter().find(|(g, _)| g.id == selected_id);
            let rows: Vec<(&BudgetRow, Pace)> = match selected {
                Some((row, pace)) => vec![(row, *pace)],
                None => plotted.iter().map(|(row, pace)| (row, *pace)).collect(),
            };
            let selected = selected.map(|(row, _)| row);
            let planned: Decimal = rows.iter().map(|(r, _)| r.planned).sum();
            let spent: Decimal = rows.iter().map(|(r, _)| r.spent).sum();
            let elapsed = rows.iter().map(|(r, _)| r.elapsed_days).max().unwrap_or(0);
            // A closure can start the budget month before the calendar one.
            let days = days.max(elapsed);
            let scheduled: Decimal = rows.iter().map(|(r, _)| r.scheduled).sum();
            let lumps = rows.iter().any(|(_, pace)| *pace == Pace::Lump);
            let projected = rows
                .iter()
                .map(|(r, pace)| month_end(r, *pace, days))
                .sum::<Option<Decimal>>()
                .filter(|_| elapsed > 0);
            let over = projected.is_some_and(|p| p > planned);
            // Running total at the end of each day; index 0 is day 1.
            let mut cumulative = Vec::with_capacity(elapsed);
            let mut running = Decimal::ZERO;
            for day in 0..elapsed {
                running += rows
                    .iter()
                    .map(|(r, _)| r.daily_spending.get(day).copied().unwrap_or_default())
                    .sum::<Decimal>();
                cumulative.push(f(running));
            }
            let planned_n = f(planned);
            let spent_n = f(spent);
            let show_projection = elapsed > 0 && elapsed < days && projected.is_some();
            let categories: Vec<String> = (1..=days).map(|d| d.to_string()).collect();
            let titles: Vec<String> = (1..=days).map(|d| format!("Day {d}")).collect();
            let mut series = vec![Series {
                key: "spent".into(),
                label: "Actual".into(),
                color: "var(--ds-chart-1)".into(),
                values: (1..=days).map(|d| cumulative.get(d - 1).copied()).collect(),
                line: Line::Step,
            }];
            if show_projection {
                let p = projected.map(f).unwrap_or_default();
                series.push(Series {
                    key: "projected".into(),
                    label: "Projected".into(),
                    color: if over {
                        "var(--ds-negative)".into()
                    } else {
                        "var(--ds-positive)".into()
                    },
                    values: (1..=days)
                        .map(|d| {
                            (d >= elapsed).then(|| {
                                spent_n
                                    + (p - spent_n) * (d - elapsed) as f64
                                        / (days - elapsed) as f64
                            })
                        })
                        .collect(),
                    line: Line::Straight,
                });
            }
            series.push(Series {
                key: "pace".into(),
                label: "Budget pace".into(),
                color: "var(--ds-chart-fixed)".into(),
                values: plan_pace(&rows, days).into_iter().map(Some).collect(),
                line: Line::Straight,
            });
            series.push(Series {
                key: "limit".into(),
                label: "Budget".into(),
                color: "var(--ds-fg-muted)".into(),
                values: vec![Some(planned_n); days],
                line: Line::Dashed,
            });
            let today = (current && elapsed > 0 && elapsed < days)
                .then(|| (elapsed - 1, "Today".to_owned()));
            let name = selected.map_or("Your planned spending".to_owned(), |s| s.name.clone());
            let aria = format!(
                "{}: {} spent against {} planned. {}",
                selected.map_or("Budget groups".to_owned(), |s| s.name.clone()),
                m(spent),
                m(planned),
                match projected {
                    None => "No spending trajectory yet.".to_owned(),
                    Some(p) => format!(
                        "Projected {} by month end from the daily average and scheduled payments. This is an estimate.",
                        m(p)
                    ),
                }
            );
            let code = unit.code.clone();
            let chart = if elapsed == 0 {
                view! { <p class="py-10 text-center text-sm text-fg-muted">"Your trajectory starts with this month's spending"</p> }
                    .into_any()
            } else {
                view! {
                    <LineChart
                        height=300
                        categories=categories
                        titles=titles
                        series=series
                        label=aria
                        marker=today
                        format=fmt(move |n| format::money_dp(Decimal::try_from(n).unwrap_or_default(), &code, 0))
                    />
                }
                .into_any()
            };
            // How the estimate is made stays a tooltip on its field.
            let method = match (lumps, scheduled > Decimal::ZERO) {
                (true, _) => "Budgets continue at your daily average; one-off payments count once.",
                (false, true) => "Your daily average continues; scheduled payments are included.",
                (false, false) => "Your daily average continues to month end.",
            };
            let gap = projected.filter(|_| elapsed > 0).map(|p| {
                let class = if over { "text-negative-fg" } else { "text-positive-fg" };
                let text = signed(p - planned, &unit);
                view! {
                    <Stat label="vs budget">
                        <span class=class>{text}</span>
                    </Stat>
                }
            });
            let pace = match projected {
                None => view! { "—" }.into_any(),
                Some(p) => view! {
                    <Money value=p.round_dp(u32::from(unit.minor_unit)) unit=unit.clone() class=if over { "text-negative-fg" } else { "" } />
                }
                .into_any(),
            };
            let spent_view = view! { <Money value=spent unit=unit.clone() /> }.into_any();
            let budget_view = view! { <Money value=planned unit=unit.clone() /> }.into_any();
            let options = options.clone();
            view! {
                <CardHeader class="flex-wrap">
                    <div class="min-w-0">
                        <Eyebrow level=Heading::P>"SPENDING TRAJECTORY"</Eyebrow>
                        <CardTitle>{name}</CardTitle>
                    </div>
                    <div class="w-52 max-w-full">
                        <Select label="Spending trajectory group" class="h-9 text-xs" value=selection options=options />
                    </div>
                </CardHeader>
                <div class=cn([METRIC_STRIP, "md:grid-cols-4"])>
                    <Stat label="Spent">{spent_view}</Stat>
                    <div title=method><Stat label="Month-end estimate">{pace}</Stat></div>
                    <Stat label="Budget">{budget_view}</Stat>
                    {gap}
                </div>
                <div class="mt-5">{chart}</div>
            }
        })
    };

    view! {
        <div class="grid gap-x-10 gap-y-8 border-y border-line py-6 lg:grid-cols-[minmax(280px,.55fr)_minmax(0,1.45fr)]">
            <div class=cn([PLAIN_CARD, "min-w-0 lg:border-r lg:border-line lg:pr-10"])>
                <CardHeader class="mb-4 block">
                    <Eyebrow level=Heading::P>{if current { "THIS MONTH" } else { "MONEY SPLIT" }}</Eyebrow>
                    <CardTitle>{if current { "Are you on track?" } else { "Next month's allocation" }}</CardTitle>
                </CardHeader>
                {split}
            </div>
            <div class=cn([PLAIN_CARD, "min-w-0"])>{trajectory}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn row(id: &str, planned: &str, daily: &[&str], scheduled: &str) -> BudgetRow {
        let daily: Vec<Decimal> = daily.iter().map(|d| dec(d)).collect();
        let spent: Decimal = daily.iter().copied().sum();
        BudgetRow {
            id: id.into(),
            name: id.into(),
            planned: dec(planned),
            spent,
            remaining: dec(planned) - spent,
            reserved: Decimal::ZERO,
            scheduled: dec(scheduled),
            categories: Vec::new(),
            history: Vec::new(),
            elapsed_days: daily.len(),
            daily_spending: daily,
            expected: Vec::new(),
            to_events: Vec::new(),
        }
    }

    #[test]
    fn a_one_off_payment_is_not_extrapolated() {
        let extras = row("extras", "900", &["0", "600", "0"], "300");
        assert_eq!(month_end(&extras, Pace::Lump, 30), Some(dec("900")));
        // The daily average would have called it 6000.
        assert_eq!(month_end(&extras, Pace::Daily, 30), Some(dec("6000")));
    }

    #[test]
    fn a_budget_runs_on_at_its_daily_average() {
        let food = row("food", "100", &["10", "0", "20"], "0");
        assert_eq!(month_end(&food, Pace::Daily, 30), Some(dec("300")));
        // Scheduled payments are a floor, not an addition.
        let quiet = row("quiet", "100", &["1", "0", "0"], "50");
        assert_eq!(month_end(&quiet, Pace::Daily, 30), Some(dec("51")));
        assert_eq!(
            month_end(&row("new", "100", &[], "0"), Pace::Daily, 30),
            None
        );
    }

    #[test]
    fn the_plan_pace_steps_with_one_off_payments_and_ends_at_the_plan() {
        let food = row("food", "300", &["10", "0", "20"], "0");
        let extras = row("extras", "900", &["0", "600", "0"], "300");
        let pace = plan_pace(&[(&food, Pace::Daily), (&extras, Pace::Lump)], 30);
        assert_eq!(pace.len(), 30);
        assert!((pace[0] - 10.0).abs() < 1e-9);
        assert!((pace[1] - 620.0).abs() < 1e-9);
        assert!((pace[2] - 630.0).abs() < 1e-9);
        assert!((pace[29] - 1200.0).abs() < 1e-9);
    }
}
