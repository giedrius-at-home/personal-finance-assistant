//! The row of headline fields across the top of the Overview: where the
//! month stands, in numbers, before any detail.

use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::{METRIC_STRIP, Metric, Tone, cn, money};
use crate::format;
use crate::pages::with_report;

use super::outlook::score_reasons;
use super::visuals::{signed, verdict};

#[component]
pub fn OverviewKpis() -> impl IntoView {
    with_report(strip)
}

/// A bank shortfall split into what the savings above the wishes cover
/// and what would eat into the next salary: `(from_savings, at_risk)`.
pub(super) fn bank_gap(spendable: Decimal, savings_room: Decimal) -> (Decimal, Decimal) {
    let gap = (-spendable).max(Decimal::ZERO);
    let from_savings = gap.min(savings_room.max(Decimal::ZERO));
    (from_savings, gap - from_savings)
}

fn strip(report: &Dashboard) -> Option<AnyView> {
    let a = report.monthly_allocation.as_ref()?.currencies.first()?;
    let unit = a.currency.clone();
    let m = |v: Decimal| money(v, &unit);
    let zero = Decimal::ZERO;
    let through = &report.monthly_spending.through;
    let month = format!("{}-01", &through[..7.min(through.len())]);

    let next_short = a.next_planned_funds > zero && a.next_shortfall > zero;
    let savings_room = report
        .money_plan
        .as_ref()
        .and_then(|p| p.savings.as_ref())
        .map_or(zero, |s| s.balance - s.must_keep);
    let (from_savings, at_risk) = bank_gap(a.current_wiggle_room, savings_room);
    // Never a negative headline: a gap shows as what is short and who pays.
    let gap = (-a.current_wiggle_room).max(zero);
    let (spend_tone, spend_word, spend_facts) = if at_risk > zero {
        let mut facts = vec![
            ("Short".to_owned(), m(gap)),
            ("Of next month\u{2019}s salary".to_owned(), m(at_risk)),
        ];
        if from_savings > zero {
            facts.insert(1, ("Take from savings".to_owned(), m(from_savings)));
        }
        (Tone::Negative, "Salary at risk", facts)
    } else if from_savings > zero {
        (
            Tone::Warning,
            "Savings cover it",
            vec![
                ("Short".to_owned(), m(gap)),
                ("Take from savings".to_owned(), m(from_savings)),
            ],
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
    let spend_value = m(a.current_wiggle_room.max(zero));
    let spendable = view! {
        <Metric label="Spendable now" chip=(spend_tone, spend_word.to_owned()) facts=spend_facts>
            {spend_value}
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
                "text-over-fg"
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
        let class = if v.net < zero { "text-over-fg" } else { "" };
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

    // Spending outside every group: what the month is expected to cost,
    // what came so far, and what is still expected.
    let unplanned = budget
        .filter(|b| !b.unplanned.expected.is_empty())
        .map(|b| {
            let u = &b.unplanned;
            let value = format!("\u{2248} {}", m(u.planned));
            let facts = vec![
                ("So far".to_owned(), m(u.spent)),
                (
                    "Still expected".to_owned(),
                    m((u.planned - u.spent).max(zero)),
                ),
            ];
            view! { <Metric label="Unplanned this month" facts=facts>{value}</Metric> }.into_any()
        });

    let saved = m(a.savings_balance);
    let saved_facts = vec![(
        "By month end".to_owned(),
        m((a.current_wiggle_room + a.savings_balance).max(zero)),
    )];
    let savings =
        view! { <Metric label="In savings" facts=saved_facts>{saved}</Metric> }.into_any();

    let week = report
        .daily_forecasts
        .iter()
        .find(|d| d.currency.code == unit.code)
        .map(|d| {
            let value = m(super::forecast::next_week(d));
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
            // What costs points, heaviest first; the way back is in the
            // outlook card.
            let facts: Vec<(String, String)> = score_reasons(o)
                .into_iter()
                .map(|r| (r.why.to_owned(), format!("\u{2212}{}", r.points)))
                .collect();
            let chip = o
                .signal
                .alert
                .then(|| (Tone::Warning, "Needs attention".to_owned()));
            match chip {
                Some(chip) => {
                    view! { <Metric label="Habit score" chip=chip facts=facts>{value}</Metric> }
                        .into_any()
                }
                None => {
                    view! { <Metric label="Habit score" facts=facts>{value}</Metric> }.into_any()
                }
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
                {unplanned}
                {savings}
                {week}
                {score}
            </section>
        }
        .into_any(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn savings_take_a_bank_gap_before_the_salary_does() {
        let d = |t: &str| t.parse::<Decimal>().unwrap();
        assert_eq!(bank_gap(d("-11.74"), d("611.36")), (d("11.74"), d("0")));
        assert_eq!(bank_gap(d("-50"), d("20")), (d("20"), d("30")));
        assert_eq!(bank_gap(d("-50"), d("-5")), (d("0"), d("50")));
        assert_eq!(bank_gap(d("10"), d("100")), (d("0"), d("0")));
    }
}
