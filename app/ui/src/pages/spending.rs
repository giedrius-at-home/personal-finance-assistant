//! The Spending section: plan & patterns, organising categories, and the
//! full history (personal-site `integrated-finance.tsx`, Spending tab).

use finance_types::report::Currency;
use finance_types::report::trends::CurrencyTrends;
use leptos::prelude::*;

use crate::ds::charts::ProgressBar;
use crate::ds::{
    Badge, Card, CardHeader, CardTitle, Eyebrow, Heading, Icon, Lucide, Money, Size, Switch, Tone,
    Variant, button_class, money,
};
use crate::pages::category_trends::CategoryTrends;
use crate::pages::{f, with_report};
use crate::{Tab, state};

#[component]
pub fn PlanPatterns() -> impl IntoView {
    let app = state();
    let variable = RwSignal::new(true);
    view! {
        {crate::pages::overview::budget_panel()}
        <button
            type="button"
            class=format!("{} -ml-4 mt-2", button_class(Variant::Ghost, Size::Md))
            on:click=move |_| app.go(Tab::Transactions)
        >
            "Review shared spending & money back →"
        </button>
        <crate::pages::goals::GoalsCard />
        <crate::pages::adjustments::AdjustmentsCard />
        {with_report(move |report| {
            let has_variable = !report.variable_time_trends.currencies.is_empty();
            let trends = if variable.get() && has_variable {
                report.variable_time_trends.currencies.clone()
            } else {
                report.time_trends.currencies.clone()
            };
            let variable_only = variable.get() && has_variable;
            view! {
                <div class="glass-card my-4 flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
                    <Switch
                        id="variable-patterns"
                        checked=Signal::derive(move || variable.get())
                        on_change=Callback::new(move |v| variable.set(v))
                        disabled=Signal::derive(move || !has_variable)
                    />
                    <label for="variable-patterns" class="cursor-pointer text-sm text-fg">
                        "Variable patterns"
                    </label>
                </div>
                {trends
                    .into_iter()
                    .map(|trend| view! { <SpendingAnalysis trend=trend variable_only=variable_only /> })
                    .collect_view()}
            }
        })}
    }
}

/// Spending by weekday (`spending-analysis.tsx`).
#[component]
fn SpendingAnalysis(trend: CurrencyTrends, variable_only: bool) -> impl IntoView {
    let peak = trend
        .weekdays
        .iter()
        .map(|w| f(w.average_purchase_day.unwrap_or_default()))
        .fold(1.0_f64, f64::max);
    let unit = trend.currency.clone();
    let label = format!("{} spending patterns", unit.code);
    view! {
        <section class="mb-6 w-full min-w-0" aria-label=label>
            <Card class="p-0 sm:p-0">
                <details>
                    <summary class="flex cursor-pointer items-center justify-between gap-4 px-4 py-3.5 [&::-webkit-details-marker]:hidden">
                        <div class="grid gap-1">
                            <Eyebrow>"SECONDARY PATTERN"</Eyebrow>
                            <strong class="font-semibold">"Spending by weekday"</strong>
                        </div>
                        <span class="text-xs text-fg-muted">"View details"</span>
                    </summary>
                    <div class="grid gap-3 border-t border-border p-4">
                        {trend
                            .weekdays
                            .iter()
                            .map(|row| {
                                let value = row.average_purchase_day.map_or("—".to_owned(), |v| money(v, &unit));
                                view! {
                                    <ProgressBar
                                        label=format!("{} · {value}", &row.label[..row.label.len().min(3)])
                                        value=f(row.average_purchase_day.unwrap_or_default())
                                        max=peak
                                    />
                                }
                            })
                            .collect_view()}
                    </div>
                    <p class="px-4 pb-4 text-xs text-fg-muted">
                        "Average purchases per active spending day across six months. "
                        {if variable_only { "The variable filter is applied." } else { "All spending is included." }}
                    </p>
                </details>
            </Card>
        </section>
    }
}

#[component]
pub fn Organise() -> impl IntoView {
    crate::pages::organise::Organise()
}

/// "All spending history": this month by group and store, every month, and
/// the category comparison.
#[component]
pub fn History() -> impl IntoView {
    with_report(|report| {
        let ms = report.monthly_spending.clone();
        let period = format!("{} — {} · Current month is partial", ms.start, ms.through);
        let cards = ms
            .currencies
            .clone()
            .into_iter()
            .map(|row| {
                let unit = row.currency.clone();
                let period = period.clone();
                let eyebrow = format!("Monthly spending · {}", unit.code);
                let refunds = format!(
                    "Refunds: {} · Previous comparison: {}",
                    money(row.refunds, &unit),
                    row.previous_spending.map_or("—".to_owned(), |p| money(p, &unit)),
                );
                view! {
                    <Card>
                        <Eyebrow level=Heading::H2>{eyebrow}</Eyebrow>
                        <Money value=row.spending unit=unit.clone() neutral=true class="mb-2 mt-3 block text-2xl font-semibold tracking-tight" />
                        <p class="text-xs text-fg-muted">{period}</p>
                        <p class="mt-1 text-xs text-fg-muted">{refunds}</p>
                        <div class="mt-4">
                            {row
                                .groups
                                .into_iter()
                                .map(|group| {
                                    let unit = unit.clone();
                                    view! {
                                        <details class="group border-t border-line py-3.5">
                                            <summary class="flex cursor-pointer list-none items-center justify-between gap-3 text-sm [&::-webkit-details-marker]:hidden">
                                                <span class="flex items-center gap-1.5">
                                                    <Lucide icon=Icon::ChevronDown size=14 class="-rotate-90 text-fg-muted transition-transform group-open:rotate-0" />
                                                    {group.group.unwrap_or_else(|| "Needs a group".into())}
                                                </span>
                                                <Money value=group.spending unit=unit.clone() neutral=true />
                                            </summary>
                                            {group
                                                .stores
                                                .into_iter()
                                                .map(|store| {
                                                    let text = format!(
                                                        "{} · {} records",
                                                        store.category.unwrap_or_else(|| "Uncategorised".into()),
                                                        store.transaction_count,
                                                    );
                                                    view! {
                                                        <p class="mt-3 flex justify-between gap-3 text-xs text-fg-muted">
                                                            <span>{text}</span>
                                                            <Money value=store.spending unit=unit.clone() neutral=true class="font-semibold text-fg" />
                                                        </p>
                                                    }
                                                })
                                                .collect_view()}
                                        </details>
                                    }
                                })
                                .collect_view()}
                        </div>
                    </Card>
                }
            })
            .collect_view();
        let through = ms.through.clone();
        let over_time = report
            .time_trends
            .currencies
            .iter()
            .map(|trend| spending_over_time(trend.clone()))
            .collect_view();
        let compare = report
            .time_trends
            .currencies
            .iter()
            .map(|trend| {
                view! { <CategoryTrends trend=trend.clone() mappings=ms.mappings.clone() through=through.clone() /> }
            })
            .collect_view();
        view! {
            <div class="grid gap-3 md:grid-cols-2">{cards}</div>
            {over_time}
            {compare}
        }
    })
}

fn spending_over_time(trend: CurrencyTrends) -> impl IntoView {
    let unit: Currency = trend.currency.clone();
    let code = unit.code.clone();
    view! {
        <Card class="my-4">
            <CardHeader class="mb-5">
                <CardTitle level=Heading::H2>"Spending over time"</CardTitle>
                <Badge tone=Tone::Outline>{code}</Badge>
            </CardHeader>
            <dl class="grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-3 lg:grid-cols-6">
                {trend
                    .months
                    .into_iter()
                    .map(|month| {
                        let detail = match month.spending {
                            None => "—".to_owned(),
                            Some(_) if month.partial_month => "Partial month".into(),
                            Some(_) => format!("{} records", month.record_count),
                        };
                        view! {
                            <div class="grid gap-2 border-l border-line pl-3">
                                <dt class="text-xs text-fg-muted">{month.start[..7].to_owned()}</dt>
                                <dd class="whitespace-nowrap text-base font-semibold">
                                    {match month.spending {
                                        None => "—".to_owned().into_any(),
                                        Some(s) => view! { <Money value=s unit=unit.clone() neutral=true /> }.into_any(),
                                    }}
                                </dd>
                                <dd class="text-xs text-fg-muted">{detail}</dd>
                            </div>
                        }
                    })
                    .collect_view()}
            </dl>
        </Card>
    }
}
