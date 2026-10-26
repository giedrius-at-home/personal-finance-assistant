//! The trends card of `overview-money.tsx` (the owner's Overview shows it
//! with `showMoney={false}`, so the money-now and money-later cards are out).

use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::ds::charts::{BarChart, Series, fmt};
use crate::ds::{
    Card, CardTitle, Eyebrow, Heading, Icon, Lucide, Size, Stat, Variant, button_class, cn, money,
};
use crate::format;
use crate::pages::{f, with_report};
use crate::{Tab, state};

#[component]
pub fn OverviewTrends() -> impl IntoView {
    let app = state();
    with_report(move |report| {
        let articles = report
            .time_trends
            .currencies
            .iter()
            .map(|trend| {
                let unit = trend.currency.clone();
                let months: Vec<_> = trend
                    .months
                    .iter()
                    .skip(trend.months.len().saturating_sub(6))
                    .cloned()
                    .collect();
                let monthly = report
                    .monthly_spending
                    .currencies
                    .iter()
                    .find(|row| row.currency.code == unit.code);
                let change = monthly.and_then(|row| {
                    row.previous_spending
                        .filter(|p| *p > Decimal::ZERO)
                        .map(|p| (row.spending - p) / p * Decimal::ONE_HUNDRED)
                });
                let week = report
                    .weekly
                    .currencies
                    .iter()
                    .find(|row| row.currency.code == unit.code);
                let this_month = monthly.map_or("—".to_owned(), |row| money(row.spending, &unit));
                let versus = change.map_or("No comparison".to_owned(), |c| {
                    let rounded = c.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
                    format!("{}{rounded}%", if c >= Decimal::ZERO { "+" } else { "" })
                });
                let positive = week.is_some_and(|w| w.net_cash_flow >= Decimal::ZERO);
                let this_week = week.map_or("—".to_owned(), |w| money(w.net_cash_flow, &unit));
                let categories: Vec<String> = months
                    .iter()
                    .map(|m| {
                        format!(
                            "{}{}",
                            format::month_short(&m.start[..7.min(m.start.len())]),
                            if m.partial_month { "*" } else { "" }
                        )
                    })
                    .collect();
                let values = months.iter().map(|m| m.spending.map(f)).collect();
                let code = unit.code.clone();
                view! {
                    <article class="min-w-0">
                        <div class="grid grid-cols-1 gap-3 border-b border-line pb-3.5 sm:grid-cols-3">
                            <Stat label="This month">{this_month}</Stat>
                            <Stat label="vs previous">{versus}</Stat>
                            <Stat label="This week">
                                <span class=if positive { "text-positive-fg" } else { "" }>{this_week}</span>
                            </Stat>
                        </div>
                        <BarChart
                            class="mt-3.5"
                            height=150
                            label=format!("{} spending by month", unit.code)
                            categories=categories
                            series=vec![Series::new(0, "spending", "Spending", values)]
                            format=fmt(move |n| format::money_dp(Decimal::try_from(n).unwrap_or_default(), &code, 0))
                        />
                    </article>
                }
            })
            .collect_view();
        view! {
            <section class="mb-3 mt-3.5 grid gap-3 text-fg" aria-label="Current money and outlook">
                <Card class="min-w-0">
                    <header class="flex items-center justify-between gap-4">
                        <div class="min-w-0">
                            <Eyebrow level=Heading::P>"TRENDS"</Eyebrow>
                            <CardTitle level=Heading::H2 class="mt-1 text-base">"Last six months"</CardTitle>
                        </div>
                        <button
                            type="button"
                            class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), "text-accent"])
                            on:click=move |_| app.go(Tab::Spending)
                        >
                            "Open spending "
                            <Lucide icon=Icon::ArrowUpRight size=13 />
                        </button>
                    </header>
                    <div class="mt-4 grid grid-cols-[repeat(auto-fit,minmax(min(100%,280px),1fr))] gap-6">
                        {articles}
                    </div>
                </Card>
            </section>
        }
    })
}
