//! The live savings suggestion beside the split (personal-site
//! `components/month-closure/suggestion.tsx`).

use finance_types::report::Currency;
use finance_types::report::closure::SavingsSuggestion;
use finance_types::report::outlook::ForecastExclusions;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::ui::CALLOUT;
use crate::ds::cn;
use crate::ds::patterns::Disclosure;

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
    /// Positive lines to wishes, goals, other accounts, plan lines and buffer
    /// lines.
    pub assigned: Decimal,
    /// projected − everyday − known future payments − safety buffer − assigned.
    pub suggested: Decimal,
}

/// What is available now less what is still expected this month, known
/// future payments, the safety buffer and this split's own lines.
pub fn figures(inputs: &SavingsSuggestion, lines: &[SuggestionLine]) -> Figures {
    let assigned: Decimal = lines
        .iter()
        .filter(|l| l.currency == "EUR" && l.amount > Decimal::ZERO)
        .filter(|l| {
            l.destination.starts_with("wish:")
                || l.destination.starts_with("goal:")
                || l.destination == "plan"
                || l.destination == "buffer"
                || l.destination.starts_with("account:")
        })
        .map(|l| l.amount)
        .sum();
    let projected = inputs.projected_available;
    let everyday = inputs.everyday_remaining.max(Decimal::ZERO);
    let known_future = inputs.known_future_payments.max(Decimal::ZERO);
    let safety_buffer = inputs.safety_buffer;
    Figures {
        projected,
        everyday,
        known_future,
        safety_buffer,
        assigned,
        suggested: projected - everyday - known_future - safety_buffer - assigned,
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

/// How much is short or left over once this month's needs are covered.
#[component]
pub fn SuggestionPanel(
    inputs: SavingsSuggestion,
    figures: Figures,
    /// Keeps the "Why this number?" disclosure open across recomputes.
    open: RwSignal<bool>,
) -> impl IntoView {
    let suggested = figures.suggested;
    let forecast_label = if inputs
        .forecast_model
        .as_deref()
        .is_some_and(|m| m.contains("timesfm"))
    {
        "TimesFM"
    } else {
        "weekday baseline"
    };
    let month = month_name(&inputs.month, 0);
    let headline = if suggested < Decimal::ZERO {
        format!("{month} is short by {}", eur(suggested.abs()))
    } else if suggested > Decimal::ZERO {
        format!("{month} has {} to spare", eur(suggested))
    } else {
        format!("{month} has nothing to spare")
    };
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
            <p><strong>{headline}</strong></p>
            <Disclosure summary_class="text-xs font-medium" summary="Why this number?" prop:open=move || open.get() on:toggle=move |e| {
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
                    {(figures.safety_buffer > Decimal::ZERO).then(|| row("− Safety buffer".into(), eur(figures.safety_buffer)))}
                    {row("− Already assigned in this split".into(), eur(figures.assigned))}
                    <li class=TOTAL_ROW><span>"= Suggested"</span><strong>{eur(suggested)}</strong></li>
                </ul>
            </Disclosure>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    fn inputs() -> SavingsSuggestion {
        SavingsSuggestion {
            as_of: "2026-10-02".into(),
            month: "2026-10-01".into(),
            projected_available: d("1000"),
            bank_balance: d("1000"),
            protected_savings: Decimal::ZERO,
            salary_reserved: Decimal::ZERO,
            bills_remaining: Decimal::ZERO,
            plans_remaining: Decimal::ZERO,
            groups_remaining: Decimal::ZERO,
            expected_income: Decimal::ZERO,
            everyday_remaining: d("200"),
            forecast_available: true,
            forecast_model: None,
            forecast_excluded: None,
            known_future_payments: d("100"),
            known_payments: Vec::new(),
            safety_buffer: d("300"),
        }
    }

    fn line(destination: &str, amount: &str) -> SuggestionLine {
        SuggestionLine {
            destination: destination.into(),
            amount: d(amount),
            currency: "EUR".into(),
        }
    }

    #[test]
    fn wish_lines_count_as_assigned() {
        let base = figures(&inputs(), &[]);
        assert_eq!(base.assigned, Decimal::ZERO);
        assert_eq!(base.suggested, d("400"));
        let wish = figures(&inputs(), &[line("wish:w1", "50")]);
        assert_eq!(wish.assigned, d("50"));
        assert_eq!(wish.suggested, base.suggested - d("50"));
        let stay = figures(&inputs(), &[line("stay", "50"), line("pot:old", "50")]);
        assert_eq!(stay, base);
    }

    #[test]
    fn move_lines_count_as_assigned() {
        let base = figures(&inputs(), &[]);
        let moved = figures(&inputs(), &[line("account:sw", "30")]);
        assert_eq!(moved.assigned, d("30"));
        assert_eq!(moved.suggested, base.suggested - d("30"));
    }
}
