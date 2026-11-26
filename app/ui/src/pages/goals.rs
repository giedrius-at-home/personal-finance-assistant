//! Spending → Habits & goals, "Goals": what the wishes and the emergency
//! fund need a month, what current habits leave, and where to spend less.
//! The emergency fund's settings are edited on the Wishes page.

use finance_types::gateway::WishSettings;
use finance_types::report::Currency;
use finance_types::report::wishes::{GoalOutlook, GoalStatus, HabitCut, WishesView};
use leptos::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::api::{NoArgs, load};
use crate::ds::patterns::{HINT, PAGE_SECTION, TITLE_BLOCK};
use crate::ds::{
    Badge, Card, CardHeader, CardTitle, Eyebrow, FactsRow, Field, Input, METRIC_STRIP, Metric,
    Size, TABLE, TABLE_CONTAINER, TCELL, TH, TH_NUM, TROW, Tone, Variant, button_class, cn,
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

const OVERTIME_NOTE: &str = "Any whole number of extra hours, every week, at your plain hourly rate after the tax withheld on extra pay. A month is 52 weeks over 12.";

/// The outlook the card shows: as it stands, or with overtime worked.
pub struct Shown<'a> {
    pub habit_free: Decimal,
    pub plan_free: Decimal,
    pub gap: Decimal,
    pub coverage: Option<u32>,
    pub goals: &'a [GoalStatus],
    pub cuts: &'a [HabitCut],
    pub uncovered: Decimal,
    pub fund_eta: Option<&'a str>,
}

/// The outlook with `hours` of overtime a week; 0, or hours no scenario
/// was judged for, is the outlook as it stands.
pub fn shown(outlook: &GoalOutlook, hours: u32) -> Shown<'_> {
    let scenario = outlook
        .overtime
        .as_ref()
        .and_then(|o| o.scenarios.iter().find(|s| s.hours_week == hours));
    match scenario {
        Some(s) => Shown {
            habit_free: s.habit_free,
            plan_free: outlook.plan_free + s.extra_monthly,
            gap: s.gap,
            coverage: s.coverage,
            goals: &s.goals,
            cuts: &s.cuts,
            uncovered: s.uncovered,
            fund_eta: s.fund_eta.as_deref(),
        },
        None => Shown {
            habit_free: outlook.habit_free,
            plan_free: outlook.plan_free,
            gap: outlook.gap,
            coverage: outlook.coverage,
            goals: &outlook.goals,
            cuts: &outlook.cuts,
            uncovered: outlook.uncovered,
            fund_eta: outlook.fund.as_ref().and_then(|f| f.eta.as_deref()),
        },
    }
}

/// The typed hours a week, never more than `most`; anything else is none.
pub fn picked(hours: &str, most: u32) -> u32 {
    hours.trim().parse::<u32>().unwrap_or(0).min(most)
}

/// Whole months from `from` to `to` (`YYYY-MM…`), negative when `to` is
/// earlier.
fn months_apart(from: &str, to: &str) -> Option<i32> {
    let index = |date: &str| {
        let year: i32 = date.get(..4)?.parse().ok()?;
        let month: i32 = date.get(5..7)?.parse().ok()?;
        Some(year * 12 + month)
    };
    Some(index(to)? - index(from)?)
}

/// `3` → `3 months sooner`.
fn sooner(months: i32) -> String {
    match months {
        1 => "1 month sooner".to_owned(),
        n => format!("{n} months sooner"),
    }
}

/// What overtime does for one goal, in a few words: ready earlier, on
/// track where it was not, or less left to find. `None` when nothing.
pub fn faster(plain: &GoalStatus, with: &GoalStatus, unit: &Currency) -> Option<String> {
    let (_, before, was_bad) = goal_status(plain);
    let (_, after, is_bad) = goal_status(with);
    let earlier = match (&plain.ready_in, &with.ready_in) {
        (Some(from), Some(to)) => months_apart(to, from).filter(|m| *m > 0),
        _ => None,
    };
    if let Some(months) = earlier {
        Some(sooner(months))
    } else if was_bad && !is_bad {
        Some("Now on track".to_owned())
    } else {
        let less = before.unwrap_or_default() - after.unwrap_or_default();
        (was_bad && less > Decimal::ZERO).then(|| format!("{} less to find", whole(less, unit)))
    }
}

/// The hours field and what an extra hour pays.
fn overtime_bar(view: &WishesView, hours: RwSignal<String>) -> Option<AnyView> {
    let overtime = view.goals.as_ref()?.overtime.as_ref()?;
    let most = overtime.scenarios.last()?.hours_week;
    let unit = &view.currency;
    let rows = vec![
        (
            "An extra hour, net".to_owned(),
            format::money_dp(overtime.hourly_net, &unit.code, 2),
        ),
        (
            "Tax on it".to_owned(),
            format!("{}%", overtime.tax_percent.normalize()),
        ),
        ("Counts from".to_owned(), month_year(&overtime.from_month)),
        ("Most a week".to_owned(), most.to_string()),
    ];
    Some(
        view! {
            <section class="mt-8 grid items-end gap-x-12 gap-y-4 border-t border-line pt-6 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]" aria-label="Overtime">
                <Field label="Extra hours a week" id="goals-overtime" hint=OVERTIME_NOTE>
                    <Input id="goals-overtime" value=hours kind="number" min="0" max=most.to_string() step="1" placeholder="0" />
                </Field>
                <FactsRow rows=rows />
            </section>
        }
        .into_any(),
    )
}

/// The big comparison: everything `hours` of overtime a week changes.
fn overview(view: &WishesView, hours: u32) -> Option<AnyView> {
    let full = view.goals.as_ref()?;
    let overtime = full.overtime.as_ref()?;
    let scenario = overtime.scenarios.iter().find(|s| s.hours_week == hours)?;
    let unit = &view.currency;
    let dash = || "—".to_owned();
    let zero = Decimal::ZERO;
    let extra = scenario.extra_monthly;

    let hours_month = (Decimal::from(hours) * Decimal::from(52) / Decimal::from(12)).round_dp(0);
    let week = overtime
        .contract_hours_week
        .map(|contract| contract + Decimal::from(hours));
    let mut worked = vec![("Extra hours a month".to_owned(), hours_month.to_string())];
    if let Some(week) = week {
        worked.push(("Your week, hours".to_owned(), week.normalize().to_string()));
    }
    // The law's average week: fine to try, worth knowing.
    let heavy = week.is_some_and(|w| w > Decimal::from(48));
    let extra_chip = if heavy {
        (Tone::Warning, "Above the legal 48-hour week".to_owned())
    } else {
        (Tone::Neutral, "Within a 48-hour week".to_owned())
    };
    let salary = overtime.salary.map(|salary| {
        let now = vec![("Now".to_owned(), whole(salary, unit))];
        let value = whole(salary + extra, unit);
        view! { <Metric label="Salary with overtime" facts=now>{value}</Metric> }
    });
    let gap_chip = match scenario.coverage {
        Some(percent) if percent >= 100 => (Tone::Positive, "Covered".to_owned()),
        Some(percent) => (Tone::Warning, format!("{percent}% covered")),
        None => (Tone::Neutral, "Nothing due".to_owned()),
    };
    let gap_class = if scenario.gap > zero {
        "text-warning-fg"
    } else {
        "text-positive-fg"
    };
    let gap_facts = vec![
        ("Now".to_owned(), whole(full.gap.max(zero), unit)),
        (
            "Left after habits".to_owned(),
            whole(scenario.habit_free, unit),
        ),
    ];
    let fund_now = full.fund.as_ref().and_then(|f| f.eta.as_deref());
    let fund_then = scenario.fund_eta.as_deref();
    // The fund's change in words: full earlier, or reachable at all.
    let fund_change = match (fund_now, fund_then) {
        (Some(now), Some(then)) => months_apart(then, now).filter(|m| *m > 0).map(sooner),
        (None, Some(_)) => Some("Now reachable".to_owned()),
        _ => None,
    };
    let filling = full.fund.as_ref().filter(|f| f.balance < f.target);
    let fund_metric = filling.map(|_| {
        let value = fund_then.map_or_else(dash, month_year);
        let chip = match &fund_change {
            Some(text) => (Tone::Positive, text.clone()),
            None => (Tone::Neutral, "No change".to_owned()),
        };
        let facts = vec![("Now".to_owned(), fund_now.map_or_else(dash, month_year))];
        view! { <Metric label="Emergency fund full" chip=chip facts=facts>{value}</Metric> }
    });

    let num = cn([TCELL, "text-right tabular-nums whitespace-nowrap"]);
    let standing = |goal: &GoalStatus| {
        let (word, amount, bad) = goal_status(goal);
        let tone = if bad { Tone::Warning } else { Tone::Positive };
        view! {
            <span class="inline-flex items-center justify-end gap-2">
                {amount.map(|a| view! { <span class="tabular-nums">{whole(a, unit)}</span> })}
                <Badge tone=tone>{word}</Badge>
            </span>
        }
    };
    let gain = |text: Option<String>| match text {
        Some(text) => view! { <Badge tone=Tone::Positive>{text}</Badge> }.into_any(),
        None => view! { <span class="text-fg-muted">"No change"</span> }.into_any(),
    };
    let mut helped = usize::from(filling.is_some() && fund_change.is_some());
    let goals = scenario
        .goals
        .iter()
        .filter_map(|with| {
            let plain = full.goals.iter().find(|g| g.wish_id == with.wish_id)?;
            let change = faster(plain, with, unit);
            helped += usize::from(change.is_some());
            Some(view! {
                <tr class=TROW>
                    <th scope="row" class=cn([TCELL, "text-left font-medium"])>{with.name.clone()}</th>
                    <td class=num.clone()>{month_year(&with.due_on)}</td>
                    <td class=cn([TCELL, "text-right"])>{standing(plain)}</td>
                    <td class=cn([TCELL, "text-right"])>{standing(with)}</td>
                    <td class=num.clone()>{plain.ready_in.as_deref().map_or_else(dash, month_year)}</td>
                    <td class=num.clone()>{with.ready_in.as_deref().map_or_else(dash, month_year)}</td>
                    <td class=cn([TCELL, "text-right"])>{gain(change)}</td>
                </tr>
            })
        })
        .collect_view();
    let fund_row = filling.map(|_| {
        view! {
            <tr class=TROW>
                <th scope="row" class=cn([TCELL, "text-left font-medium"])>"Emergency fund"</th>
                <td class=num.clone()>{dash()}</td>
                <td class=cn([TCELL, "text-right"])><Badge tone=Tone::Neutral>"Filling"</Badge></td>
                <td class=cn([TCELL, "text-right"])><Badge tone=Tone::Neutral>"Filling"</Badge></td>
                <td class=num.clone()>{fund_now.map_or_else(dash, month_year)}</td>
                <td class=num.clone()>{fund_then.map_or_else(dash, month_year)}</td>
                <td class=cn([TCELL, "text-right"])>{gain(fund_change.clone())}</td>
            </tr>
        }
    });
    let faster_chip = match helped {
        0 => (Tone::Neutral, "Nothing changes yet".to_owned()),
        1 => (Tone::Positive, "1 goal helped".to_owned()),
        n => (Tone::Positive, format!("{n} goals helped")),
    };
    let extra_text = format!("+{}", whole(extra, unit));
    let gap_text = whole(scenario.gap.max(zero), unit);
    let title = match hours {
        1 => "With 1 extra hour a week".to_owned(),
        n => format!("With {n} extra hours a week"),
    };
    Some(
        view! {
            <section class="mt-8 rounded-lg border border-border bg-card-muted p-5 sm:p-6" aria-label="With overtime">
                <div class="flex flex-wrap items-center justify-between gap-3">
                    <div class=TITLE_BLOCK>
                        <Eyebrow>"WHAT GOES FASTER"</Eyebrow>
                        <CardTitle>{title}</CardTitle>
                    </div>
                    <Badge tone=faster_chip.0>{faster_chip.1}</Badge>
                </div>
                <div class=cn([METRIC_STRIP, "mt-5 md:grid-cols-4"])>
                    <Metric label="Extra a month, net" chip=extra_chip facts=worked>
                        {extra_text}
                    </Metric>
                    {salary}
                    <Metric label="Gap a month" chip=gap_chip facts=gap_facts>
                        <span class=gap_class>{gap_text}</span>
                    </Metric>
                    {fund_metric}
                </div>
                <div class=cn([TABLE_CONTAINER, "mt-6"])>
                    <table class=TABLE>
                        <thead>
                            <tr>
                                <th class=cn([TH, "text-left"])>"Goal"</th>
                                <th class=TH_NUM>"Due"</th>
                                <th class=TH_NUM>"Now"</th>
                                <th class=TH_NUM>"With overtime"</th>
                                <th class=TH_NUM>"Paid for now"</th>
                                <th class=TH_NUM>"Paid for then"</th>
                                <th class=TH_NUM>"What changes"</th>
                            </tr>
                        </thead>
                        <tbody>{goals}{fund_row}</tbody>
                    </table>
                </div>
            </section>
        }
        .into_any(),
    )
}

/// The figures and tables at current habits.
fn body(view: &WishesView) -> Option<AnyView> {
    let full = view.goals.as_ref()?;
    let outlook = shown(full, 0);
    let unit = &view.currency;
    let zero = Decimal::ZERO;
    let dash = || "—".to_owned();

    // The headline as fields: need, what habits leave, the gap, what stays open.
    let gap_chip = match outlook.coverage {
        Some(percent) if percent >= 100 => (Tone::Positive, "Covered".to_owned()),
        Some(percent) => (Tone::Warning, format!("{percent}% covered")),
        None => (Tone::Neutral, "Nothing due".to_owned()),
    };
    let need = whole(full.needed_monthly.max(zero), unit);
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
    let fund = full.fund.as_ref().map(|fund| {
        let held = fund.balance >= fund.target;
        let (tone, word) = if held {
            (Tone::Positive, "Full")
        } else if outlook.fund_eta.is_some() {
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
                <td class=num.clone()>{outlook.fund_eta.map_or_else(dash, month_year)}</td>
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
        }
        .into_any(),
    )
}

/// The card: the goals at current habits, the hours field, then what the
/// typed hours change. Only that last part redraws while typing.
fn card(view: &WishesView, hours: RwSignal<String>) -> Option<AnyView> {
    let outlook = view.goals.as_ref()?;
    let app = state();
    let most = outlook
        .overtime
        .as_ref()
        .and_then(|o| o.scenarios.last())
        .map_or(0, |s| s.hours_week);
    let setup = outlook.overtime.is_none().then(|| {
        view! {
            <p class=cn([HINT, "mt-6"])>
                "To see what overtime would change, enter your hourly rate and the tax on extra pay under Manage, Settings."
            </p>
        }
    });
    let figures = body(view);
    let bar = overtime_bar(view, hours);
    let compared = {
        let view = view.clone();
        move || overview(&view, picked(&hours.get(), most))
    };
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
                    {figures}
                    {bar}
                    {compared}
                    {setup}
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
    // Extra hours a week, as typed.
    let hours = RwSignal::new(String::new());
    let wishes = load::<WishesView, _>("wishes", move || {
        revision.track();
        NoArgs {}
    });
    move || {
        wishes.data.with(|data| {
            data.as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|view| card(view, hours))
        })
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
    fn the_picked_hours_choose_the_outlook_shown() {
        use finance_types::report::wishes::{OvertimeOutlook, OvertimeScenario};
        let scenario = |hours_week, extra: &str, gap: &str| OvertimeScenario {
            hours_week,
            extra_monthly: dec(extra),
            habit_free: dec("30") + dec(extra),
            gap: dec(gap),
            coverage: Some(50),
            goals: Vec::new(),
            cuts: Vec::new(),
            uncovered: dec("0"),
            fund_eta: Some("2027-03-01".into()),
        };
        let outlook = GoalOutlook {
            needed_monthly: dec("450"),
            habit_free: dec("30"),
            plan_free: dec("500"),
            gap: dec("420"),
            coverage: Some(6),
            horizon_months: 3,
            goals: Vec::new(),
            cuts: Vec::new(),
            uncovered: dec("0"),
            fund: None,
            overtime: Some(OvertimeOutlook {
                hourly_gross: dec("23.59"),
                tax_percent: dec("50.47"),
                hourly_net: dec("11.68"),
                contract_hours_week: Some(dec("36")),
                salary: Some(dec("3127.27")),
                from_month: "2027-01-01".into(),
                scenarios: vec![
                    scenario(1, "50.63", "369.37"),
                    scenario(2, "101.26", "318.74"),
                ],
            }),
        };
        let plain = shown(&outlook, 0);
        assert_eq!((plain.habit_free, plain.gap), (dec("30"), dec("420")));
        assert_eq!(plain.fund_eta, None);
        let two = shown(&outlook, 2);
        assert_eq!((two.habit_free, two.gap), (dec("131.26"), dec("318.74")));
        assert_eq!(two.plan_free, dec("601.26"));
        assert_eq!(two.fund_eta, Some("2027-03-01"));
        // Hours nothing was judged for show the outlook as it stands.
        assert_eq!(shown(&outlook, 9).gap, dec("420"));
        assert_eq!(picked(" 4 ", 132), 4);
        assert_eq!(picked("", 132), 0);
        assert_eq!(picked("-3", 132), 0);
        // More than the week holds is the most it holds.
        assert_eq!(picked("500", 132), 132);
    }

    #[test]
    fn overtime_says_what_it_does_for_a_goal() {
        let goal = |short: &str, ready: Option<&str>| GoalStatus {
            wish_id: "trip".into(),
            name: "Trip".into(),
            due_on: "2027-06-20".into(),
            amount: dec("900"),
            must: false,
            short: dec(short),
            card_short: dec("0"),
            on_card: dec("0"),
            plan_short: dec("0"),
            ready_in: ready.map(str::to_owned),
        };
        let eur = Currency::eur();
        let late = goal("0", Some("2027-06-01"));
        assert_eq!(
            faster(&late, &goal("0", Some("2027-03-01")), &eur).as_deref(),
            Some("3 months sooner")
        );
        assert_eq!(
            faster(&late, &goal("0", Some("2027-05-01")), &eur).as_deref(),
            Some("1 month sooner")
        );
        assert_eq!(faster(&late, &late, &eur), None);
        let short = goal("400", None);
        assert_eq!(
            faster(&short, &goal("0", Some("2027-06-01")), &eur).as_deref(),
            Some("Now on track")
        );
        assert_eq!(
            faster(&short, &goal("150", None), &eur).as_deref(),
            Some("€250 less to find")
        );
        assert_eq!(faster(&short, &short, &eur), None);
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
