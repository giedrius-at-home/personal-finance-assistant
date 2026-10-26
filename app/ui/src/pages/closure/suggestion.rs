//! The live savings suggestion beside the split (personal-site
//! `components/month-closure/suggestion.tsx`).

use finance_types::report::Currency;
use finance_types::report::closure::SavingsSuggestion;
use finance_types::report::outlook::ForecastExclusions;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::ui::{ACTIONS, CALLOUT, SMALL};
use crate::ds::{Button, Size, Variant, cn};

const BREAKDOWN: &str = "grid gap-1 text-xs";
const BREAKDOWN_ROW: &str = "flex justify-between gap-3 [&>:last-child]:shrink-0 [&>:last-child]:text-right [&>:last-child]:tabular-nums";
const TOTAL_ROW: &str = "flex justify-between gap-3 [&>:last-child]:shrink-0 [&>:last-child]:text-right [&>:last-child]:tabular-nums mt-1 border-t border-line pt-2 text-sm";

/// The suggestion is computed for EUR accounts only.
fn eur(value: Decimal) -> String {
    crate::ds::money(value, &Currency::eur())
}

/// One line of the split as the suggestion sees it (EUR lines only count).
#[derive(Clone, Debug)]
pub struct SuggestionLine {
    pub destination: String,
    pub amount: Decimal,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Figures {
    /// Overview's projected available, as of the suggestion's date.
    pub projected: Decimal,
    pub everyday: Decimal,
    /// One-off payments due in the three months after the current one.
    pub known_future: Decimal,
    pub safety_buffer: Decimal,
    pub takes: Decimal,
    /// Of `takes`, what this split already takes from general savings.
    pub general_takes: Decimal,
    /// Positive lines to pots other than general, plan lines and buffer lines.
    pub assigned: Decimal,
    /// projected − everyday − known future payments − safety buffer − assigned + takes.
    pub suggested: Decimal,
    /// Already assigned to the general pot in this split.
    pub general: Decimal,
    /// General savings balance before this month's split.
    pub general_balance: Decimal,
}

/// What is available now less what is still expected this month, known
/// future payments, the safety buffer and this split's own lines.
pub fn figures(inputs: &SavingsSuggestion, lines: &[SuggestionLine]) -> Figures {
    let general_destination = format!("pot:{}", inputs.general_pot_id);
    let (mut takes, mut general_takes, mut assigned, mut general) =
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for line in lines.iter().filter(|l| l.currency == "EUR") {
        let pot = line.destination.starts_with("pot:");
        if pot && line.amount < Decimal::ZERO {
            takes += line.amount.abs();
            if line.destination == general_destination {
                general_takes += line.amount.abs();
            }
        } else if line.destination == general_destination {
            general += line.amount;
        } else if line.amount <= Decimal::ZERO {
            continue;
        } else if pot || line.destination == "plan" || line.destination == "buffer" {
            assigned += line.amount;
        }
    }
    let projected = inputs.projected_available;
    let everyday = inputs.everyday_remaining.max(Decimal::ZERO);
    let known_future = inputs.known_future_payments.max(Decimal::ZERO);
    let safety_buffer = inputs.safety_buffer;
    Figures {
        projected,
        everyday,
        known_future,
        safety_buffer,
        takes,
        general_takes,
        assigned,
        general,
        suggested: projected - everyday - known_future - safety_buffer - assigned + takes,
        general_balance: inputs.general_balance,
    }
}

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Short month name (`en`) `offset` months after `month` (YYYY-MM-01).
fn month_name(month: &str, offset: i32) -> String {
    let index = month
        .get(5..7)
        .and_then(|m| m.parse::<i32>().ok())
        .unwrap_or(1)
        - 1
        + offset;
    MONTHS_EN[index.rem_euclid(12) as usize].to_owned()
}

/// `Oct 26, 2026` (`en`).
fn day_name(day: &str) -> String {
    match (
        day.get(5..7).and_then(|m| m.parse::<usize>().ok()),
        day.get(8..10).and_then(|d| d.parse::<u32>().ok()),
        day.get(..4),
    ) {
        (Some(m @ 1..=12), Some(d), Some(y)) => format!("{} {d}, {y}", MONTHS_EN[m - 1]),
        _ => day.to_owned(),
    }
}

/// What the everyday forecast left out of the last six months.
fn excluded_row(excluded: &Option<ForecastExclusions>) -> Option<impl IntoView + use<>> {
    let excluded = excluded.as_ref()?;
    let mut parts = Vec::new();
    if excluded.large_count > 0 {
        parts.push(format!(
            "{} {} over {} ({})",
            excluded.large_count,
            if excluded.large_count == 1 {
                "purchase"
            } else {
                "purchases"
            },
            eur(excluded.limit),
            eur(excluded.large_amount)
        ));
    }
    parts.extend(
        excluded
            .categories
            .iter()
            .map(|c| format!("{} ({})", c.category, eur(c.amount))),
    );
    (!parts.is_empty()).then(|| {
        view! {
            <li class=BREAKDOWN_ROW>
                <span>{format!("Left out of the forecast (last 6 months): {}", parts.join(", "))}</span>
                <span></span>
            </li>
        }
    })
}

fn row(label: String, value: String) -> impl IntoView {
    view! { <li class=BREAKDOWN_ROW><span>{label}</span><span>{value}</span></li> }
}

/// Suggests what to move to general savings, or how much to take from it
/// when money is short.
#[component]
pub fn SuggestionPanel(
    inputs: SavingsSuggestion,
    figures: Figures,
    disabled: Signal<bool>,
    /// Adds (or grows) a general-pot line by the amount.
    on_apply: Callback<Decimal>,
    /// Adds (or grows) a take from the general pot by the amount.
    on_take: Callback<Decimal>,
    /// Keeps the "Why this number?" disclosure open across recomputes.
    open: RwSignal<bool>,
) -> impl IntoView {
    let Figures {
        suggested,
        general,
        general_balance,
        ..
    } = figures;
    let to_apply = suggested - general;
    let short = if suggested < Decimal::ZERO {
        suggested.abs()
    } else {
        Decimal::ZERO
    };
    // General savings can give what it held before this split, less what
    // this split already takes from it.
    let can_take = short
        .min(general_balance - figures.general_takes)
        .max(Decimal::ZERO);
    let forecast_label = if inputs
        .forecast_model
        .as_deref()
        .is_some_and(|m| m.contains("timesfm"))
    {
        "TimesFM"
    } else {
        "weekday baseline"
    };
    let positive = (suggested > Decimal::ZERO).then(|| {
        view! {
            <p><strong>{format!("Suggested to General savings: {}", eur(suggested))}</strong></p>
            {(general > Decimal::ZERO).then(|| view! {
                <small class=SMALL>{format!("{} already assigned to General savings in this split.", eur(general))}</small>
            })}
            {if to_apply > Decimal::ZERO {
                view! {
                    <div class=ACTIONS>
                        <Button size=Size::Sm disabled=disabled on_click=Callback::new(move |_| on_apply.run(to_apply))>
                            {format!("Apply {}", eur(to_apply))}
                        </Button>
                    </div>
                }
                .into_any()
            } else {
                let text = if to_apply.is_zero() {
                    "Suggestion applied.".to_owned()
                } else {
                    format!("{} more than suggested.", eur(to_apply.abs()))
                };
                view! { <small class=SMALL>{text}</small> }.into_any()
            }}
        }
    });
    let negative = (suggested < Decimal::ZERO).then(|| {
        let tail = if can_take > Decimal::ZERO {
            format!(" — take {} from General savings?", eur(can_take))
        } else {
            ". General savings has nothing to take.".to_owned()
        };
        view! {
            <p><strong>{format!("{} is short by {}", month_name(&inputs.month, 0), eur(short))}</strong>{tail}</p>
            {(can_take > Decimal::ZERO).then(|| view! {
                <div class=ACTIONS>
                    <Button size=Size::Sm variant=Variant::Secondary disabled=disabled on_click=Callback::new(move |_| on_take.run(can_take))>
                        {format!("Take {}", eur(can_take))}
                    </Button>
                </div>
            })}
        }
    });
    let known = (!inputs.known_payments.is_empty()).then(|| {
        let items = inputs
            .known_payments
            .iter()
            .map(|p| row(format!("{} (due {})", p.name, p.due_on), eur(p.amount)))
            .collect_view();
        view! { <li class="pl-4"><ul class=cn([BREAKDOWN, "text-fg-muted"])>{items}</ul></li> }
    });
    let forecast = if inputs.forecast_available {
        view! {
            {row(format!("− Everyday spending still expected ({forecast_label})"), eur(figures.everyday))}
            {excluded_row(&inputs.forecast_excluded)}
        }
        .into_any()
    } else {
        row(
            "Forecast unavailable — everyday spending not reserved".into(),
            String::new(),
        )
        .into_any()
    };
    view! {
        <section class=CALLOUT aria-label="Savings suggestion">
            {positive}
            {negative}
            {suggested.is_zero().then(|| view! { <p><strong>"Nothing left to suggest for General savings."</strong></p> })}
            <details prop:open=move || open.get() on:toggle=move |e| {
                if let Some(target) = e.target() {
                    let now = js_sys::Reflect::get(&target, &"open".into())
                        .ok()
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if now != open.get_untracked() {
                        open.set(now);
                    }
                }
            }>
                <summary class="cursor-pointer text-xs font-medium">"Why this number?"</summary>
                <ul class=cn([BREAKDOWN, "mt-2"])>
                    {row(format!("As of {}, for {}", day_name(&inputs.as_of), month_name(&inputs.month, 0)), String::new())}
                    {row("Bank balance on spending accounts".into(), eur(inputs.bank_balance))}
                    {(inputs.protected_savings > Decimal::ZERO).then(|| row("− Protected savings goals".into(), eur(inputs.protected_savings)))}
                    {(inputs.salary_reserved > Decimal::ZERO).then(|| row("− Salary kept for next month".into(), eur(inputs.salary_reserved)))}
                    {row("− Bills still to pay this month".into(), eur(inputs.bills_remaining))}
                    {row("− Planned payments still to pay".into(), eur(inputs.plans_remaining))}
                    {row("− Groups still to spend".into(), eur(inputs.groups_remaining))}
                    {row("+ Income still expected".into(), eur(inputs.expected_income))}
                    <li class=TOTAL_ROW><span>"= Projected available (same as Overview)"</span><strong>{eur(figures.projected)}</strong></li>
                    {forecast}
                    {row(
                        format!("− Known payments {}–{}", month_name(&inputs.month, 1), month_name(&inputs.month, 3)),
                        eur(figures.known_future),
                    )}
                    {known}
                    {row("− Safety buffer".into(), eur(figures.safety_buffer))}
                    {row("− Already assigned in this split".into(), eur(figures.assigned))}
                    {(figures.takes > Decimal::ZERO).then(|| row("+ Taken from pots".into(), eur(figures.takes)))}
                    <li class=TOTAL_ROW><span>"= Suggested"</span><strong>{eur(suggested)}</strong></li>
                    {row("General savings balance before this month's split".into(), eur(general_balance))}
                </ul>
            </details>
        </section>
    }
}
