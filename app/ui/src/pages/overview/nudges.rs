//! What could be better right now, as fields under the headline numbers:
//! every group over its limit or its own norm with what is left of it, how
//! many visits that still pays for (this month, a week, and next month), a
//! fast last three days, and unsorted spending. The advice holds spending
//! to the budget; it never asks for a bigger one.

use finance_types::report::adjustments::Habit;
use finance_types::report::events::{EventPhase, EventRow, Pace};
use finance_types::report::{Currency, Dashboard};
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

/// How often the owner went and usually goes, one count per fact.
fn visit_facts(h: &Habit) -> Vec<(String, String)> {
    let mut facts = vec![
        ("Visits this month".to_owned(), h.visits.to_string()),
        (
            "Visits this week".to_owned(),
            h.visits_this_week.to_string(),
        ),
    ];
    if let Some(month) = h.typical_visits {
        facts.push((
            "Usual visits a month".to_owned(),
            month.normalize().to_string(),
        ));
    }
    facts.push((
        "Usual visits a week".to_owned(),
        h.typical_week_visits.normalize().to_string(),
    ));
    facts
}

/// An allowance as money, and as visits when a visit has a price.
fn allowance_facts(
    label: &str,
    visits_label: &str,
    money: String,
    visits: Option<Decimal>,
) -> Vec<(String, String)> {
    let mut facts = vec![(label.to_owned(), money)];
    if let Some(visits) = visits {
        facts.push((visits_label.to_owned(), visits.normalize().to_string()));
    }
    facts
}

/// A running event as a field: what is left of it, how it is going, and
/// what a day may still cost.
pub fn event_nudge(event: &EventRow, unit: &Currency) -> Nudge {
    let m = |v: Decimal| money(v, unit);
    let over = event.left < Decimal::ZERO;
    let mut facts = vec![
        ("Planned".to_owned(), m(event.row.wish.amount)),
        ("Spent".to_owned(), m(event.spent)),
        ("Days left".to_owned(), event.days_left.to_string()),
    ];
    if event.left > Decimal::ZERO {
        facts.push(("A day, to stay inside".to_owned(), m(event.per_day)));
    }
    let chip = match event.pace {
        Pace::OnTrack => (Tone::Positive, "On track"),
        Pace::Fast => (Tone::Warning, "Spending fast"),
        Pace::Over => (Tone::Negative, "Over"),
    };
    Nudge {
        label: event.row.wish.name.clone(),
        value: if over {
            format!("{} over", m(-event.left))
        } else {
            format!("{} left", m(event.left))
        },
        bad: over,
        chip: Some((chip.0, chip.1.to_owned())),
        facts,
        weight: event.left.abs(),
    }
}

/// The running events first, then the [`MOST`] heaviest of the rest.
fn ordered(events: Vec<Nudge>, mut rest: Vec<Nudge>) -> Vec<Nudge> {
    rest.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label.cmp(&b.label)));
    rest.truncate(MOST);
    events.into_iter().chain(rest).collect()
}

/// The running events, then every nudge for the first currency, heaviest
/// first, at most [`MOST`].
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

    for adjustments in report.adjustments.iter().filter(|x| x.currency == *unit) {
        for row in &adjustments.rows {
            let name = &row.row.name;
            let h = &row.habit;
            let limit = (row.budgeted && row.row.planned > zero).then_some(row.row.planned);
            // Held to the limit; without one, to the owner's own usual month.
            let cap = limit.or(h.typical_spent.filter(|usual| *usual > zero));
            // The usual days handed to an event are not this group's to spend.
            let handed: Decimal = row.row.to_events.iter().map(|(_, n)| *n).sum();
            let cap = cap.map(|c| c - handed);
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
                facts.push((cap_label.to_owned(), m(cap + handed)));
            }
            for (event, amount) in &row.row.to_events {
                facts.push((format!("To {event}"), m(*amount)));
            }
            if let (Some(_), Some(expected)) = (heading, expected) {
                facts.push(("Month end at this pace".to_owned(), m(expected)));
            }
            facts.extend(visit_facts(h));
            if price > zero {
                facts.push(("A visit costs".to_owned(), m(price)));
            }
            if let Some(rest) = &rest {
                if let Some(visits) = rest.visits_left {
                    facts.push(("Visits left this month".to_owned(), visits.to_string()));
                }
                if rest.left > zero {
                    facts.extend(allowance_facts(
                        "A week, to stay inside",
                        "Visits a week, to stay inside",
                        m(rest.week),
                        rest.week_visits,
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
                    facts.extend(allowance_facts(
                        &next_month,
                        &format!("{next_month} visits"),
                        m(next_cap),
                        month.visits_left.map(Decimal::from),
                    ));
                } else {
                    facts.extend(allowance_facts(
                        &format!("{next_month}, a week"),
                        &format!("{next_month}, visits a week"),
                        m(week),
                        visits,
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
                facts.push((format!("{} spent", store.category), m(store.habit.spent)));
                facts.push((
                    format!("{} visits", store.category),
                    store.habit.visits.to_string(),
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

    let events = report
        .events
        .iter()
        .filter(|e| e.phase == EventPhase::Running && e.row.wish.currency == *unit)
        .map(|e| event_nudge(e, unit))
        .collect();
    ordered(events, out)
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
    fn visits_and_usual_visits_are_separate_facts() {
        let h = Habit {
            visits: 6,
            visits_this_week: 0,
            typical_visits: Some(d("5")),
            typical_week_visits: d("1.5"),
            ..Habit::default()
        };
        assert_eq!(
            visit_facts(&h),
            vec![
                ("Visits this month".to_owned(), "6".to_owned()),
                ("Visits this week".to_owned(), "0".to_owned()),
                ("Usual visits a month".to_owned(), "5".to_owned()),
                ("Usual visits a week".to_owned(), "1.5".to_owned()),
            ]
        );
        // No usual month yet: the weekly norm alone.
        let new = Habit {
            typical_visits: None,
            ..h
        };
        assert_eq!(visit_facts(&new).len(), 3);
    }

    #[test]
    fn an_allowance_is_money_and_then_visits_when_a_visit_has_a_price() {
        assert_eq!(
            allowance_facts(
                "A week, to stay inside",
                "Visits a week, to stay inside",
                "€40".into(),
                Some(d("2"))
            ),
            vec![
                ("A week, to stay inside".to_owned(), "€40".to_owned()),
                ("Visits a week, to stay inside".to_owned(), "2".to_owned()),
            ]
        );
        assert_eq!(allowance_facts("x", "y", "€40".into(), None).len(), 1);
    }

    fn running(left: &str, pace: Pace) -> EventRow {
        use finance_types::gateway::{Wish, WishKind, WishSource, WishStatus};
        use finance_types::report::wishes::WishRow;
        EventRow {
            row: WishRow {
                wish: Wish {
                    wish_id: "e".into(),
                    parent_id: None,
                    name: "Parents in NL".into(),
                    amount: d("400"),
                    currency: Currency::new("EUR"),
                    due_on: "2026-11-30".into(),
                    source: WishSource::Card,
                    saved: Decimal::ZERO,
                    saved_month: None,
                    status: WishStatus::Planned,
                    bought: None,
                    kind: WishKind::OneOff,
                    must: false,
                    event: None,
                    created_at: String::new(),
                },
                children: Vec::new(),
                rest: Decimal::ZERO,
                over_cap: false,
                monthly_set_aside: Decimal::ZERO,
                suggested_saved: Decimal::ZERO,
                short: Decimal::ZERO,
                tradeoffs: false,
                not_judged: None,
            },
            phase: EventPhase::Running,
            usual: d("150"),
            extra: d("250"),
            paid_ahead: Decimal::ZERO,
            spent: d("400") - d(left),
            left: d(left),
            days: 6,
            days_left: 3,
            per_day: (d(left) / d("3")).max(Decimal::ZERO).round_dp(2),
            pace,
            fits_up_to: None,
            months: Vec::new(),
            transactions: Vec::new(),
        }
    }

    #[test]
    fn a_running_event_is_a_field_with_what_is_left_and_a_day() {
        let unit = Currency::new("EUR");
        let n = event_nudge(&running("212.40", Pace::OnTrack), &unit);
        assert_eq!((n.label.as_str(), n.bad), ("Parents in NL", false));
        assert!(n.value.starts_with("€212.40"));
        assert_eq!(n.chip, Some((Tone::Positive, "On track".to_owned())));
        let labels: Vec<&str> = n.facts.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["Planned", "Spent", "Days left", "A day, to stay inside"]
        );
        assert_eq!(n.facts[3].1, "€70.80");
        // Over: the overrun, and no daily amount to stay inside.
        let n = event_nudge(&running("-20", Pace::Over), &unit);
        assert!(n.bad && n.value.starts_with("€20"));
        assert_eq!(n.facts.len(), 3);
    }

    #[test]
    fn the_running_events_come_before_the_six_heaviest() {
        let unit = Currency::new("EUR");
        let nudge = |weight: i64| Nudge {
            label: weight.to_string(),
            value: String::new(),
            bad: false,
            chip: None,
            facts: Vec::new(),
            weight: Decimal::from(weight),
        };
        let event = event_nudge(&running("10", Pace::Fast), &unit);
        let list = ordered(vec![event.clone()], (1..=8).map(nudge).collect());
        assert_eq!(list.len(), 7);
        assert_eq!(list[0], event);
        assert_eq!(list[1].label, "8");
        assert_eq!(list[6].label, "3");
    }

    #[test]
    fn next_month_is_its_own_limit_spread_over_the_weeks() {
        assert_eq!(month_week(d("100"), d("10")), (d("23.08"), Some(d("2.3"))));
        assert_eq!(month_week(d("100"), d("0")), (d("23.08"), None));
    }
}
