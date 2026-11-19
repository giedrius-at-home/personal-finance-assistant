//! Spending → Plan & patterns, "Goals": what the wishes and the emergency
//! fund need a month, what current habits leave, and where to spend less.
//! The emergency fund's settings are edited on the Wishes page.

use finance_types::gateway::WishSettings;
use finance_types::report::Currency;
use finance_types::report::wishes::{EmergencyFund, GoalOutlook, GoalStatus, HabitCut, WishesView};
use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::api::{NoArgs, load};
use crate::ds::{Badge, Card, CardHeader, CardTitle, Eyebrow, Size, Tone, Variant, button_class};
use crate::{Tab, format, state};

/// Whole units.
fn whole(value: Decimal, unit: &Currency) -> String {
    format::money_dp(value, &unit.code, 0)
}

/// `2031-06-01` → `Jun 2031`.
fn month_year(date: &str) -> String {
    let short = format::month_short_year(date);
    match (short.split(' ').next(), date.get(..4)) {
        (Some(month), Some(year)) => format!("{month} {year}"),
        _ => date.to_owned(),
    }
}

/// How many visits a month the cut is, at the group's usual ticket; `None`
/// when that is unknown or less than one.
pub fn fewer_visits(cut: &HabitCut) -> Option<u32> {
    // A place visited a few times a month is not counted in visits.
    let visits = cut.typical_visits.filter(|v| *v >= Decimal::from(4))?;
    if cut.typical <= Decimal::ZERO {
        return None;
    }
    let fewer = (cut.monthly * visits / cut.typical)
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    u32::try_from(fewer.mantissa()).ok().filter(|n| *n >= 1)
}

pub fn cut_text(cut: &HabitCut, unit: &Currency) -> String {
    let base = format!(
        "\u{2212}{} a month (usually {})",
        whole(cut.monthly, unit),
        whole(cut.typical, unit)
    );
    match fewer_visits(cut) {
        Some(1) => format!("{base} · about 1 fewer visit"),
        Some(n) => format!("{base} · about {n} fewer visits"),
        None => base,
    }
}

/// What happens to a wish when nothing changes, and whether that is bad.
pub fn goal_text(goal: &GoalStatus, unit: &Currency) -> (String, bool) {
    if goal.short > Decimal::ZERO {
        let mut text = format!("{} short at current habits", whole(goal.short, unit));
        if goal.plan_short > Decimal::ZERO {
            text.push_str(&format!(
                ", {} even on the budget",
                whole(goal.plan_short, unit)
            ));
        }
        (text, true)
    } else if goal.on_card > Decimal::ZERO {
        (
            format!("{} would go on the card", whole(goal.on_card, unit)),
            true,
        )
    } else {
        ("on track at current habits".to_owned(), false)
    }
}

pub fn headline(outlook: &GoalOutlook, unit: &Currency) -> String {
    let left = outlook.habit_free;
    if outlook.needed_monthly <= Decimal::ZERO {
        return format!(
            "No dated goal needs money in the coming months. Your habits leave {} a month.",
            whole(left, unit)
        );
    }
    let need = format!(
        "Your goals need {} a month.",
        whole(outlook.needed_monthly, unit)
    );
    if outlook.gap <= Decimal::ZERO {
        return format!(
            "{need} Your habits leave {}: they are covered.",
            whole(left, unit)
        );
    }
    let habits = if left < Decimal::ZERO {
        format!("Your habits overspend by {}.", whole(-left, unit))
    } else {
        format!("Your habits leave {}.", whole(left, unit))
    };
    format!("{need} {habits} Gap: {} a month.", whole(outlook.gap, unit))
}

pub fn fund_text(fund: &EmergencyFund, unit: &Currency) -> String {
    let holds = format!(
        "holds {} of {}",
        whole(fund.balance, unit),
        whole(fund.target, unit)
    );
    if fund.balance >= fund.target {
        return format!("full: {holds}");
    }
    match &fund.eta {
        Some(month) => format!("{holds} · full by {} at current habits", month_year(month)),
        None => format!("{holds} · nothing is left over to fill it at current habits"),
    }
}

/// A comma list of spending groups.
pub fn parse_groups(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The emergency fund's months: 0 (off) to the most the app stores.
pub fn parse_months(text: &str) -> Option<u32> {
    text.trim()
        .parse()
        .ok()
        .filter(|m| *m <= WishSettings::MAX_EMERGENCY_MONTHS)
}

fn card(view: &WishesView) -> Option<AnyView> {
    let outlook = view.goals.as_ref()?;
    let unit = &view.currency;
    let app = state();
    let coverage = outlook.coverage.map(|percent| {
        let tone = if percent >= 100 {
            Tone::Positive
        } else {
            Tone::Warning
        };
        view! { <Badge tone=tone>{format!("habits cover {percent}%")}</Badge> }
    });
    let goals = outlook
        .goals
        .iter()
        .map(|goal| {
            let (text, bad) = goal_text(goal, unit);
            let class = if bad { "text-sm text-warning-fg" } else { "text-sm text-fg-muted" };
            let must = goal.must.then_some(" · must");
            view! {
                <li class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 py-2">
                    <span>
                        <strong class="font-semibold">{goal.name.clone()}</strong>
                        <span class="text-sm tabular-nums text-fg-muted">
                            {format!(" {} · {}", whole(goal.amount, unit), month_year(&goal.due_on))}
                            {must}
                        </span>
                    </span>
                    <span class=class>{text}</span>
                </li>
            }
        })
        .collect_view();
    let fund = outlook.fund.as_ref().map(|fund| {
        view! {
            <li class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 py-2">
                <span>
                    <strong class="font-semibold">"Emergency fund"</strong>
                    <span class="text-sm tabular-nums text-fg-muted">
                        {format!(
                            " {} months × {} bare living",
                            fund.months,
                            whole(fund.fixed_monthly + fund.essentials_monthly, unit),
                        )}
                    </span>
                </span>
                <span class="text-sm tabular-nums text-fg-muted">{fund_text(fund, unit)}</span>
            </li>
        }
    });
    let cuts = (!outlook.cuts.is_empty()).then(|| {
        let rows = outlook
            .cuts
            .iter()
            .map(|cut| {
                view! {
                    <li class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 py-1.5">
                        <span>{cut.name.clone()}</span>
                        <span class="text-sm tabular-nums text-fg-muted">{cut_text(cut, unit)}</span>
                    </li>
                }
            })
            .collect_view();
        let open = (outlook.uncovered > Decimal::ZERO).then(|| {
            let text = format!(
                "Even that leaves {} a month open: move a date or lower a wish.",
                whole(outlook.uncovered, unit)
            );
            view! { <p class="mt-2 text-sm text-warning-fg">{text}</p> }
        });
        view! {
            <section class="mt-4" aria-label="Where to spend less">
                <Eyebrow>"TO CLOSE THE GAP, EVERY MONTH"</Eyebrow>
                <ul class="mt-1 divide-y divide-border">{rows}</ul>
                {open}
                <p class="mt-2 text-xs text-fg-muted">
                    "Each group gives the same share of its room: what it usually costs, down to any floor you set. Essentials you have not ranked give nothing. "
                    "Any other split of the same total works."
                </p>
            </section>
        }
    });
    let stuck = (outlook.cuts.is_empty() && outlook.uncovered > Decimal::ZERO).then(|| {
        let text = format!(
            "{} a month is open and nothing flexible is left to cut: move a date or lower a wish.",
            whole(outlook.uncovered, unit)
        );
        view! { <p class="mt-2 text-sm text-warning-fg">{text}</p> }
    });
    let lead = headline(outlook, unit);
    Some(
        view! {
            <section class="my-4 w-full min-w-0" aria-label="Goals against habits">
                <Card>
                    <CardHeader>
                        <div class="grid gap-1">
                            <Eyebrow>"WHAT YOUR GOALS ASK OF YOUR SPENDING"</Eyebrow>
                            <CardTitle>"Goals"</CardTitle>
                        </div>
                        {coverage}
                    </CardHeader>
                    <p class="tabular-nums">{lead}</p>
                    <ul class="mt-3 divide-y divide-border">{goals}{fund}</ul>
                    {cuts}
                    {stuck}
                    <div class="mt-3 flex flex-wrap items-center justify-between gap-2">
                        <p class="text-xs text-fg-muted">
                            "Current habits: every group costs what it usually does from next month on. "
                            "Bills, rent changes and new subscriptions planned as wishes are counted."
                        </p>
                        <button
                            type="button"
                            class=button_class(Variant::Link, Size::Inline)
                            on:click=move |_| app.go(Tab::Wishes)
                        >
                            "Edit wishes and the fund →"
                        </button>
                    </div>
                </Card>
            </section>
        }
        .into_any(),
    )
}

/// Spending → Plan & patterns: the goals against current habits.
#[component]
pub fn GoalsCard() -> impl IntoView {
    let revision = state().revision;
    let wishes = load::<WishesView, _>("wishes", move || {
        revision.track();
        NoArgs {}
    });
    move || {
        wishes
            .data
            .with(|data| data.as_ref().and_then(|r| r.as_ref().ok()).and_then(card))
    }
}

#[cfg(test)]
mod tests {
    use finance_types::report::wishes::HabitCut;
    use rust_decimal::Decimal;

    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn cut(monthly: &str, typical: &str, visits: Option<&str>) -> HabitCut {
        HabitCut {
            group_id: "food-out".into(),
            name: "Food & drinks out".into(),
            typical: dec(typical),
            monthly: dec(monthly),
            typical_visits: visits.map(dec),
        }
    }

    #[test]
    fn a_cut_reads_as_fewer_visits() {
        // €572 over 49 visits is about €11.70 a visit.
        assert_eq!(fewer_visits(&cut("416", "572", Some("49"))), Some(36));
        assert_eq!(fewer_visits(&cut("5", "572", Some("49"))), None);
        assert_eq!(fewer_visits(&cut("416", "572", None)), None);
        assert_eq!(fewer_visits(&cut("416", "0", Some("49"))), None);
        // Twice a month is not a habit of visiting.
        assert_eq!(fewer_visits(&cut("327", "450", Some("2"))), None);
    }

    #[test]
    fn essentials_parse_from_a_comma_list() {
        assert_eq!(
            parse_groups(" Groceries, Transport ,, Health & personal care "),
            ["Groceries", "Transport", "Health & personal care"]
        );
        assert!(parse_groups("  ").is_empty());
        assert_eq!(parse_months("6"), Some(6));
        assert_eq!(parse_months(" 0 "), Some(0));
        assert_eq!(parse_months("37"), None);
        assert_eq!(parse_months("six"), None);
    }
}
