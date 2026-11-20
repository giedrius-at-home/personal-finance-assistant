//! What could be better right now, in one list under the headline fields:
//! groups and stores running above the owner's own norm, groups heading
//! over their limit, a fast last three days, and unsorted spending.

use finance_types::report::Dashboard;
use finance_types::report::adjustments::Habit;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::{
    Card, CardTitle, Eyebrow, Heading, Icon, Lucide, Size, Variant, button_class, cn, money,
};
use crate::pages::with_report;
use crate::{SpendingSection, state};

use super::remaining_days;

/// The most a list shows: the heaviest ones.
const MOST: usize = 6;
/// Spending this far above the usual month is worth a word (and at least
/// `MIN_OVER` in money).
const OVER_RATIO: Decimal = Decimal::from_parts(12, 0, 0, false, 1);
const MIN_OVER: Decimal = Decimal::TEN;

/// One thing to improve; `weight` is the money it is about, for ordering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nudge {
    pub title: String,
    pub detail: String,
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

/// What a visit costs this month, for weighing extra visits.
fn per_visit(habit: &Habit) -> Decimal {
    if habit.visits == 0 {
        Decimal::ZERO
    } else {
        (habit.spent / Decimal::from(habit.visits)).max(Decimal::ZERO)
    }
}

/// What a group heading to `expected` is over `limit`, and the daily amount
/// that keeps it inside over `days`; `None` inside the limit or without one.
pub fn over_limit(
    spent: Decimal,
    expected: Decimal,
    limit: Decimal,
    days: u32,
) -> Option<(Decimal, Decimal)> {
    (limit > Decimal::ZERO && expected > limit).then(|| {
        let daily = ((limit - spent).max(Decimal::ZERO) / Decimal::from(days.max(1))).round_dp(2);
        (expected - limit, daily)
    })
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
    let mut out = Vec::new();

    let days = remaining_days(&report.monthly_spending.through);
    let mut heading_over: Vec<&str> = Vec::new();
    for forecast in report
        .spending_forecasts
        .iter()
        .filter(|f| f.currency == *unit)
    {
        let limit = a
            .groups
            .iter()
            .find(|g| g.group == forecast.group)
            .map_or(Decimal::ZERO, |g| g.limit);
        if let Some((over, daily)) = over_limit(forecast.spent, forecast.expected, limit, days) {
            heading_over.push(&forecast.group);
            out.push(Nudge {
                title: format!("{} is heading {} over its limit", forecast.group, m(over)),
                detail: if daily > Decimal::ZERO {
                    format!(
                        "{} a day for the rest of the month keeps it inside.",
                        m(daily)
                    )
                } else {
                    "The limit is already spent: anything more adds to the overrun.".to_owned()
                },
                weight: over,
            });
        }
    }

    for adjustments in report.adjustments.iter().filter(|x| x.currency == *unit) {
        for row in &adjustments.rows {
            let name = &row.row.name;
            let (spent, visits) = above_norm(&row.habit);
            // A group already named for its limit is not named twice.
            if let (Some(over), false) = (spent, heading_over.contains(&name.as_str())) {
                out.push(Nudge {
                    title: format!("{name}: {} above your usual month", m(over)),
                    detail: format!(
                        "{} so far against a usual {}.",
                        m(row.habit.spent),
                        m(row.habit.typical_spent.unwrap_or_default())
                    ),
                    weight: over,
                });
            }
            if let Some(extra) = visits {
                out.push(Nudge {
                    title: format!(
                        "{name}: {} visits this week, usually {}",
                        row.habit.visits_this_week,
                        row.habit.typical_week_visits.normalize()
                    ),
                    detail: format!(
                        "A visit costs about {} this month.",
                        m(per_visit(&row.habit))
                    ),
                    weight: extra * per_visit(&row.habit),
                });
            }
            // The one store that drives the group most above its norm.
            let store = row
                .stores
                .iter()
                .filter_map(|s| above_norm(&s.habit).0.map(|over| (s, over)))
                .max_by_key(|(_, over)| *over);
            if let Some((store, over)) = store {
                out.push(Nudge {
                    title: format!("{}: {} above its usual month", store.category, m(over)),
                    detail: format!(
                        "{} in {} visits, in {name}.",
                        m(store.habit.spent),
                        store.habit.visits
                    ),
                    // Under the group's own line when both show.
                    weight: over / Decimal::TWO,
                });
            }
        }
        if adjustments.unsorted > Decimal::ZERO {
            out.push(Nudge {
                title: format!("{} is not sorted into a group", m(adjustments.unsorted)),
                detail: "Sorted spending counts against the right budget.".to_owned(),
                weight: adjustments.unsorted / Decimal::TWO,
            });
        }
    }

    if let Some(outlook) = report.savings_outlooks.iter().find(|o| o.currency == *unit) {
        let s = &outlook.signal;
        if s.accelerating || s.excessive_speed {
            out.push(Nudge {
                title: format!("The last three days cost {}", m(s.spending)),
                detail: format!(
                    "The three before cost {}; {} of it was unplanned.",
                    m(s.previous_spending),
                    m(s.unplanned)
                ),
                weight: (s.spending - s.previous_spending).max(s.unplanned),
            });
        }
    }

    out.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.title.cmp(&b.title)));
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
                    view! {
                        <li class="grid gap-0.5 border-t border-line py-2.5 first:border-t-0">
                            <strong class="text-sm font-semibold">{n.title}</strong>
                            <span class="text-sm text-fg-muted">{n.detail}</span>
                        </li>
                    }
                })
                .collect_view();
            view! { <ul class="mt-2 grid md:grid-cols-2 md:gap-x-12">{rows}</ul> }.into_any()
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
    fn a_group_over_its_limit_gets_the_daily_amount_that_holds_it() {
        assert_eq!(
            over_limit(d("200"), d("360"), d("300"), 10),
            Some((d("60"), d("10")))
        );
        assert_eq!(
            over_limit(d("320"), d("360"), d("300"), 10),
            Some((d("60"), d("0")))
        );
        assert_eq!(over_limit(d("200"), d("290"), d("300"), 10), None);
        assert_eq!(over_limit(d("200"), d("290"), d("0"), 10), None);
    }
}
