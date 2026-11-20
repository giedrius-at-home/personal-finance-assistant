//! The daily spending forecast for the next seven days (`daily-forecast.tsx`).

use finance_types::gateway::Direction;
use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{BarChart, Series, fmt};
use crate::ds::{
    Badge, Card, CardTitle, Eyebrow, Facts, Heading, Metric, Money, TABLE, TABLE_CONTAINER,
    TCELL_NUM, TH, TH_NUM, TROW, Tone, cn, money,
};
use crate::format;
use crate::pages::f;

use super::outlook::{CAPTION, ROW_HEADER, chart_frame, percent};
use super::weekday;

#[component]
pub fn DailyForecastPanel(report: Dashboard, currency_code: String) -> impl IntoView {
    let Some(forecast) = report
        .daily_forecasts
        .iter()
        .find(|d| d.currency.code == currency_code)
        .cloned()
    else {
        return view! {
            <Card class="grid gap-2">
                <CardTitle>"Daily forecast"</CardTitle>
                <span title="Prepared in the background. It needs at least four weeks of spending history."><Badge tone=Tone::Outline>"Preparing"</Badge></span>
            </Card>
        }
        .into_any();
    };
    let unit = forecast.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let amount = |v: Decimal, class: &str| {
        let (unit, class) = (unit.clone(), class.to_owned());
        view! { <Money value=v unit=unit class=class /> }.into_any()
    };
    let week: Vec<_> = forecast.days.iter().take(7).cloned().collect();
    let heaviest = forecast
        .habits
        .iter()
        .fold(
            None,
            |best: Option<&finance_types::report::outlook::WeekdayHabit>, h| match best {
                Some(b) if h.average <= b.average => Some(b),
                _ => Some(h),
            },
        )
        .cloned();
    let label = |day: &str| format!("{} {}", weekday(day), format::day_month(day));
    let categories: Vec<String> = week.iter().map(|d| label(&d.day)).collect();
    let series = vec![
        Series::new(
            0,
            "habits",
            "Habits",
            week.iter().map(|d| Some(f(d.habit_expected))).collect(),
        ),
        Series {
            color: "var(--ds-chart-4)".into(),
            ..Series::new(
                1,
                "planned",
                "Planned payments and bills",
                week.iter().map(|d| Some(f(d.planned))).collect(),
            )
        },
    ];
    let code = unit.code.clone();
    let chart = view! {
        <BarChart
            categories=categories
            series=series
            stacked=true
            height=260
            label="Forecast per day"
            format=fmt(move |n| format::money_dp(Decimal::try_from(n).unwrap_or_default(), &code, 0))
        />
    }
    .into_any();
    let list = week
        .iter()
        .map(|d| {
            let aria = format!(
                "{} {}: habits {}, planned {}",
                weekday(&d.day),
                format::day_month(&d.day),
                m(d.habit_expected),
                m(d.planned)
            );
            view! { <li aria-label=aria>{format!("{}: {}", label(&d.day), m(d.habit_expected + d.planned))}</li> }
        })
        .collect_view();
    // A bill with nothing to pay (an unused card) is not worth a line.
    let coming: Vec<_> = forecast
        .upcoming
        .iter()
        .filter(|flow| !flow.amount.is_zero())
        .cloned()
        .collect();
    let upcoming = (!coming.is_empty()).then(|| {
        view! {
            <ul class="grid content-start" aria-label="Coming up">
                <li class="pb-1 text-eyebrow font-bold uppercase tracking-eyebrow text-fg-muted">"Coming up"</li>
                {coming
                    .iter()
                    .map(|flow| {
                        view! {
                            <li class="flex justify-between gap-3 border-b border-line py-2 text-sm last:border-b-0">
                                <span>{format!("{} · {}", format::day_month(&flow.day), flow.name)}</span>
                                <strong class="font-semibold">
                                    {if flow.direction == Direction::Inflow { "+" } else { "" }}
                                    {amount(flow.amount, "")}
                                </strong>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        }
    });
    let habits = forecast
        .habits
        .iter()
        .map(|h| {
            view! {
                <tr class=TROW>
                    <th scope="row" class=cn([TH, ROW_HEADER])>{h.weekday.clone()}</th>
                    <td class=TCELL_NUM>{m(h.average)}</td>
                    <td class=TCELL_NUM>{format!("{}%", percent(h.spend_day_share))}</td>
                </tr>
            }
        })
        .collect_view();
    let mut evidence = vec![
        (
            "Habit spending this month".to_owned(),
            m(forecast.habit_spent_this_month),
        ),
        (
            "Bills and planned payments paid".to_owned(),
            m(forecast.planned_paid_this_month),
        ),
    ];
    if let Some(h) = &heaviest {
        evidence.insert(
            0,
            (
                "Heaviest habit day".into(),
                format!("{} · {}", h.weekday, m(h.average)),
            ),
        );
    }
    let next_week = amount(forecast.next_week_habit + forecast.next_week_planned, "");
    let next_week_facts = vec![
        ("Habits".to_owned(), m(forecast.next_week_habit)),
        (
            "Likely range".to_owned(),
            format!(
                "{} – {}",
                m(forecast.next_week_lower),
                m(forecast.next_week_upper)
            ),
        ),
        ("Planned".to_owned(), m(forecast.next_week_planned)),
    ];
    let month_end = amount(forecast.month_end_habit, "");
    let month_end_facts = vec![
        (
            "Likely range".to_owned(),
            format!(
                "{} – {}",
                m(forecast.month_end_lower),
                m(forecast.month_end_upper)
            ),
        ),
        (
            "Still planned".to_owned(),
            m(forecast.month_remaining_planned),
        ),
        (
            "Income expected".to_owned(),
            m(forecast.month_remaining_income),
        ),
    ];
    let frame = chart_frame("Forecast per day", String::new(), 300, chart);
    let content = view! {
        <div role="region" aria-label="Daily spending forecast" class="contents">
            <header>
                <Eyebrow level=Heading::P>"THE NEXT SEVEN DAYS"</Eyebrow>
                <CardTitle>"Daily spending forecast"</CardTitle>
            </header>
            <div class="grid gap-x-10 gap-y-6 lg:grid-cols-[minmax(240px,.4fr)_minmax(0,1.1fr)_minmax(260px,.5fr)]">
                <div class="grid content-start gap-6">
                    <Metric label="Next 7 days" facts=next_week_facts>{next_week}</Metric>
                    <Metric label="Everyday spending by month end" facts=month_end_facts>{month_end}</Metric>
                </div>
                {frame}
                {upcoming}
            </div>
            <ol class="sr-only" aria-label="Forecast per day, as a list">{list}</ol>
            <details class="grid gap-3 text-sm leading-relaxed [&>*]:mt-3">
                <summary class="cursor-pointer font-medium">"Your usual week"</summary>
                <div tabindex="0" role="region" aria-label="Last twelve weeks" class=TABLE_CONTAINER>
                    <table class=TABLE data-density="md">
                        <caption class=CAPTION>"Last twelve weeks"</caption>
                        <thead class="text-left">
                            <tr class=cn([TROW, "border-t-0"])>
                                <th class=cn([TH, "text-left"])>"Day"</th>
                                <th class=TH_NUM>"Average spend"</th>
                                <th class=TH_NUM>"Days with a purchase"</th>
                            </tr>
                        </thead>
                        <tbody>{habits}</tbody>
                    </table>
                </div>
                <Facts rows=evidence class="max-w-md" />
            </details>
        </div>
    }
    .into_any();
    view! { <Card class="grid gap-5">{content}</Card> }.into_any()
}
