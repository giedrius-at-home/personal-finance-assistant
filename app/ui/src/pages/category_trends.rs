//! "Compare categories": up to three categories plotted over the months
//! (personal-site `category-trends.tsx`).

use finance_types::gateway::GroupMapping;
use finance_types::report::Currency;
use finance_types::report::trends::{CategoryTrend, CurrencyTrends, MonthSpending};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{LineChart, Series, fmt};
use crate::ds::{
    Badge, Card, CardHeader, CardTitle, CardTone, EmptyState, Eyebrow, Heading, Money,
    SegmentedControl, Size, Tone, Variant, button_class, cn, money,
};
use crate::format;
use crate::pages::f;

/// Three lines is the most the finance palette keeps apart for colour-blind readers.
const COLORS: [&str; 3] = [
    "var(--ds-chart-1)",
    "var(--ds-chart-2)",
    "var(--ds-chart-3)",
];
const UNCATEGORISED: &str = "__uncategorised__";

fn key(row: &CategoryTrend) -> String {
    row.category.clone().unwrap_or_else(|| UNCATEGORISED.into())
}

fn name(row: &CategoryTrend) -> String {
    row.category
        .clone()
        .unwrap_or_else(|| "Uncategorised".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    MonthlySpend,
    DailyPace,
    PaceChange,
}

impl Metric {
    const ALL: [Self; 3] = [Self::MonthlySpend, Self::DailyPace, Self::PaceChange];

    fn id(self) -> &'static str {
        match self {
            Self::MonthlySpend => "monthly_spend",
            Self::DailyPace => "daily_pace",
            Self::PaceChange => "pace_change",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MonthlySpend => "Monthly spend",
            Self::DailyPace => "Daily pace",
            Self::PaceChange => "Pace change",
        }
    }

    fn note(self, code: &str) -> String {
        match self {
            Self::MonthlySpend => "Net spending recorded in each month.".into(),
            Self::DailyPace => format!("Average net spending per calendar day, in {code} / day."),
            Self::PaceChange => {
                format!("Difference in daily pace from the previous month, in {code} / day.")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Momentum {
    pub daily_pace: Decimal,
    pub projected: Decimal,
    pub change: Option<Decimal>,
    pub pace_change: Option<Decimal>,
    pub direction: &'static str,
}

fn days_of(year_month: &str, offset: i32) -> Decimal {
    Decimal::from(format::days_in_month(&format::add_months(
        year_month, offset,
    )))
}

/// `categoryMomentum`: this month's pace and projection against last month.
pub fn momentum(row: &CategoryTrend, through: &str, partial: bool) -> Momentum {
    let latest = row.values.last().copied().flatten().unwrap_or_default();
    let elapsed = Decimal::from(
        through
            .get(8..10)
            .and_then(|d| d.parse::<u32>().ok())
            .unwrap_or(1)
            .max(1),
    );
    let month = &through[..7];
    let current_days = days_of(month, 0);
    let previous_days = days_of(month, -1);
    let latest_pace = latest / if partial { elapsed } else { current_days };
    let projected = if partial {
        row.projected_month_end
    } else {
        latest
    };
    let n = row.values.len();
    let previous = n.checked_sub(2).and_then(|i| row.values[i]);
    let earlier = n.checked_sub(3).and_then(|i| row.values[i]);
    let Some(previous) = previous else {
        return Momentum {
            daily_pace: latest_pace,
            projected,
            change: None,
            pace_change: None,
            direction: "Building history",
        };
    };
    let pace_change = latest_pace - previous / previous_days;
    let steady = ((previous / previous_days).abs() * Decimal::new(8, 2)).max(Decimal::new(10, 2));
    Momentum {
        daily_pace: latest_pace,
        projected,
        change: Some(projected - previous),
        pace_change: Some(pace_change),
        direction: if earlier.is_none() {
            "Building history"
        } else if pace_change > steady {
            "Increasing pace"
        } else if pace_change < -steady {
            "Decreasing pace"
        } else {
            "Stable pace"
        },
    }
}

fn days_observed(month: &MonthSpending, through: &str) -> f64 {
    if month.partial_month {
        f64::from(
            through
                .get(8..10)
                .and_then(|d| d.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1),
        )
    } else {
        f64::from(format::days_in_month(&month.start))
    }
}

fn plot_values(
    row: &CategoryTrend,
    months: &[MonthSpending],
    through: &str,
    metric: Metric,
) -> Vec<Option<f64>> {
    if metric == Metric::MonthlySpend {
        return row.values.iter().map(|v| v.map(f)).collect();
    }
    let speeds: Vec<Option<f64>> = row
        .values
        .iter()
        .enumerate()
        .map(|(i, v)| v.map(|v| f(v) / months.get(i).map_or(30.0, |m| days_observed(m, through))))
        .collect();
    if metric == Metric::DailyPace {
        return speeds;
    }
    speeds
        .iter()
        .enumerate()
        .map(
            |(i, v)| match (v, i.checked_sub(1).and_then(|p| speeds[p])) {
                (Some(v), Some(p)) => Some(v - p),
                _ => None,
            },
        )
        .collect()
}

fn month_label(month: &MonthSpending) -> String {
    let name = format::month_short_year(&month.start);
    if month.partial_month {
        format!("{name} · so far")
    } else {
        name
    }
}

fn direction_tone(direction: &str) -> &'static str {
    match direction {
        "Increasing pace" => "text-warning-fg",
        "Decreasing pace" => "text-positive-fg",
        _ => "text-fg-muted",
    }
}

#[component]
pub fn CategoryTrends(
    trend: CurrencyTrends,
    mappings: Vec<GroupMapping>,
    #[prop(into)] through: String,
) -> impl IntoView {
    let mut sorted = trend.categories.clone();
    sorted.sort_by_key(|row| std::cmp::Reverse(row.total));
    // Each category keeps its colour slot while selected, so toggling one never repaints the others.
    let slots = RwSignal::new(
        (0..COLORS.len())
            .map(|i| sorted.get(i).map(key))
            .collect::<Vec<Option<String>>>(),
    );
    let query = RwSignal::new(String::new());
    let metric = RwSignal::new(Metric::MonthlySpend);
    let trend = StoredValue::new(trend);
    let sorted = StoredValue::new(sorted);
    let mappings = StoredValue::new(mappings);
    let through = StoredValue::new(through);
    let code = trend.with_value(|t| t.currency.code.clone());
    let unit: Currency = trend.with_value(|t| t.currency.clone());
    let unit = StoredValue::new(unit);
    let toggle = move |k: String| {
        slots.update(|s| {
            if let Some(slot) = s.iter_mut().find(|x| x.as_deref() == Some(k.as_str())) {
                *slot = None;
            } else if let Some(free) = s.iter_mut().find(|x| x.is_none()) {
                *free = Some(k);
            }
        });
    };
    let plotted = move || {
        slots
            .get()
            .into_iter()
            .enumerate()
            .filter_map(|(slot, k)| {
                let k = k?;
                trend
                    .with_value(|t| t.categories.iter().find(|c| key(c) == k).cloned())
                    .map(|row| (row, slot))
            })
            .collect::<Vec<_>>()
    };
    let chips = move || {
        let needle = query.get().trim().to_lowercase();
        let selected = slots.get().iter().flatten().count();
        sorted.with_value(|rows| {
            rows.iter()
                .filter(|row| {
                    let group = mappings.with_value(|m| {
                        m.iter()
                            .find(|g| Some(&g.category) == row.category.as_ref())
                            .and_then(|g| g.group.clone())
                            .unwrap_or_default()
                    });
                    format!("{} {group}", name(row)).to_lowercase().contains(&needle)
                })
                .map(|row| {
                    let k = key(row);
                    let slot = slots.get().iter().position(|s| s.as_deref() == Some(k.as_str()));
                    let active = slot.is_some();
                    let class = button_class(if active { Variant::Primary } else { Variant::Secondary }, Size::Sm);
                    let k2 = k.clone();
                    view! {
                        <button
                            type="button"
                            class=class
                            aria-pressed=active.to_string()
                            disabled={!active && selected >= COLORS.len()}
                            on:click=move |_| toggle(k2.clone())
                        >
                            {slot.map(|s| view! { <i aria-hidden="true" class="size-2 rounded-pill" style:background=COLORS[s]></i> })}
                            {name(row)}
                        </button>
                    }
                })
                .collect_view()
        })
    };
    let partial = trend.with_value(|t| t.months.last().is_some_and(|m| m.partial_month));
    view! {
        <Card class="grid gap-4">
            <CardHeader class="mb-0">
                <div>
                    <Eyebrow level=Heading::P>{format!("CATEGORY TRENDS · {code}")}</Eyebrow>
                    <CardTitle level=Heading::H2>"Compare categories"</CardTitle>
                </div>
                <Badge tone=Tone::Outline>{format!("Choose up to {}", COLORS.len())}</Badge>
            </CardHeader>
            {(sorted.with_value(Vec::len) > 8).then(|| view! {
                <input
                    type="search"
                    aria-label="Find a category"
                    placeholder="Find a category"
                    class=crate::ds::CONTROL
                    prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                />
            })}
            <div class="flex flex-wrap gap-2" role="group" aria-label="Categories to plot">{chips}</div>
            {move || {
                let rows = plotted();
                if rows.is_empty() {
                    return view! { <EmptyState title="Choose a category." /> }.into_any();
                }
                let m = metric.get();
                let code = unit.with_value(|u| u.code.clone());
                let (months, categories) = trend.with_value(|t| {
                    (t.months.clone(), t.months.iter().map(month_label).collect::<Vec<_>>())
                });
                let series: Vec<Series> = rows
                    .iter()
                    .map(|(row, slot)| Series {
                        key: format!("slot{slot}"),
                        label: name(row),
                        color: COLORS[*slot].into(),
                        values: plot_values(row, &months, &through.get_value(), m),
                    })
                    .collect();
                let u = unit.get_value();
                let suffix = if m == Metric::MonthlySpend { "" } else { "/day" };
                let format_metric = fmt(move |v| {
                    let sign = if m == Metric::PaceChange && v > 0.0 { "+" } else { "" };
                    format!("{sign}{}{suffix}", money(Decimal::from_f64_retain(v).unwrap_or_default(), &u))
                });
                view! {
                    <div class="flex flex-wrap items-center justify-between gap-3">
                        <SegmentedControl
                            label="Spending measure"
                            small=true
                            value=Signal::derive(move || metric.get().id().to_owned())
                            on_change=Callback::new(move |v: String| {
                                if let Some(x) = Metric::ALL.into_iter().find(|x| x.id() == v) {
                                    metric.set(x);
                                }
                            })
                            items=Metric::ALL.iter().map(|x| (x.id().to_owned(), x.label().to_owned())).collect()
                        />
                        <p class="text-sm text-fg-muted">
                            <strong class="font-semibold text-fg">{m.label()}</strong>
                            {format!(" · {}", m.note(&code))}
                        </p>
                    </div>
                    <div>
                        <LineChart
                            categories=categories
                            series=series
                            format=format_metric
                            y_axis_width=76
                            height=260
                            label=format!("{} of monthly category spending in {code}", m.label())
                        />
                    </div>
                    <div class="grid gap-3 [grid-template-columns:repeat(auto-fit,minmax(190px,1fr))]">
                        {rows
                            .into_iter()
                            .map(|(row, slot)| {
                                let mo = momentum(&row, &through.get_value(), partial);
                                let u = unit.get_value();
                                view! {
                                    <div
                                        class=cn([crate::ds::card_class(CardTone::Muted), "grid gap-2 border-t-2 p-3 sm:p-3"])
                                        style:border-top-color=COLORS[slot]
                                    >
                                        <strong class="truncate">{name(&row)}</strong>
                                        <div class="flex justify-between gap-3 text-xs text-fg-muted">
                                            <span>{if partial { "Monthly spend · projected" } else { "Monthly spend" }}</span>
                                            <b class="font-medium text-fg"><Money value=mo.projected unit=u.clone() neutral=true /></b>
                                        </div>
                                        <div class="flex justify-between gap-3 text-xs text-fg-muted">
                                            <span>"Daily pace"</span>
                                            <b class="font-medium text-fg"><Money value=mo.daily_pace unit=u.clone() neutral=true />"/day"</b>
                                        </div>
                                        <div class="flex justify-between gap-3 text-xs text-fg-muted">
                                            <span>"Pace change"</span>
                                            <b class="font-medium text-fg">
                                                {match mo.pace_change {
                                                    None => "—".to_owned().into_any(),
                                                    Some(p) => view! {
                                                        {if p >= Decimal::ZERO { "+" } else { "" }}
                                                        <Money value=p unit=u.clone() neutral=true />
                                                        "/day"
                                                    }
                                                    .into_any(),
                                                }}
                                            </b>
                                        </div>
                                        <em class=format!("text-xs not-italic {}", direction_tone(mo.direction))>
                                            {format!("{}{}", mo.direction, if partial { " at the current pace" } else { " versus last month" })}
                                        </em>
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                .into_any()
            }}
        </Card>
    }
}
