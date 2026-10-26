//! The daily spending forecast for the next seven days (`daily-forecast.tsx`).

use finance_types::gateway::Direction;
use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{BarChart, Series, fmt};
use crate::ds::{
    Badge, Card, CardTitle, Eyebrow, Heading, Money, Stat, TABLE, TABLE_CONTAINER, TCELL_NUM, TH,
    TH_NUM, TROW, Tone, cn, money,
};
use crate::format;
use crate::pages::f;

use super::outlook::{CAPTION, ROW_HEADER, chart_frame, percent};
use super::weekday;

fn method_label(method: &str) -> String {
    match method {
        "timesfm" => "TimesFM".into(),
        "timesfm_planned" => "TimesFM with your planned payments".into(),
        "weekday_baseline" => "Same-weekday average".into(),
        other => other.into(),
    }
}

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
                <p class="text-sm text-fg-muted">"Forecast preparing in the background. It needs at least four weeks of spending history."</p>
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
            height=186
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
    let upcoming = (!forecast.upcoming.is_empty()).then(|| {
        view! {
            <ul class="grid" aria-label="Coming up">
                {forecast
                    .upcoming
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
    let errors = forecast.baseline_error.map(|baseline| {
        let model = forecast
            .model_error
            .map(|e| format!("; TimesFM {}", m(e)))
            .unwrap_or_default();
        let planned = forecast
            .planned_model_error
            .map(|e| format!("; TimesFM with planned payments {}", m(e)))
            .unwrap_or_default();
        view! { <p>{format!("Average weekly miss in past weeks: weekday average {}{model}{planned}.", m(baseline))}</p> }
    });
    let badge = format!(
        "{}{}",
        method_label(&forecast.method),
        if forecast.backtest_windows > 0 {
            format!(
                " · checked against {} past weeks",
                forecast.backtest_windows
            )
        } else {
            String::new()
        }
    );
    let next_week = amount(
        forecast.next_week_habit + forecast.next_week_planned,
        "text-2xl",
    );
    let next_week_delta = format!(
        "habits {} (likely {}–{}) · planned {}",
        m(forecast.next_week_habit),
        m(forecast.next_week_lower),
        m(forecast.next_week_upper),
        m(forecast.next_week_planned),
    );
    let month_end = amount(forecast.month_end_habit, "text-2xl");
    let month_end_delta = format!(
        "likely {}–{} · {} still planned · {} income expected",
        m(forecast.month_end_lower),
        m(forecast.month_end_upper),
        m(forecast.month_remaining_planned),
        m(forecast.month_remaining_income),
    );
    let frame = chart_frame(
        "Forecast per day",
        "Habits and planned payments and bills for each of the next seven days".into(),
        220,
        chart,
    );
    let heaviest = heaviest.map(|h| {
        view! { <p>{format!("Your heaviest habit day is {}, about {}.", h.weekday, m(h.average))}</p> }
    });
    let history = format!(
        "Habit spending this month so far: {}. Bills and planned payments paid: {}. Model: {}, from {} days of history.",
        m(forecast.habit_spent_this_month),
        m(forecast.planned_paid_this_month),
        forecast.model,
        forecast.training_days,
    );
    let content = view! {
        <div role="region" aria-label="Daily spending forecast" class="contents">
            <header class="flex flex-col items-start justify-between gap-3 sm:flex-row">
                <div>
                    <Eyebrow level=Heading::P>"THE NEXT SEVEN DAYS"</Eyebrow>
                    <CardTitle>"Daily spending forecast"</CardTitle>
                </div>
                <Badge tone=Tone::Outline class="h-auto whitespace-normal text-left">{badge}</Badge>
            </header>
            <div class="grid gap-4 sm:grid-cols-2">
                <Stat label="Next 7 days" delta=next_week_delta>{next_week}</Stat>
                <Stat label="Habit spending by month end" delta=month_end_delta>{month_end}</Stat>
            </div>
            {frame}
            <ol class="sr-only" aria-label="Forecast per day, as a list">{list}</ol>
            {upcoming}
            <details class="grid gap-3 text-sm leading-relaxed [&>*]:mt-3">
                <summary class="cursor-pointer font-medium">"Your weekday habits & forecast evidence"</summary>
                {heaviest}
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
                <p>{history}</p>
                {errors}
            </details>
        </div>
    }
    .into_any();
    view! { <Card class="grid gap-5">{content}</Card> }.into_any()
}
