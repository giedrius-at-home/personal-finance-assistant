//! The Spending section: habits & goals, organising categories, and the
//! full history (personal-site `integrated-finance.tsx`, Spending tab).

use finance_types::report::trends::CurrencyTrends;
use leptos::prelude::*;

use crate::ds::charts::ProgressBar;
use crate::ds::patterns::{
    Disclosure, EMPHASIS, HINT, LEGEND_ITEM, PAGE_SECTION, REPEAT_STACK, REVIEW_GRID, TITLE_BLOCK,
};
use crate::ds::{
    Badge, Card, Eyebrow, Facts, Heading, Help, Icon, Lucide, Money, Size, Switch, Tone, Variant,
    button_class, money,
};
use crate::layout::{Initial, PageView, PanelDef};
use crate::pages::category_trends::CategoryTrends;
use crate::pages::overview::outlook;
use crate::pages::{f, report_is, with_report};
use crate::{Tab, state};

/// Habits & goals and History panels, in their first-run order.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "spending.habits.outlook",
        title: "Three-month outlook",
        page: HABITS,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: outlook::panel,
        summary: Some(outlook::summary),
        shown: Some(|| report_is(|r| !r.savings_outlooks.is_empty())),
    },
    PanelDef {
        id: "spending.habits.goals",
        title: "Goals",
        page: HABITS,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || crate::pages::goals::GoalsCard().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "spending.habits.adjustments",
        title: "Adjustments",
        page: HABITS,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || crate::pages::adjustments::AdjustmentsCard().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "spending.habits.weekday",
        title: "Spending by weekday",
        page: HABITS,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || Weekday().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "spending.history.monthly",
        title: "Monthly spending",
        page: HISTORY,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: monthly,
        summary: None,
        shown: Some(|| report_is(|r| !r.monthly_spending.currencies.is_empty())),
    },
    PanelDef {
        id: "spending.history.compare",
        title: "Compare categories",
        page: HISTORY,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: compare,
        summary: None,
        shown: Some(|| report_is(|r| !r.time_trends.currencies.is_empty())),
    },
];

const HABITS: &str = "spending.habits";
const HISTORY: &str = "spending.history";

#[component]
pub fn PlanPatterns() -> impl IntoView {
    let app = state();
    view! {
        <button
            type="button"
            class=format!("{} -ml-4", button_class(Variant::Ghost, Size::Md))
            on:click=move |_| app.go(Tab::Transactions)
        >
            "Review shared spending & money back →"
        </button>
        <PageView tab="spending" tab_name="Spending" page=HABITS />
    }
}

/// The weekday panel, with its "Variable patterns" switch.
#[component]
fn Weekday() -> impl IntoView {
    let variable = RwSignal::new(true);
    with_report(move |report| {
        let has_variable = !report.variable_time_trends.currencies.is_empty();
        let trends = if variable.get() && has_variable {
            report.variable_time_trends.currencies.clone()
        } else {
            report.time_trends.currencies.clone()
        };
        let variable_only = variable.get() && has_variable;
        view! {
            <div class=REPEAT_STACK>
                <div class="glass-card flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
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
            </div>
        }
    })
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
        <section class=PAGE_SECTION aria-label=label>
            <Card class="p-0 sm:p-0">
                <Disclosure
                    summary_class="flex items-center justify-between gap-4 px-4 py-3.5 [&::-webkit-details-marker]:hidden"
                    summary=view! {
                        <div class=TITLE_BLOCK>
                            <Eyebrow>"SECONDARY PATTERN"</Eyebrow>
                            <strong class="font-semibold">"Spending by weekday"</strong>
                        </div>
                        <span class=HINT>"View details"</span>
                    }
                >
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
                    <div class="flex items-center gap-2 px-4 pb-4">
                        <Badge tone=Tone::Outline>{if variable_only { "Variable only" } else { "All spending" }}</Badge>
                        <Help text="Average purchases per active spending day across six months." />
                    </div>
                </Disclosure>
            </Card>
        </section>
    }
}

#[component]
pub fn Organise() -> impl IntoView {
    crate::pages::organise::Organise()
}

#[component]
pub fn History() -> impl IntoView {
    view! { <PageView tab="spending" tab_name="Spending" page=HISTORY /> }
}

/// "All spending history": this month by group and store. The months side
/// by side are the Overview's trends.
fn monthly() -> AnyView {
    with_report(|report| {
        let ms = report.monthly_spending.clone();
        let period = format!("{} — {}", ms.start, ms.through);
        let cards = ms
            .currencies
            .into_iter()
            .map(|row| {
                let unit = row.currency.clone();
                let period = period.clone();
                let eyebrow = format!("Monthly spending · {}", unit.code);
                let facts = vec![
                    ("Period, partial month".to_owned(), period),
                    ("Refunds".to_owned(), money(row.refunds, &unit)),
                    (
                        "Previous comparison".to_owned(),
                        row.previous_spending.map_or("—".to_owned(), |p| money(p, &unit)),
                    ),
                ];
                view! {
                    <Card>
                        <Eyebrow level=Heading::H2>{eyebrow}</Eyebrow>
                        <Money value=row.spending unit=unit.clone() neutral=true class="mb-2 mt-3 block text-2xl font-semibold tracking-tight" />
                        <Facts rows=facts />
                        <div class="mt-4">
                            {row
                                .groups
                                .into_iter()
                                .map(|group| {
                                    let unit = unit.clone();
                                    view! {
                                        <Disclosure
                                            class="group border-t border-line py-3.5"
                                            summary_class="flex list-none items-center justify-between gap-3 text-sm [&::-webkit-details-marker]:hidden"
                                            summary=view! {
                                                <span class=LEGEND_ITEM>
                                                    <Lucide icon=Icon::ChevronDown size=14 class="-rotate-90 text-fg-muted transition-transform group-open:rotate-0" />
                                                    {group.group.unwrap_or_else(|| "Needs a group".into())}
                                                </span>
                                                <Money value=group.spending unit=unit.clone() neutral=true />
                                            }
                                        >
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
                                                            <Money value=store.spending unit=unit.clone() neutral=true class=EMPHASIS />
                                                        </p>
                                                    }
                                                })
                                                .collect_view()}
                                        </Disclosure>
                                    }
                                })
                                .collect_view()}
                        </div>
                    </Card>
                }
            })
            .collect_view();
        view! { <div class=REVIEW_GRID>{cards}</div> }
    })
    .into_any()
}

/// The category comparison, one per currency.
fn compare() -> AnyView {
    with_report(|report| {
        let ms = &report.monthly_spending;
        let compare = report
            .time_trends
            .currencies
            .iter()
            .map(|trend| {
                view! { <CategoryTrends trend=trend.clone() mappings=ms.mappings.clone() through=ms.through.clone() /> }
            })
            .collect_view();
        view! { <div class=REPEAT_STACK>{compare}</div> }
    })
    .into_any()
}
