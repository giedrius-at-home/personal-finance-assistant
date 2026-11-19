//! The row of headline fields across the top of the Overview: where the
//! month stands, in numbers, before any detail.

use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::{METRIC_STRIP, Metric, Tone, cn, money};
use crate::format;
use crate::pages::with_report;

use super::visuals::{signed, verdict};

#[component]
pub fn OverviewKpis() -> impl IntoView {
    with_report(strip)
}

fn strip(report: &Dashboard) -> Option<AnyView> {
    let a = report.monthly_allocation.as_ref()?.currencies.first()?;
    let unit = a.currency.clone();
    let m = |v: Decimal| money(v, &unit);
    let zero = Decimal::ZERO;
    let through = &report.monthly_spending.through;
    let month = format!("{}-01", &through[..7.min(through.len())]);

    let next_short = a.next_planned_funds > zero && a.next_shortfall > zero;
    let (spend_tone, spend_word, spend_facts) = if a.salary_at_risk > zero {
        (
            Tone::Warning,
            "Salary at risk",
            vec![(
                "Of next month\u{2019}s salary".to_owned(),
                m(a.salary_at_risk),
            )],
        )
    } else if next_short {
        (
            Tone::Warning,
            "Next month short",
            vec![("Missing next month".to_owned(), m(a.next_shortfall))],
        )
    } else {
        (Tone::Positive, "Covered", Vec::new())
    };
    let spend_value = m(a.current_wiggle_room);
    let spend_class = if a.current_wiggle_room < zero {
        "text-negative-fg"
    } else {
        ""
    };
    let spendable = view! {
        <Metric label="Spendable now" chip=(spend_tone, spend_word.to_owned()) facts=spend_facts>
            <span class=spend_class>{spend_value}</span>
        </Metric>
    }
    .into_any();

    let budget = report
        .monthly_budgets
        .iter()
        .find(|b| b.plan.month == month && b.plan.currency.code == unit.code);
    let plan = budget.map(|budget| {
        let days = format::days_in_month(&budget.plan.month) as usize;
        let v = verdict(budget, days);
        let month_end = v.ahead.map(|ahead| {
            let class = if ahead > zero {
                "text-negative-fg"
            } else {
                "text-positive-fg"
            };
            let text = signed(ahead, &unit);
            view! {
                <Metric label="Month end vs plan">
                    <span class=class>{text}</span>
                </Metric>
            }
        });
        let facts = vec![
            ("Planned".to_owned(), m(budget.planned)),
            ("Spent".to_owned(), m(budget.spent)),
        ];
        let class = if v.net < zero { "text-negative-fg" } else { "" };
        let value = m(v.net.abs());
        view! {
            <Metric
                label=if v.net < zero { "Over plan" } else { "Left in plan" }
                chip=(v.tone, v.word.to_owned())
                facts=facts
            >
                <span class=class>{value}</span>
            </Metric>
            {month_end}
        }
        .into_any()
    });

    let saved = m(a.savings_balance);
    let saved_facts = vec![(
        "By month end".to_owned(),
        m(a.current_wiggle_room.max(zero) + a.savings_balance),
    )];
    let savings =
        view! { <Metric label="In savings" facts=saved_facts>{saved}</Metric> }.into_any();

    let week = report
        .daily_forecasts
        .iter()
        .find(|d| d.currency.code == unit.code)
        .map(|d| {
            let value = m(d.next_week_habit + d.next_week_planned);
            let facts = vec![
                ("Habits".to_owned(), m(d.next_week_habit)),
                ("Planned".to_owned(), m(d.next_week_planned)),
            ];
            view! { <Metric label="Next 7 days" facts=facts>{value}</Metric> }.into_any()
        });

    let score = report
        .savings_outlooks
        .iter()
        .find(|o| o.currency.code == unit.code)
        .map(|o| {
            let value = o
                .behavior_score
                .map_or("Paused".to_owned(), |s| format!("{s}/100"));
            let facts = vec![("Last three days".to_owned(), m(o.signal.spending))];
            if o.signal.alert {
                view! {
                    <Metric label="Behavior score" chip=(Tone::Negative, "Needs attention".to_owned()) facts=facts>
                        {value}
                    </Metric>
                }
                .into_any()
            } else {
                view! { <Metric label="Behavior score" facts=facts>{value}</Metric> }.into_any()
            }
        });

    Some(
        view! {
            <section
                aria-label="This month at a glance"
                class=cn([METRIC_STRIP, "xl:grid-cols-[repeat(auto-fit,minmax(12rem,1fr))]"])
            >
                {spendable}
                {plan}
                {savings}
                {week}
                {score}
            </section>
        }
        .into_any(),
    )
}
