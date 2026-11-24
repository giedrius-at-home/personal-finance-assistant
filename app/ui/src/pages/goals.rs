//! Spending → Habits & goals, "Goals": what the wishes and the emergency
//! fund need a month, what current habits leave, and where to spend less.
//! The emergency fund's settings are edited on the Wishes page.

use finance_types::gateway::WishSettings;
use finance_types::report::Currency;
use finance_types::report::wishes::{GoalStatus, HabitCut, WishesView};
use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::api::{NoArgs, load};
use crate::ds::patterns::{PAGE_SECTION, TITLE_BLOCK};
use crate::ds::{
    Badge, Card, CardHeader, CardTitle, Eyebrow, METRIC_STRIP, Metric, Size, TABLE,
    TABLE_CONTAINER, TCELL, TH, TH_NUM, TROW, Tone, Variant, button_class, cn,
};
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

/// How many visits a month the cut is, at what a visit usually costs;
/// `None` when that is unknown, the place is visited only a few times a
/// month, or the cut is less than one visit.
pub fn fewer_visits(cut: &HabitCut) -> Option<u32> {
    cut.typical_visits.filter(|v| *v >= Decimal::from(4))?;
    let ticket = cut.ticket.filter(|t| *t > Decimal::ZERO)?;
    let fewer =
        (cut.monthly / ticket).round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    u32::try_from(fewer.mantissa()).ok().filter(|n| *n >= 1)
}

/// A wish when nothing changes: the status word, the amount behind it and
/// whether that is bad.
pub fn goal_status(goal: &GoalStatus) -> (&'static str, Option<Decimal>, bool) {
    if goal.plan_short > Decimal::ZERO {
        // The same verdict as Wishes: short with budgets kept.
        ("Short", Some(goal.plan_short), true)
    } else if goal.short > Decimal::ZERO {
        // Fine on budget, short only if spending stays as usual.
        ("Needs cuts", Some(goal.short), true)
    } else if goal.card_short > Decimal::ZERO {
        // Paid by card: the card's charge the month after is what lacks.
        ("Card bill short", Some(goal.card_short), true)
    } else if goal.on_card > Decimal::ZERO {
        ("On the card", Some(goal.on_card), true)
    } else {
        ("On track", None, false)
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

const HABITS_NOTE: &str = "Current habits: every group costs what it usually does from next month on. \"Usually\" counts months without spending. The need falls as wishes are paid. Bills, rent changes and new subscriptions planned as wishes are counted.";
const CUTS_NOTE: &str = "Each group gives the same share of its room: what it usually costs, down to any floor you set. Essentials you have not ranked give nothing. Any other split of the same total works.";

fn card(view: &WishesView) -> Option<AnyView> {
    let outlook = view.goals.as_ref()?;
    let unit = &view.currency;
    let app = state();
    let zero = Decimal::ZERO;
    let dash = || "—".to_owned();

    // The headline as fields: need, what habits leave, the gap, what stays open.
    let gap_chip = match outlook.coverage {
        Some(percent) if percent >= 100 => (Tone::Positive, "Covered".to_owned()),
        Some(percent) => (Tone::Warning, format!("{percent}% covered")),
        None => (Tone::Neutral, "Nothing due".to_owned()),
    };
    let need = whole(outlook.needed_monthly.max(zero), unit);
    let left = whole(outlook.habit_free, unit);
    let left_class = if outlook.habit_free < zero {
        "text-over-fg"
    } else {
        ""
    };
    let gap = whole(outlook.gap.max(zero), unit);
    let gap_class = if outlook.gap > zero {
        "text-warning-fg"
    } else {
        "text-positive-fg"
    };
    let budget_left = vec![("On budget".to_owned(), whole(outlook.plan_free, unit))];
    let open = (outlook.uncovered > zero).then(|| {
        let value = whole(outlook.uncovered, unit);
        view! {
            <div title="Cutting every flexible group to its floor still leaves this: move a date or lower a wish.">
                <Metric label="Still open a month" chip=(Tone::Warning, "Move a date".to_owned())>
                    <span class="text-warning-fg">{value}</span>
                </Metric>
            </div>
        }
    });

    let num = cn([TCELL, "text-right tabular-nums whitespace-nowrap"]);
    let goals = outlook
        .goals
        .iter()
        .map(|goal| {
            let (word, amount, bad) = goal_status(goal);
            let tone = if bad { Tone::Warning } else { Tone::Positive };
            view! {
                <tr class=TROW>
                    <th scope="row" class=cn([TCELL, "text-left font-medium"])>
                        {goal.name.clone()}
                        {goal.must.then(|| view! { <Badge tone=Tone::Outline class="ml-2">"Must"</Badge> })}
                    </th>
                    <td class=num.clone()>{whole(goal.amount, unit)}</td>
                    <td class=num.clone()>{month_year(&goal.due_on)}</td>
                    <td class=num.clone()>{amount.map_or_else(dash, |a| whole(a, unit))}</td>
                    <td class=cn([TCELL, "text-right"])><Badge tone=tone>{word}</Badge></td>
                </tr>
            }
        })
        .collect_view();
    let fund = outlook.fund.as_ref().map(|fund| {
        let full = fund.balance >= fund.target;
        let (tone, word) = if full {
            (Tone::Positive, "Full")
        } else if fund.eta.is_some() {
            (Tone::Neutral, "Filling")
        } else {
            (Tone::Warning, "Nothing left over")
        };
        let bare = whole(fund.fixed_monthly + fund.essentials_monthly, unit);
        view! {
            <tr class=TROW>
                <th scope="row" class=cn([TCELL, "text-left font-medium"])>
                    "Emergency fund"
                    <small class="ml-2 font-normal tabular-nums text-fg-muted">{format!("{} × {bare}", fund.months)}</small>
                </th>
                <td class=num.clone()>{whole(fund.target, unit)}</td>
                <td class=num.clone()>{fund.eta.as_deref().map_or_else(dash, month_year)}</td>
                <td class=num.clone()>{whole((fund.target - fund.balance).max(zero), unit)}</td>
                <td class=num.clone()>{dash()}</td>
                <td class=cn([TCELL, "text-right"])><Badge tone=tone>{word}</Badge></td>
            </tr>
        }
    });
    let cuts = (!outlook.cuts.is_empty()).then(|| {
        let rows = outlook
            .cuts
            .iter()
            .map(|cut| {
                view! {
                    <tr class=TROW>
                        <th scope="row" class=cn([TCELL, "text-left font-medium"])>{cut.name.clone()}</th>
                        <td class=num.clone()>{whole(cut.typical, unit)}</td>
                        <td class=num.clone()>{format!("\u{2212}{}", whole(cut.monthly, unit))}</td>
                        <td class=num.clone()>{fewer_visits(cut).map_or_else(dash, |n| n.to_string())}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <section class="min-w-0" aria-label="Where to spend less">
                <Eyebrow><span title=CUTS_NOTE>"TO CLOSE THE GAP, A MONTH"</span></Eyebrow>
                <div class=cn([TABLE_CONTAINER, "mt-3"])>
                    <table class=TABLE>
                        <thead>
                            <tr>
                                <th class=cn([TH, "text-left"])>"Group"</th>
                                <th class=TH_NUM>"Usually"</th>
                                <th class=TH_NUM>"Spend less"</th>
                                <th class=TH_NUM>"Fewer visits"</th>
                            </tr>
                        </thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            </section>
        }
    });
    Some(
        view! {
            <section class=PAGE_SECTION aria-label="Goals against habits">
                <Card>
                    <CardHeader>
                        <div class=TITLE_BLOCK>
                            <Eyebrow><span title=HABITS_NOTE>"WHAT YOUR GOALS ASK OF YOUR SPENDING"</span></Eyebrow>
                            <CardTitle>"Goals"</CardTitle>
                        </div>
                        <button
                            type="button"
                            class=button_class(Variant::Ghost, Size::Sm)
                            on:click=move |_| app.go(Tab::Wishes)
                        >
                            "Edit wishes and the fund →"
                        </button>
                    </CardHeader>
                    <div class=cn([METRIC_STRIP, "md:grid-cols-4"])>
                        <Metric label="Goals need a month">{need}</Metric>
                        <Metric label="Habits leave a month" facts=budget_left>
                            <span class=left_class>{left}</span>
                        </Metric>
                        <Metric label="Gap a month" chip=gap_chip>
                            <span class=gap_class>{gap}</span>
                        </Metric>
                        {open}
                    </div>
                    <div class="mt-8 grid gap-x-12 gap-y-8 xl:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
                        <section class="min-w-0" aria-label="Goals at current habits">
                            <Eyebrow>"AT CURRENT HABITS"</Eyebrow>
                            <div class=cn([TABLE_CONTAINER, "mt-3"])>
                                <table class=TABLE>
                                    <thead>
                                        <tr>
                                            <th class=cn([TH, "text-left"])>"Goal"</th>
                                            <th class=TH_NUM>"Amount"</th>
                                            <th class=TH_NUM>"Due"</th>
                                            <th class=TH_NUM>"Gap"</th>
                                            <th class=TH_NUM>"Status"</th>
                                        </tr>
                                    </thead>
                                    <tbody>{goals}{fund}</tbody>
                                </table>
                            </div>
                        </section>
                        {cuts}
                    </div>
                </Card>
            </section>
        }
        .into_any(),
    )
}

/// Spending → Habits & goals: the goals against current habits.
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

    fn cut(monthly: &str, visits: Option<&str>, ticket: Option<&str>) -> HabitCut {
        HabitCut {
            group_id: "food-out".into(),
            name: "Food & drinks out".into(),
            typical: dec("572"),
            monthly: dec(monthly),
            typical_visits: visits.map(dec),
            ticket: ticket.map(dec),
        }
    }

    #[test]
    fn a_cut_reads_as_fewer_visits() {
        assert_eq!(
            fewer_visits(&cut("416", Some("49"), Some("11.70"))),
            Some(36)
        );
        assert_eq!(fewer_visits(&cut("5", Some("49"), Some("11.70"))), None);
        assert_eq!(fewer_visits(&cut("416", None, Some("11.70"))), None);
        assert_eq!(fewer_visits(&cut("416", Some("49"), None)), None);
        assert_eq!(fewer_visits(&cut("416", Some("49"), Some("0"))), None);
        // Twice a month is not a habit of visiting.
        assert_eq!(fewer_visits(&cut("327", Some("2"), Some("225"))), None);
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
