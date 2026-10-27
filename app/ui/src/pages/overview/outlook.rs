//! The three-month savings outlook (`savings-outlook.tsx`).

use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::ds::charts::{LineChart, Series, fmt, series_color};
use crate::ds::{
    Card, CardDescription, CardHeader, CardTitle, Eyebrow, Heading, Money, Stat, TABLE,
    TABLE_CONTAINER, TCELL_NUM, TH, TH_NUM, TROW, Tone, cn, money,
};
use crate::format;
use crate::pages::f;
use crate::{ManageSection, state};

/// Days since 0000-03-01 (proleptic Gregorian), for date differences.
pub fn day_number(date: &str) -> i64 {
    let part = |range: std::ops::Range<usize>| {
        date.get(range)
            .and_then(|p| p.parse::<i64>().ok())
            .unwrap_or(1)
    };
    let (y, m, d) = (part(0..4), part(5..7), part(8..10));
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    365 * y + y / 4 - y / 100 + y / 400 + (153 * m + 2) / 5 + d - 1
}

/// `Math.round(value * 100)` for a share between 0 and 1.
pub fn percent(value: Decimal) -> Decimal {
    (value * Decimal::ONE_HUNDRED).round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

/// The site's `ChartFrame` (title, description, fixed-height body) without
/// a card around it. The chart inside, legend included, fits `height`.
pub fn chart_frame(
    title: &'static str,
    description: String,
    height: u32,
    chart: AnyView,
) -> impl IntoView {
    view! {
        <div class="min-w-0 text-fg">
            <CardHeader>
                <div class="min-w-0">
                    <CardTitle>{title}</CardTitle>
                    <CardDescription>{description}</CardDescription>
                </div>
            </CardHeader>
            <div class="min-w-0" style:height=format!("{height}px")>{chart}</div>
        </div>
    }
}

pub const ROW_HEADER: &str =
    "py-2.5 text-left text-sm font-medium normal-case tracking-normal text-fg";
pub const CAPTION: &str = "caption-top pb-2 pt-0 text-left text-sm font-semibold text-fg";

#[component]
pub fn SavingsOutlookPanel(report: Dashboard, currency_code: String) -> impl IntoView {
    let app = state();
    let Some(outlook) = report
        .savings_outlooks
        .iter()
        .find(|o| o.currency.code == currency_code)
        .cloned()
    else {
        return view! {
            <Card class="grid gap-2">
                <CardTitle>"Three-month outlook"</CardTitle>
                <p class="text-sm text-fg-muted">"Forecast preparing in the background. Refresh shortly to see your savings outlook."</p>
            </Card>
        }
        .into_any();
    };
    let unit = outlook.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let salary = report
        .planned_payments
        .iter()
        .find(|p| {
            p.plan_id.starts_with("recurring-salary-") && p.amount.currency.code == currency_code
        })
        .cloned();

    let start = (
        outlook.as_of.clone(),
        outlook.starting_balance,
        outlook.starting_balance,
        outlook.starting_balance,
        outlook.starting_balance,
    );
    let points: Vec<_> = std::iter::once(start)
        .chain(outlook.months.iter().map(|p| {
            (
                p.through.clone(),
                p.planned_balance,
                p.expected_balance,
                p.lower_balance,
                p.upper_balance,
            )
        }))
        .enumerate()
        // A month end within ten days of today adds nothing to the line.
        .filter(|(i, p)| !(*i == 1 && day_number(&p.0) - day_number(&outlook.as_of) < 10))
        .map(|(_, p)| p)
        .collect();
    let categories: Vec<String> = points.iter().map(|p| format::day_month(&p.0)).collect();
    let column = |pick: fn(&(String, Decimal, Decimal, Decimal, Decimal)) -> Decimal| {
        points.iter().map(|p| Some(f(pick(p)))).collect::<Vec<_>>()
    };
    let series = vec![
        Series::new(0, "planned", "Following plan", column(|p| p.1)),
        Series::new(1, "expected", "Current habits", column(|p| p.2)),
        Series {
            color: series_color(2, Some("limit")),
            ..Series::new(2, "lower", "Estimated low", column(|p| p.3))
        },
        Series {
            color: series_color(3, Some("limit")),
            ..Series::new(3, "upper", "Estimated high", column(|p| p.4))
        },
    ];

    let status = if !outlook.data_fresh {
        "Waiting for fresh bank data".to_owned()
    } else {
        match outlook.on_track_probability {
            None if outlook.expected_savings < outlook.planned_savings => {
                "Current habits put your plan at risk · limited probability evidence".into()
            }
            None => "Limited forecast evidence".into(),
            Some(p) => {
                let text = if p < Decimal::new(1, 2) {
                    "<1".to_owned()
                } else if p > Decimal::new(99, 2) {
                    ">99".to_owned()
                } else {
                    percent(p).to_string()
                };
                format!("{text}% modeled chance of staying on plan")
            }
        }
    };
    let salary_line = salary.map(|s| {
        let day: u32 = s
            .due_on
            .get(8..10)
            .and_then(|d| d.parse().ok())
            .unwrap_or(1);
        let text = format!("Monthly salary: {} around day {day}. ", m(s.amount.amount));
        view! {
            <CardDescription class="mt-0">
                {text}
                <a
                    class="underline underline-offset-2"
                    href="?view=settings"
                    on:click=move |e| {
                        e.prevent_default();
                        app.go_manage(ManageSection::Settings);
                    }
                >
                    "Edit salary"
                </a>
            </CardDescription>
        }
    });
    let summary = format!(
        "Following plan {}, current habits {}. Estimated range {} to {}.",
        m(outlook.planned_final),
        m(outlook.expected_final),
        m(outlook.lower_final),
        m(outlook.upper_final),
    );
    let aria = format!(
        "Balance projection: following plan {}, current habits {}. Estimated range {} to {}.",
        m(outlook.planned_final),
        m(outlook.expected_final),
        m(outlook.lower_final),
        m(outlook.upper_final),
    );
    let code = unit.code.clone();
    let chart = view! {
        <LineChart
            height=206
            categories=categories
            series=series
            label=aria
            format=fmt(move |n| format::money_dp(Decimal::try_from(n).unwrap_or_default(), &code, 0))
        />
    }
    .into_any();
    let amount = |v: Decimal, class: &str| {
        let (unit, class) = (unit.clone(), class.to_owned());
        view! { <Money value=v unit=unit class=class /> }.into_any()
    };
    let deductions = (!outlook.deductions.is_empty()).then(|| {
        view! {
            <ul class="list-disc pl-5">
                {outlook
                    .deductions
                    .iter()
                    .map(|d| view! { <li>{format!("−{} points: {}", d.points, d.reason)}</li> })
                    .collect_view()}
            </ul>
        }
    });
    let history = outlook
        .unplanned_history
        .iter()
        .map(|row| {
            let month = format!(
                "{}{}",
                format::month_short(&row.month[..7.min(row.month.len())]),
                if row.reconstructed { "*" } else { "" }
            );
            view! {
                <tr class=TROW>
                    <th scope="row" class=cn([TH, ROW_HEADER])>{month}</th>
                    <td class=cn([TCELL_NUM, "whitespace-nowrap"])>{m(row.net_spending)}</td>
                    <td class=cn([TCELL_NUM, "whitespace-nowrap"])>{m(row.same_elapsed_spending)}</td>
                    <td class=TCELL_NUM>{format!("{}%", percent(row.share))}</td>
                    <td class=TCELL_NUM>{row.purchase_count}</td>
                </tr>
            }
        })
        .collect_view();
    let method = match outlook.method.as_str() {
        "timesfm" => "TimesFM",
        "insufficient_history" => "Insufficient history",
        _ => "Weighted weekday baseline",
    };
    let errors = outlook.baseline_error.map(|baseline| {
        view! {
            <p>{format!(
                "Held-out 28-day error: baseline {}; TimesFM {}. {} three-month validation windows.",
                m(baseline),
                outlook.model_error.map_or("unavailable".to_owned(), &m),
                outlook.horizon_validation_windows,
            )}</p>
        }
    });
    let as_of_day: u32 = outlook
        .as_of
        .get(8..10)
        .and_then(|d| d.parse().ok())
        .unwrap_or(1);
    let title = format!("Your money by {}", format::day_month(&outlook.through));
    let planned = amount(outlook.planned_savings, "text-2xl");
    let planned_delta = format!("new savings · final balance {}", m(outlook.planned_final));
    let expected = amount(outlook.expected_savings, "text-2xl");
    let expected_delta = format!("new savings · final balance {}", m(outlook.expected_final));
    let frame = chart_frame("Balance projection", summary, 240, chart);
    let recent = amount(outlook.signal.spending, "text-xl");
    let recent_delta = format!(
        "previous three: {} · unplanned: {}",
        m(outlook.signal.previous_spending),
        m(outlook.signal.unplanned),
    );
    let alert = outlook.signal.alert;
    let score = outlook
        .behavior_score
        .map_or("Paused".to_owned(), |s| format!("{s}/100"));
    let unplanned = amount(outlook.expected_unplanned, "text-xl");
    let evidence = format!(
        "Refunds received this month: {}. Expected income over the forecast: {}. Starting included cash: {}.",
        m(outlook.received_refunds),
        m(outlook.expected_income),
        m(outlook.starting_balance),
    );
    let model = format!(
        "* Reconstructed using the current plan. Selected model: {method}. {} days of evidence; trained in {}s.",
        outlook.training_days, outlook.training_seconds,
    );
    let assumptions = outlook
        .assumptions
        .iter()
        .map(|a| view! { <li>{a.clone()}</li> })
        .collect_view();
    let content = view! {
        <div role="region" aria-label="Three-month savings forecast" class="contents">
            <header class="flex flex-col items-start justify-between gap-3 sm:flex-row">
                <div>
                    <Eyebrow level=Heading::P>"LOOKING AHEAD"</Eyebrow>
                    <CardTitle>{title}</CardTitle>
                </div>
                <p class="max-w-60 text-sm text-fg-muted sm:text-right">{status}</p>
            </header>
            {salary_line}
            <div class="grid gap-4 sm:grid-cols-2">
                <Stat label="Following your plan" delta=planned_delta>{planned}</Stat>
                <Stat label="Continuing current habits" delta=expected_delta>{expected}</Stat>
            </div>
            {frame}
            <div class="grid gap-4 border-t border-border pt-5 sm:grid-cols-3">
                <Stat label="Last three complete days" delta=recent_delta>{recent}</Stat>
                <Stat
                    label="Behavior score"
                    delta=if alert { "Spending needs attention" } else { "See the evidence below" }
                    tone=if alert { Tone::Negative } else { Tone::Neutral }
                >
                    <span class="text-xl">{score}</span>
                </Stat>
                <Stat label="Expected unplanned spending" delta="over the next three months">{unplanned}</Stat>
            </div>
            <details class="grid gap-3 text-sm leading-relaxed [&>*]:mt-3">
                <summary class="cursor-pointer font-medium">"Unplanned spending, refunds & forecast evidence"</summary>
                <p>{evidence}</p>
                {deductions}
                <div tabindex="0" role="region" aria-label="Unplanned spending by month" class=TABLE_CONTAINER>
                    <table class=TABLE data-density="md">
                        <caption class=CAPTION>"Unplanned spending by month"</caption>
                        <thead class="text-left">
                            <tr class=cn([TROW, "border-t-0"])>
                                <th class=cn([TH, "text-left"])>"Month"</th>
                                <th class=TH_NUM>"Net spending"</th>
                                <th class=TH_NUM>{format!("Through day {as_of_day}")}</th>
                                <th class=TH_NUM>"Share of variable spending"</th>
                                <th class=TH_NUM>"Purchases"</th>
                            </tr>
                        </thead>
                        <tbody>{history}</tbody>
                    </table>
                </div>
                <p>{model}</p>
                {errors}
                <ul class="list-disc pl-5">{assumptions}</ul>
                // The app sends no email; alerts would need a sender first.
                <p>"Email alerts: Sender setup required."</p>
            </details>
        </div>
    }
    .into_any();
    view! { <Card class="grid gap-5">{content}</Card> }.into_any()
}
