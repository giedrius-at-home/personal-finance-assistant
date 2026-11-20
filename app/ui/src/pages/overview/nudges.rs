//! What could be better right now, as fields under the headline numbers:
//! every group over its limit or its own norm with what is left of it, how
//! many visits that still pays for (this month, a week, and next month), a
//! fast last three days, and unsorted spending. The advice holds spending
//! to the budget; it never asks for a bigger one.

use finance_types::report::Dashboard;
use finance_types::report::adjustments::Habit;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ds::{
    Card, CardTitle, Eyebrow, Heading, Icon, Lucide, Metric, Size, Tone, Variant, button_class, cn,
    money,
};
use crate::pages::with_report;
use crate::{SpendingSection, state};

use super::{month_name, remaining_days};

/// The most a list shows: the heaviest ones.
const MOST: usize = 6;
/// Spending this far above the usual month is worth a word (and at least
/// `MIN_OVER` in money).
const OVER_RATIO: Decimal = Decimal::from_parts(12, 0, 0, false, 1);
const MIN_OVER: Decimal = Decimal::TEN;
const WEEK: Decimal = Decimal::from_parts(7, 0, 0, false, 0);
/// Weeks in a month: 52 / 12.
const MONTH_WEEKS: Decimal = Decimal::from_parts(4333, 0, 0, false, 3);

/// One thing to improve, as a field: `weight` is the money it is about,
/// for ordering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nudge {
    pub label: String,
    pub value: String,
    /// The value is an overrun.
    pub bad: bool,
    pub chip: Option<(Tone, String)>,
    pub facts: Vec<(String, String)>,
    pub weight: Decimal,
}

/// How far `habit` runs above its own norm: `(spent above the usual month,
/// visits above the usual week)`, each `None` when within the norm or
/// without history.
pub fn above_norm(habit: &Habit) -> (Option<Decimal>, Option<Decimal>) {
    let spent = habit.typical_spent.and_then(|usual| {
        let over = habit.spent - usual;
        (usual > Decimal::ZERO && habit.spent > usual * OVER_RATIO && over >= MIN_OVER)
            .then_some(over)
    });
    let visits = Decimal::from(habit.visits_this_week);
    let extra = visits - habit.typical_week_visits;
    let visits = (habit.visits_this_week >= 2 && extra > Decimal::ONE).then_some(extra);
    (spent, visits)
}

/// What a visit costs: this month's average, else the usual month's; zero
/// when neither is known.
pub fn ticket(habit: &Habit) -> Decimal {
    if habit.visits > 0 {
        return (habit.spent / Decimal::from(habit.visits)).max(Decimal::ZERO);
    }
    match (habit.typical_spent, habit.typical_visits) {
        (Some(spent), Some(visits)) if visits > Decimal::ZERO => {
            (spent / visits).max(Decimal::ZERO)
        }
        _ => Decimal::ZERO,
    }
}

/// What a group heading to `expected` ends over `limit`; `None` inside the
/// limit or without one.
pub fn over_limit(expected: Decimal, limit: Decimal) -> Option<Decimal> {
    (limit > Decimal::ZERO && expected > limit).then(|| expected - limit)
}

/// What staying inside a cap still allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allowance {
    /// `cap − spent`; negative once over.
    pub left: Decimal,
    /// Whole visits the rest pays for; `None` without a visit price.
    pub visits_left: Option<u32>,
    /// Money a week over the days left (all of it in the last week).
    pub week: Decimal,
    pub week_visits: Option<Decimal>,
}

/// The rest of the month inside `cap`, over `days`, at `ticket` a visit.
pub fn allowance(cap: Decimal, spent: Decimal, ticket: Decimal, days: u32) -> Allowance {
    let left = cap - spent;
    let room = left.max(Decimal::ZERO);
    let weeks = (Decimal::from(days) / WEEK).max(Decimal::ONE);
    let visits = (ticket > Decimal::ZERO).then(|| (room / ticket).floor());
    Allowance {
        left,
        visits_left: visits.and_then(|v| v.to_u32()),
        week: (room / weeks).round_dp(2),
        week_visits: visits.map(|v| (v / weeks).round_dp(1)),
    }
}

/// A full month of `cap` as a week: `(money, visits)`.
pub fn month_week(cap: Decimal, ticket: Decimal) -> (Decimal, Option<Decimal>) {
    let week = (cap.max(Decimal::ZERO) / MONTH_WEEKS).round_dp(2);
    let visits = (ticket > Decimal::ZERO).then(|| (week / ticket).round_dp(1));
    (week, visits)
}

fn visits_text(count: Decimal) -> String {
    let count = count.normalize();
    if count == Decimal::ONE {
        "1 visit".to_owned()
    } else {
        format!("{count} visits")
    }
}

/// Every nudge for the first currency, heaviest first, at most [`MOST`].
pub fn nudges(report: &Dashboard) -> Vec<Nudge> {
    let Some(a) = report
        .monthly_allocation
        .as_ref()
        .and_then(|m| m.currencies.first())
    else {
        return Vec::new();
    };
    let unit = &a.currency;
    let m = |v: Decimal| money(v, unit);
    let zero = Decimal::ZERO;
    let mut out = Vec::new();

    let days = remaining_days(&report.monthly_spending.through);
    let next_month = if a.funded_month == a.current_month {
        "Next month".to_owned()
    } else {
        month_name(&a.funded_month)
    };
    let allowance_text = |money: Decimal, visits: Option<Decimal>| match visits {
        Some(v) => format!("{} · {}", m(money), visits_text(v)),
        None => m(money),
    };

    for adjustments in report.adjustments.iter().filter(|x| x.currency == *unit) {
        for row in &adjustments.rows {
            let name = &row.row.name;
            let h = &row.habit;
            let limit = (row.budgeted && row.row.planned > zero).then_some(row.row.planned);
            // Held to the limit; without one, to the owner's own usual month.
            let cap = limit.or(h.typical_spent.filter(|usual| *usual > zero));
            let (norm_over, extra_visits) = above_norm(h);
            let expected = report
                .spending_forecasts
                .iter()
                .find(|f| f.currency == *unit && f.group == *name)
                .map(|f| f.expected);
            let heading = limit.and_then(|l| over_limit(expected.unwrap_or_default(), l));
            let over = cap.map(|c| h.spent - c).filter(|o| *o > zero);
            let stands_out = match limit {
                Some(_) => over.is_some() || heading.is_some(),
                None => norm_over.is_some(),
            } || extra_visits.is_some();
            if !stands_out {
                continue;
            }
            let price = ticket(h);
            let rest = cap.map(|c| allowance(c, h.spent, price, days));
            let cap_label = if limit.is_some() {
                "Limit"
            } else {
                "Usual month"
            };
            let (value, bad, chip) = match (&rest, over) {
                (Some(_), Some(over)) => (
                    format!("{} over", m(over)),
                    true,
                    (Tone::Negative, format!("Over {}", cap_label.to_lowercase())),
                ),
                (Some(rest), None) => (
                    format!("{} left", m(rest.left)),
                    false,
                    if heading.is_some() {
                        (Tone::Warning, "Heading over".to_owned())
                    } else {
                        (Tone::Warning, "More visits".to_owned())
                    },
                ),
                (None, _) => (m(h.spent), false, (Tone::Warning, "More visits".to_owned())),
            };

            let mut facts = vec![("Spent".to_owned(), m(h.spent))];
            if let Some(cap) = cap {
                facts.push((cap_label.to_owned(), m(cap)));
            }
            if let (Some(_), Some(expected)) = (heading, expected) {
                facts.push(("Month end at this pace".to_owned(), m(expected)));
            }
            facts.push((
                "Visits".to_owned(),
                format!("{} this month · {} this week", h.visits, h.visits_this_week),
            ));
            facts.push((
                "Usually".to_owned(),
                match h.typical_visits {
                    Some(month) => format!(
                        "{} a month · {} a week",
                        month.normalize(),
                        h.typical_week_visits.normalize()
                    ),
                    None => format!("{} a week", h.typical_week_visits.normalize()),
                },
            ));
            if price > zero {
                facts.push(("A visit costs".to_owned(), m(price)));
            }
            if let Some(rest) = &rest {
                if let Some(visits) = rest.visits_left {
                    facts.push(("Visits left this month".to_owned(), visits.to_string()));
                }
                if rest.left > zero {
                    facts.push((
                        "A week, to stay inside".to_owned(),
                        allowance_text(rest.week, rest.week_visits),
                    ));
                }
            }
            // Next month starts from its own limit: an overrun now does not
            // raise it.
            let next_cap = a
                .next_group_rows
                .iter()
                .find(|g| g.group == *name)
                .map(|g| g.limit)
                .filter(|l| *l > zero)
                .or(cap);
            if let Some(next_cap) = next_cap {
                let (week, visits) = month_week(next_cap, price);
                // Less than a visit a week reads better as the whole month.
                if visits.is_some_and(|v| v < Decimal::ONE) {
                    let month = allowance(next_cap, zero, price, 31);
                    facts.push((
                        next_month.clone(),
                        allowance_text(next_cap, month.visits_left.map(Decimal::from)),
                    ));
                } else {
                    facts.push((
                        format!("{next_month}, a week"),
                        allowance_text(week, visits),
                    ));
                }
            }
            // The one store that drives the group most above its norm.
            let store = row
                .stores
                .iter()
                .filter(|s| s.category != *name)
                .filter_map(|s| above_norm(&s.habit).0.map(|over| (s, over)))
                .max_by_key(|(_, over)| *over);
            if let Some((store, _)) = store {
                facts.push((
                    format!("Mostly {}", store.category),
                    format!(
                        "{} · {}",
                        m(store.habit.spent),
                        visits_text(Decimal::from(store.habit.visits))
                    ),
                ));
            }
            let weight = [over, heading, norm_over, extra_visits.map(|e| e * price)]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or_default();
            out.push(Nudge {
                label: name.clone(),
                value,
                bad,
                chip: Some(chip),
                facts,
                weight,
            });
        }
        if adjustments.unsorted > zero {
            out.push(Nudge {
                label: "Not sorted into a group".to_owned(),
                value: m(adjustments.unsorted),
                bad: false,
                chip: Some((Tone::Neutral, "Sort it".to_owned())),
                facts: Vec::new(),
                weight: adjustments.unsorted / Decimal::TWO,
            });
        }
    }

    if let Some(outlook) = report.savings_outlooks.iter().find(|o| o.currency == *unit) {
        let s = &outlook.signal;
        if s.accelerating || s.excessive_speed {
            out.push(Nudge {
                label: "Last three days".to_owned(),
                value: m(s.spending),
                bad: false,
                chip: Some((Tone::Warning, "Faster".to_owned())),
                facts: vec![
                    ("Three days before".to_owned(), m(s.previous_spending)),
                    ("Unplanned".to_owned(), m(s.unplanned)),
                ],
                weight: (s.spending - s.previous_spending).max(s.unplanned),
            });
        }
    }

    out.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label.cmp(&b.label)));
    out.truncate(MOST);
    out
}

#[component]
pub fn OverviewNudges() -> impl IntoView {
    with_report(|report| {
        report.monthly_allocation.as_ref()?;
        let app = state();
        let list = nudges(report);
        let body = if list.is_empty() {
            view! { <p class="mt-2 text-sm text-fg-muted">"Nothing stands out: spending is within your own norms and limits."</p> }
                .into_any()
        } else {
            let rows = list
                .into_iter()
                .map(|n| {
                    let class = if n.bad { "text-negative-fg" } else { "" };
                    let value = n.value;
                    let field = match n.chip {
                        Some(chip) => view! {
                            <Metric label=n.label chip=chip facts=n.facts size="text-xl">
                                <span class=class>{value}</span>
                            </Metric>
                        }
                        .into_any(),
                        None => view! {
                            <Metric label=n.label facts=n.facts size="text-xl">
                                <span class=class>{value}</span>
                            </Metric>
                        }
                        .into_any(),
                    };
                    view! { <li class="min-w-0">{field}</li> }
                })
                .collect_view();
            view! { <ul class="mt-4 grid gap-x-12 gap-y-7 sm:grid-cols-2 xl:grid-cols-3">{rows}</ul> }.into_any()
        };
        Some(view! {
            <Card>
                <header class="flex items-start justify-between gap-4">
                    <div>
                        <Eyebrow level=Heading::P>"WORTH A LOOK"</Eyebrow>
                        <CardTitle level=Heading::H2 class="mt-1">"What could be better"</CardTitle>
                    </div>
                    <button
                        type="button"
                        class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), "shrink-0 text-accent"])
                        on:click=move |_| app.go_spending(SpendingSection::PlanPatterns)
                    >
                        "Habits "
                        <Lucide icon=Icon::ArrowRight size=14 />
                    </button>
                </header>
                {body}
            </Card>
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn habit(spent: &str, usual: Option<&str>, week: usize, usual_week: &str) -> Habit {
        Habit {
            visits: 4,
            spent: d(spent),
            typical_spent: usual.map(d),
            visits_this_week: week,
            typical_week_visits: d(usual_week),
            ..Habit::default()
        }
    }

    #[test]
    fn spending_counts_once_clearly_above_the_usual_month() {
        assert_eq!(
            above_norm(&habit("150", Some("100"), 0, "0")).0,
            Some(d("50"))
        );
        // Within a fifth of the norm, a small amount, or no history: quiet.
        assert_eq!(above_norm(&habit("115", Some("100"), 0, "0")).0, None);
        assert_eq!(above_norm(&habit("9", Some("3"), 0, "0")).0, None);
        assert_eq!(above_norm(&habit("150", None, 0, "0")).0, None);
    }

    #[test]
    fn visits_count_when_more_than_one_above_the_usual_week() {
        assert_eq!(above_norm(&habit("0", None, 4, "2")).1, Some(d("2")));
        assert_eq!(above_norm(&habit("0", None, 3, "2")).1, None);
        assert_eq!(above_norm(&habit("0", None, 1, "0")).1, None);
    }

    #[test]
    fn a_group_heading_past_its_limit_is_over_by_the_difference() {
        assert_eq!(over_limit(d("360"), d("300")), Some(d("60")));
        assert_eq!(over_limit(d("290"), d("300")), None);
        assert_eq!(over_limit(d("290"), d("0")), None);
    }

    #[test]
    fn the_rest_of_a_cap_becomes_visits_and_a_weekly_amount() {
        // €120 left over 14 days at €25 a visit: 4 visits, 2 a week.
        let rest = allowance(d("300"), d("180"), d("25"), 14);
        assert_eq!(rest.left, d("120"));
        assert_eq!(rest.visits_left, Some(4));
        assert_eq!(rest.week, d("60"));
        assert_eq!(rest.week_visits, Some(d("2")));
        // The last days are one week: all that is left.
        let rest = allowance(d("300"), d("240"), d("25"), 3);
        assert_eq!((rest.week, rest.visits_left), (d("60"), Some(2)));
        // Over the cap: nothing left, and the cap does not grow.
        let rest = allowance(d("74.74"), d("306.10"), d("61.22"), 10);
        assert_eq!(rest.left, d("-231.36"));
        assert_eq!((rest.visits_left, rest.week), (Some(0), d("0")));
        // No visit price: money only.
        let rest = allowance(d("100"), d("30"), d("0"), 7);
        assert_eq!((rest.visits_left, rest.week_visits), (None, None));
    }

    #[test]
    fn a_visit_costs_this_months_average_or_the_usual_one() {
        assert_eq!(ticket(&habit("100", Some("60"), 0, "0")), d("25"));
        let quiet = Habit {
            typical_spent: Some(d("60")),
            typical_visits: Some(d("3")),
            ..Habit::default()
        };
        assert_eq!(ticket(&quiet), d("20"));
        assert_eq!(ticket(&Habit::default()), d("0"));
    }

    #[test]
    fn next_month_is_its_own_limit_spread_over_the_weeks() {
        assert_eq!(month_week(d("100"), d("10")), (d("23.08"), Some(d("2.3"))));
        assert_eq!(month_week(d("100"), d("0")), (d("23.08"), None));
    }
}
