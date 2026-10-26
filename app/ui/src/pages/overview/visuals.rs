//! The money split donut and the spending trajectory (`budget-visuals.tsx`).

use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::budget::{BudgetRow, MonthlyBudget};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{Donut, LineChart, Series, Slice, fmt, series_color};
use crate::ds::{CardHeader, CardTitle, Eyebrow, Heading, Money, Select, Stat, cn, money};
use crate::format;
use crate::pages::f;

/// The site's `Card` classes without the app's glass layer (these sit flat
/// inside the budget panel).
const PLAIN_CARD: &str = "rounded-xl border border-border bg-card-strong p-5 text-fg sm:p-6";

pub const BUDGET_COLORS: [&str; 6] = [
    "var(--ds-chart-1)",
    "var(--ds-chart-2)",
    "var(--ds-chart-3)",
    "var(--ds-chart-4)",
    "var(--ds-chart-5)",
    "var(--ds-chart-6)",
];

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

    // Money split.
    let mut pieces: Vec<(String, f64, String)> = vec![(
        "Bills left".into(),
        f(budget.fixed.remaining),
        series_color(0, Some("fixed")),
    )];
    pieces.extend(budget.groups.iter().enumerate().map(|(i, g)| {
        (
            g.name.clone(),
            f(g.reserved),
            BUDGET_COLORS[i % BUDGET_COLORS.len()].to_owned(),
        )
    }));
    if let Some(a) = &allocation {
        pieces.push((
            "Extra payments".into(),
            f(if current {
                a.current_plans_remaining
            } else {
                a.next_plans
            }),
            "var(--ds-chart-5)".into(),
        ));
        if !current {
            pieces.push((
                "Savings goals".into(),
                f(a.next_goals),
                "var(--ds-chart-6)".into(),
            ));
        }
        pieces.push((
            "Projected savings".into(),
            f(if current {
                a.current_planned_wiggle_room
            } else {
                a.next_wiggle_room
            })
            .max(0.0),
            series_color(0, Some("free")),
        ));
    }
    pieces.retain(|p| p.1 > 0.0);
    let total: f64 = pieces.iter().map(|p| p.1).sum();
    let fmt_money = {
        let unit = unit.clone();
        fmt(move |n| money(Decimal::try_from(n).unwrap_or_default(), &unit))
    };
    let split = if pieces.is_empty() {
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
                            <small class="text-[0.65rem] font-normal leading-tight text-fg-muted">
                                {if current { "Remaining allocation" } else { "Planned allocation" }}
                            </small>
                            <Money value=total_value unit=center_unit.clone() neutral=true class="text-lg" />
                        </span>
                    }
                    .into_any()
                })
            />
            <ul class="mt-4 grid gap-2 text-sm">{list}</ul>
        }
        .into_any()
    };

    // Spending trajectory.
    let mut plotted: Vec<BudgetRow> = budget.groups.clone();
    let has_extras = budget.extras.planned > Decimal::ZERO || budget.extras.spent > Decimal::ZERO;
    if has_extras {
        plotted.push(budget.extras.clone());
    }
    let all_label = if has_extras {
        "Budgets & extra payments"
    } else {
        "All budget groups"
    };
    let mut options = vec![("all".to_owned(), all_label.to_owned())];
    options.extend(plotted.iter().map(|g| (g.id.clone(), g.name.clone())));
    let plotted = StoredValue::new(plotted);
    let month = budget.plan.month.clone();
    let days = format::days_in_month(&month) as usize;
    let trajectory = move || {
        let selected_id = selection.get();
        plotted.with_value(|plotted| {
            let selected = plotted.iter().find(|g| g.id == selected_id);
            let rows: Vec<&BudgetRow> = match selected {
                Some(s) => vec![s],
                None => plotted.iter().collect(),
            };
            let planned: Decimal = rows.iter().map(|r| r.planned).sum();
            let spent: Decimal = rows.iter().map(|r| r.spent).sum();
            let elapsed = rows.iter().map(|r| r.elapsed_days).max().unwrap_or(0);
            let scheduled: Decimal = rows.iter().map(|r| r.scheduled).sum();
            let projected = (elapsed > 0).then(|| {
                rows.iter()
                    .map(|r| {
                        let pace = r.spent / Decimal::from(elapsed) * Decimal::from(days);
                        pace.max(r.spent + r.scheduled).max(Decimal::ZERO)
                    })
                    .sum::<Decimal>()
            });
            let over = projected.is_some_and(|p| p > planned);
            let mut cumulative = vec![0.0_f64];
            let mut running = Decimal::ZERO;
            for day in 0..elapsed {
                running += rows
                    .iter()
                    .map(|r| r.daily_spending.get(day).copied().unwrap_or_default())
                    .sum::<Decimal>();
                cumulative.push(f(running));
            }
            let planned_n = f(planned);
            let spent_n = f(spent);
            let show_projection = elapsed > 0 && elapsed < days && projected.is_some();
            let categories: Vec<String> = (0..=days).map(|d| d.to_string()).collect();
            let mut series = vec![Series {
                key: "spent".into(),
                label: "Actual".into(),
                color: "var(--ds-chart-1)".into(),
                values: (0..=days)
                    .map(|d| (d <= elapsed).then(|| cumulative[d]))
                    .collect(),
            }];
            if show_projection {
                let p = projected.map(f).unwrap_or_default();
                series.push(Series {
                    key: "projected".into(),
                    label: "Projected pace".into(),
                    color: if over {
                        "var(--ds-negative)".into()
                    } else {
                        "var(--ds-positive)".into()
                    },
                    values: (0..=days)
                        .map(|d| {
                            (d >= elapsed).then(|| {
                                if d == elapsed {
                                    spent_n
                                } else {
                                    spent_n
                                        + (p - spent_n) * (d - elapsed) as f64
                                            / (days - elapsed) as f64
                                }
                            })
                        })
                        .collect(),
                });
            }
            series.push(Series {
                key: "limit".into(),
                label: "Monthly limit".into(),
                color: series_color(0, Some("limit")),
                values: vec![Some(planned_n); days + 1],
            });
            series.push(Series {
                key: "even".into(),
                label: "Even budget pace".into(),
                color: "var(--ds-chart-fixed)".into(),
                values: (0..=days)
                    .map(|d| Some(planned_n * d as f64 / days as f64))
                    .collect(),
            });
            let name = selected.map_or("Your planned spending".to_owned(), |s| s.name.clone());
            let aria = format!(
                "{}: {} spent against {} planned. {}",
                selected.map_or("Budget groups".to_owned(), |s| s.name.clone()),
                m(spent),
                m(planned),
                match projected {
                    None => "No spending trajectory yet.".to_owned(),
                    Some(p) => format!(
                        "Projected {} using the daily average and scheduled payments. This is an estimate.",
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
                        height=240
                        categories=categories
                        series=series
                        label=aria
                        format=fmt(move |n| format::money_dp(Decimal::try_from(n).unwrap_or_default(), &code, 0))
                    />
                }
                .into_any()
            };
            let scheduled_note = if scheduled > Decimal::ZERO {
                ", including scheduled payments"
            } else {
                ""
            };
            let note = match projected {
                _ if elapsed == 0 => "Ready for the coming month.".to_owned(),
                Some(p) if over => format!("{} above budget at your current pace{scheduled_note}.", m(p - planned)),
                Some(p) => format!("{} below budget at your current pace{scheduled_note}.", m(planned - p)),
                None => String::new(),
            };
            let note = if elapsed > 0 {
                format!("{note} Projection uses your daily average and scheduled payments.")
            } else {
                format!("{note} ")
            };
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
                <div class="grid grid-cols-3 gap-4">
                    <Stat label="Spent">{spent_view}</Stat>
                    <Stat label="Month-end pace">{pace}</Stat>
                    <Stat label="Budget">{budget_view}</Stat>
                </div>
                <div class="mt-4">{chart}</div>
                <p class="mt-4 text-xs leading-relaxed text-fg-muted">{note}</p>
            }
        })
    };

    view! {
        <div class="grid border-b border-border lg:grid-cols-[minmax(260px,.85fr)_minmax(0,1.25fr)]">
            <div class=cn([PLAIN_CARD, "min-w-0 rounded-none border-0 bg-transparent lg:border-r lg:border-border"])>
                <CardHeader class="mb-2 block">
                    <Eyebrow level=Heading::P>"MONEY SPLIT"</Eyebrow>
                    <CardTitle>{if current { "What's left this month" } else { "Next month's allocation" }}</CardTitle>
                </CardHeader>
                {split}
            </div>
            <div class=cn([PLAIN_CARD, "min-w-0 rounded-none border-0 bg-transparent"])>{trajectory}</div>
        </div>
    }
}
