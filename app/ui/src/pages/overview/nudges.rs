//! What to do right now, as tiles under the headline numbers: the money to
//! move between accounts, every group over its limit or its own norm, a
//! fast last three days, and unsorted spending. Each tile says the one
//! thing to do, the amount it is about and at most two numbers to steer
//! by; the visit counts sit behind "More". The advice holds spending to
//! the budget; it never asks for a bigger one.

use std::collections::HashMap;

use finance_types::report::adjustments::Habit;
use finance_types::report::closure::MoneyPlan;
use finance_types::report::events::{EventPhase, EventRow, Pace};
use finance_types::report::{Currency, Dashboard};
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ds::{
    Badge, Card, CardTitle, EmptyState, Eyebrow, Facts, Heading, Icon, Lucide, Size, Tone, Variant,
    button_class, cn, money,
};
use crate::pages::with_report;
use crate::{SpendingSection, state};

use super::{month_name, remaining_days};
use crate::ds::patterns::{CARD_HEADER, Disclosure, EMPTY_COMPACT, FIGURE_LG};

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nudge {
    pub label: String,
    /// The one thing to do, in two or three words.
    pub action: String,
    /// At most two numbers to steer by, shown under the action.
    pub key: Vec<(String, String)>,
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
    let (chip, action) = match event.pace {
        Pace::OnTrack => ((Tone::Positive, "On track"), "Keep going"),
        Pace::Fast => ((Tone::Warning, "Spending fast"), "Slow down"),
        Pace::Over => ((Tone::Negative, "Over"), "Stop spending"),
    };
    let mut key = Vec::new();
    if event.left > Decimal::ZERO {
        key.push(("A day".to_owned(), m(event.per_day)));
    }
    key.push(("Days left".to_owned(), event.days_left.to_string()));
    Nudge {
        label: event.row.wish.name.clone(),
        action: action.to_owned(),
        key,
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

const STOP: &str = "Stop for this month";

/// Two or more groups over their limit read as one tile, not the same
/// command again and again: each group and its overrun inside it.
fn merge_stops(rest: Vec<Nudge>) -> Vec<Nudge> {
    let (stops, mut rest): (Vec<Nudge>, Vec<Nudge>) =
        rest.into_iter().partition(|n| n.action == STOP);
    if stops.len() < 2 {
        rest.extend(stops);
        return rest;
    }
    let mut stops = stops;
    stops.sort_by_key(|a| std::cmp::Reverse(a.weight));
    let lines: Vec<(String, String)> = stops
        .iter()
        .map(|n| (n.label.clone(), n.value.clone()))
        .collect();
    rest.push(Nudge {
        label: format!("Over budget in {} groups", stops.len()),
        action: STOP.to_owned(),
        key: lines.iter().take(2).cloned().collect(),
        value: format!("{} groups", stops.len()),
        bad: true,
        chip: Some((Tone::Negative, "Over".to_owned())),
        facts: lines.into_iter().skip(2).collect(),
        weight: stops.iter().map(|n| n.weight).sum(),
    });
    rest
}

/// The running events first, then the [`MOST`] heaviest of the rest.
fn ordered(events: Vec<Nudge>, rest: Vec<Nudge>) -> Vec<Nudge> {
    let mut rest = merge_stops(rest);
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
            let (value, bad, chip, action) = match (&rest, over) {
                (Some(_), Some(over)) => (
                    format!("{} over", m(over)),
                    true,
                    (Tone::Negative, format!("Over {}", cap_label.to_lowercase())),
                    STOP,
                ),
                (Some(rest), None) => (
                    format!("{} left", m(rest.left)),
                    false,
                    if heading.is_some() {
                        (Tone::Warning, "Heading over".to_owned())
                    } else {
                        (Tone::Warning, "More visits".to_owned())
                    },
                    if heading.is_some() {
                        "Slow down"
                    } else {
                        "Fewer visits this week"
                    },
                ),
                (None, _) => (
                    m(h.spent),
                    false,
                    (Tone::Warning, "More visits".to_owned()),
                    "Fewer visits this week",
                ),
            };
            // What to steer by: the rest of this month while some is left,
            // else next month's allowance.
            let mut key = Vec::new();
            if let Some(rest) = rest.as_ref().filter(|r| r.left > zero) {
                key = allowance_facts("A week", "Visits a week", m(rest.week), rest.week_visits);
            }

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
                let next = if visits.is_some_and(|v| v < Decimal::ONE) {
                    let month = allowance(next_cap, zero, price, 31);
                    allowance_facts(
                        &next_month,
                        &format!("{next_month} visits"),
                        m(next_cap),
                        month.visits_left.map(Decimal::from),
                    )
                } else {
                    allowance_facts(
                        &format!("{next_month}, a week"),
                        &format!("{next_month}, visits a week"),
                        m(week),
                        visits,
                    )
                };
                if key.is_empty() {
                    key = next.clone();
                }
                facts.extend(next);
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
                action: action.to_owned(),
                key,
                value,
                bad,
                chip: Some(chip),
                facts,
                weight,
            });
        }
        if adjustments.unsorted > zero {
            out.push(Nudge {
                label: "Not in any group".to_owned(),
                action: "Sort it".to_owned(),
                key: Vec::new(),
                value: m(adjustments.unsorted),
                bad: false,
                chip: None,
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
                action: "Slow down".to_owned(),
                key: vec![("Three days before".to_owned(), m(s.previous_spending))],
                value: m(s.spending),
                bad: false,
                chip: Some((Tone::Warning, "Faster".to_owned())),
                facts: vec![("Of it unplanned".to_owned(), m(s.unplanned))],
                weight: (s.spending - s.previous_spending).max(s.unplanned),
            });
        }
    }

    let events: Vec<Nudge> = report
        .events
        .iter()
        .filter(|e| e.phase == EventPhase::Running && e.row.wish.currency == *unit)
        .map(|e| event_nudge(e, unit))
        .collect();
    let moves = report
        .money_plan
        .as_ref()
        .filter(|p| p.currency == *unit)
        .and_then(move_nudge);
    ordered(moves.into_iter().chain(events).collect(), out)
}

/// The transfers the money plan asks for, as one tile: the total to move
/// and every transfer behind "More".
pub fn move_nudge(plan: &MoneyPlan) -> Option<Nudge> {
    let total: Decimal = plan.moves.iter().map(|m| m.amount).sum();
    if total <= Decimal::ZERO {
        return None;
    }
    let names: HashMap<String, String> = plan
        .accounts
        .iter()
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect();
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let facts: Vec<(String, String)> = plan
        .moves
        .iter()
        .map(|m| {
            (
                format!("{} → {}", name(&m.from_account_id), name(&m.to_account_id)),
                money(m.amount, &plan.currency),
            )
        })
        .collect();
    Some(Nudge {
        label: "Between accounts".to_owned(),
        action: "Move money".to_owned(),
        key: facts.iter().take(2).cloned().collect(),
        value: money(total, &plan.currency),
        bad: false,
        chip: None,
        facts: facts.into_iter().skip(2).collect(),
        weight: total,
    })
}

#[component]
pub fn OverviewNudges() -> impl IntoView {
    with_report(|report| {
        report.monthly_allocation.as_ref()?;
        let app = state();
        let list = nudges(report);
        let body = if list.is_empty() {
            view! {
                <EmptyState class=EMPTY_COMPACT icon=Icon::CircleCheck title="Nothing to do. Spending is inside your limits." />
            }
            .into_any()
        } else {
            let tiles = list.into_iter().map(tile).collect_view();
            view! { <ul class="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-3">{tiles}</ul> }
                .into_any()
        };
        Some(view! {
            <Card>
                <header class=CARD_HEADER>
                    <div>
                        <Eyebrow level=Heading::P>"TODAY"</Eyebrow>
                        <CardTitle level=Heading::H2 class="mt-1 text-xl">"Do this now"</CardTitle>
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

/// One thing to do, as a box: where, what, how much, and two numbers to
/// steer by; everything else behind "More".
fn tile(n: Nudge) -> impl IntoView {
    let more = (!n.facts.is_empty()).then(|| {
        view! {
            <Disclosure class="mt-1 text-xs" summary_class="text-fg-muted" summary="More">
                <Facts rows=n.facts class="mt-2" />
            </Disclosure>
        }
    });
    view! {
        <li class="grid min-w-0 content-start gap-2 rounded-lg border border-border bg-card-strong p-4">
            <span class="truncate text-eyebrow font-bold uppercase tracking-eyebrow text-fg-muted">{n.label}</span>
            <strong class="text-lg font-semibold leading-tight tracking-tight">{n.action}</strong>
            <div class="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                <span class=cn([FIGURE_LG, if n.bad { "text-over-fg" } else { "" }])>{n.value}</span>
                {n.chip.map(|(tone, text)| view! { <Badge tone=tone>{text}</Badge> })}
            </div>
            <Facts rows=n.key />
            {more}
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_over_their_limit_merge_into_one_tile() {
        let stop = |label: &str, weight: u32| Nudge {
            label: label.into(),
            action: STOP.into(),
            value: format!("€{weight} over"),
            weight: Decimal::from(weight),
            ..Nudge::default()
        };
        let other = Nudge {
            label: "Last three days".into(),
            action: "Slow down".into(),
            ..Nudge::default()
        };
        let out = merge_stops(vec![
            stop("Fun", 50),
            other.clone(),
            stop("Food", 200),
            stop("Gifts", 10),
        ]);
        assert_eq!(out.len(), 2);
        let merged = &out[1];
        assert_eq!(merged.label, "Over budget in 3 groups");
        assert_eq!(merged.key[0], ("Food".to_owned(), "€200 over".to_owned()));
        assert_eq!(merged.facts, [("Gifts".to_owned(), "€10 over".to_owned())]);
        assert_eq!(merged.weight, Decimal::from(260));
        // One group alone keeps its own tile.
        assert_eq!(merge_stops(vec![stop("Fun", 50)])[0].label, "Fun");
    }

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
            weight: Decimal::from(weight),
            ..Nudge::default()
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
