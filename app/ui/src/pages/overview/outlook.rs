//! The three-month savings outlook (`savings-outlook.tsx`).

use finance_types::report::outlook::{SavingsOutlook, ThreeDaySignal};
use finance_types::report::{Currency, Dashboard};
use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::ds::charts::{LineChart, Series, fmt, series_color};
use crate::ds::{
    Badge, Card, CardDescription, CardHeader, CardTitle, Eyebrow, Facts, Heading, METRIC_STRIP,
    Metric, Money, TABLE, TABLE_CONTAINER, TCELL_NUM, TH, TH_NUM, TROW, Tone, cn, money,
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
                    <CardTitle class="text-sm text-fg-muted">{title}</CardTitle>
                    {(!description.is_empty()).then(|| view! { <CardDescription>{description}</CardDescription> })}
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
                <span title="Prepared in the background. Refresh shortly."><Badge tone=Tone::Outline>"Preparing"</Badge></span>
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

    let (status_tone, status) = if !outlook.data_fresh {
        (Tone::Neutral, "Waiting for bank data".to_owned())
    } else {
        match outlook.on_track_probability {
            None if outlook.expected_savings < outlook.planned_savings => {
                (Tone::Warning, "Plan at risk".into())
            }
            None => (Tone::Neutral, "Limited evidence".into()),
            Some(p) => {
                let text = if p < Decimal::new(1, 2) {
                    "<1".to_owned()
                } else if p > Decimal::new(99, 2) {
                    ">99".to_owned()
                } else {
                    percent(p).to_string()
                };
                let tone = if p < Decimal::new(5, 1) {
                    Tone::Warning
                } else {
                    Tone::Positive
                };
                (tone, format!("{text}% chance on plan"))
            }
        }
    };
    let salary_line = salary.map(|s| {
        let day: u32 = s
            .due_on
            .get(8..10)
            .and_then(|d| d.parse().ok())
            .unwrap_or(1);
        view! {
            <div class="grid gap-2 border-t border-line pt-4">
                <Facts rows=vec![
                    ("Monthly salary".to_owned(), m(s.amount.amount)),
                    ("Pay day".to_owned(), day.to_string()),
                ] />
                <a
                    class="text-xs text-fg-muted underline underline-offset-2"
                    href="?view=settings"
                    on:click=move |e| {
                        e.prevent_default();
                        app.go_manage(ManageSection::Settings);
                    }
                >
                    "Edit salary"
                </a>
            </div>
        }
    });
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
            height=300
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
    let as_of_day: u32 = outlook
        .as_of
        .get(8..10)
        .and_then(|d| d.parse().ok())
        .unwrap_or(1);
    let title = format!("Your money by {}", format::day_month(&outlook.through));
    let planned = amount(outlook.planned_savings, "");
    let planned_facts = vec![("Final balance".to_owned(), m(outlook.planned_final))];
    let expected = amount(outlook.expected_savings, "");
    let expected_facts = vec![
        ("Final balance".to_owned(), m(outlook.expected_final)),
        (
            "Estimated range".to_owned(),
            format!("{} – {}", m(outlook.lower_final), m(outlook.upper_final)),
        ),
    ];
    let frame = chart_frame("Balance projection", String::new(), 340, chart);
    let recent = amount(outlook.signal.spending, "");
    let recent_facts = vec![
        (
            "Three days before".to_owned(),
            m(outlook.signal.previous_spending),
        ),
        ("Unplanned".to_owned(), m(outlook.signal.unplanned)),
    ];
    let alert = outlook.signal.alert;
    let score = outlook
        .behavior_score
        .map_or("Paused".to_owned(), |s| format!("{s}/100"));
    let score_chip = alert.then(|| (Tone::Warning, "Needs attention".to_owned()));
    let reasons = score_reasons(&outlook);
    let score_help = (!reasons.is_empty()).then(|| {
        let rows = reasons
            .into_iter()
            .map(|r| {
                let fix = r.fix(&outlook.signal, &unit);
                view! {
                    <li class="grid min-w-0 content-start gap-1.5 rounded-lg border border-border bg-card-strong p-3">
                        <span class="flex items-center justify-between gap-3 text-eyebrow font-bold uppercase tracking-eyebrow text-fg-muted">
                            {r.why}
                            <Badge tone=Tone::Warning>{format!("\u{2212}{} points", r.points)}</Badge>
                        </span>
                        <strong class="text-sm font-semibold">{r.action}</strong>
                        <Facts rows=fix />
                    </li>
                }
            })
            .collect_view();
        view! {
            <section aria-label="How to raise the habit score" class="grid gap-3">
                <Eyebrow level=Heading::P>"RAISE YOUR SCORE"</Eyebrow>
                <ul class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">{rows}</ul>
            </section>
        }
    });
    let unplanned = amount(outlook.expected_unplanned, "");
    let unplanned_facts = vec![(
        "Rest of this month".to_owned(),
        m(outlook.expected_unplanned_month),
    )];
    let evidence = vec![
        (
            "Refunds received this month".to_owned(),
            m(outlook.received_refunds),
        ),
        (
            "Expected income over the forecast".to_owned(),
            m(outlook.expected_income),
        ),
        (
            "Starting included cash".to_owned(),
            m(outlook.starting_balance),
        ),
    ];
    let content = view! {
        <div role="region" aria-label="Three-month savings forecast" class="contents">
            <header class="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <Eyebrow level=Heading::P>"NEXT THREE MONTHS"</Eyebrow>
                    <CardTitle>{title}</CardTitle>
                </div>
                <Badge tone=status_tone>{status}</Badge>
            </header>
            <div class="grid gap-x-10 gap-y-6 lg:grid-cols-[minmax(240px,.4fr)_minmax(0,1.6fr)]">
                <div class="grid content-start gap-6">
                    <Metric label="Following your plan" facts=planned_facts>{planned}</Metric>
                    <Metric label="Spending like now" facts=expected_facts>{expected}</Metric>
                    {salary_line}
                </div>
                {frame}
            </div>
            <div class=cn([METRIC_STRIP, "border-t border-line pt-5"])>
                <Metric label="Last three days" size="text-xl" facts=recent_facts>{recent}</Metric>
                {match score_chip {
                    Some(chip) => view! { <Metric label="Habit score" size="text-xl" chip=chip>{score}</Metric> }.into_any(),
                    None => view! { <Metric label="Habit score" size="text-xl">{score}</Metric> }.into_any(),
                }}
                <Metric label="Unplanned, next 3 months" size="text-xl" facts=unplanned_facts>{unplanned}</Metric>
            </div>
            {score_help}
            <details class="grid gap-3 text-sm leading-relaxed [&>*]:mt-3">
                <summary class="cursor-pointer font-medium">"Unplanned spending, month by month"</summary>
                <Facts rows=evidence class="max-w-md" />
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
            </details>
        </div>
    }
    .into_any();
    view! { <Card class="grid gap-5">{content}</Card> }.into_any()
}

/// One thing that costs habit-score points, with the way to win them back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreReason {
    pub points: u32,
    pub why: &'static str,
    pub action: &'static str,
    kind: ReasonKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReasonKind {
    Speed,
    Rise,
    Unusual,
    Month,
    Other,
}

impl ScoreReason {
    /// The numbers to aim for.
    pub fn fix(&self, signal: &ThreeDaySignal, unit: &Currency) -> Vec<(String, String)> {
        let m = |v: Decimal| money(v.max(Decimal::ZERO), unit);
        match self.kind {
            ReasonKind::Speed => vec![("Next three days, at most".to_owned(), m(signal.allowance))],
            ReasonKind::Rise => vec![(
                "Next three days, at most".to_owned(),
                m(signal.previous_spending),
            )],
            ReasonKind::Unusual => {
                vec![("Unplanned, last three days".to_owned(), m(signal.unplanned))]
            }
            ReasonKind::Month => vec![
                (
                    "Every".to_owned(),
                    format!("{} unplanned", m(Decimal::from(50))),
                ),
                ("Costs".to_owned(), "5 points".to_owned()),
            ],
            ReasonKind::Other => Vec::new(),
        }
    }
}

/// The score's deductions in plain words, heaviest first.
pub fn score_reasons(outlook: &SavingsOutlook) -> Vec<ScoreReason> {
    let mut out: Vec<ScoreReason> = outlook
        .deductions
        .iter()
        .map(|d| {
            let (kind, why, action) = if d.reason.starts_with("Three-day") {
                (
                    ReasonKind::Speed,
                    "Fast three days",
                    "Spend less for three days",
                )
            } else if d.reason.starts_with("Spending rose") {
                (
                    ReasonKind::Rise,
                    "Spending jumped",
                    "Go back to last week\u{2019}s pace",
                )
            } else if d.reason.starts_with("Unplanned purchases") {
                (
                    ReasonKind::Unusual,
                    "Many unplanned buys",
                    "Skip unplanned buys for three days",
                )
            } else if d.reason.starts_with("At least") {
                (
                    ReasonKind::Month,
                    "Unplanned this month",
                    "Plan buys ahead; resets next month",
                )
            } else {
                (ReasonKind::Other, "Other", "")
            };
            ScoreReason {
                points: d.points,
                why,
                action,
                kind,
            }
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.points));
    out
}
