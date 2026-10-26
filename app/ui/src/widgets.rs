//! Small building blocks shared by the screens.

use finance_types::{Money, MonthFlow};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::format;

/// An amount with tabular figures, coloured by sign when `tone` is set.
#[component]
pub fn Amount(
    amount: Decimal,
    #[prop(into)] currency: String,
    #[prop(optional)] signed: bool,
    #[prop(optional)] tone: bool,
    #[prop(optional)] whole: bool,
) -> impl IntoView {
    let text = match (signed, whole) {
        (true, _) => format::money_signed(amount, &currency),
        (false, true) => format::money_dp(amount, &currency, 0),
        (false, false) => format::money(amount, &currency),
    };
    let class = match (
        tone,
        amount.is_sign_negative() && !amount.is_zero(),
        amount.is_zero(),
    ) {
        (false, _, _) | (true, _, true) => "amount",
        (true, true, _) => "amount negative",
        (true, false, _) => "amount positive",
    };
    view! { <span class=class>{text}</span> }
}

/// Several currencies on one line (`€1,200.00 · $40.00`).
pub fn amounts(list: &[Money], signed: bool) -> impl IntoView + use<> {
    if list.is_empty() {
        return view! { <span class="amount">"—"</span> }.into_any();
    }
    list.iter()
        .enumerate()
        .map(|(i, m)| {
            view! {
                {(i > 0).then_some(view! { <span class="sep">" · "</span> })}
                <Amount amount=m.amount currency=m.currency.clone() signed=signed />
            }
        })
        .collect_view()
        .into_any()
}

#[component]
pub fn Stat(
    #[prop(into)] label: String,
    children: Children,
    #[prop(optional, into)] hint: Option<String>,
) -> impl IntoView {
    view! {
        <div class="stat card">
            <div class="stat-label">{label}</div>
            <div class="stat-value">{children()}</div>
            {hint.map(|h| view! { <div class="stat-hint">{h}</div> })}
        </div>
    }
}

/// Explains a failed load in plain words.
#[component]
pub fn Problem(#[prop(into)] error: String) -> impl IntoView {
    if error == "no-data" {
        view! {
            <div class="empty card">
                <h2>"No finance data yet"</h2>
                <p>
                    "Copy your data from laptopthree once with "
                    <code>"finance-migrate"</code>
                    ", then reopen this window."
                </p>
            </div>
        }
        .into_any()
    } else {
        view! {
            <div class="empty card">
                <h2>"Could not load this view"</h2>
                <p class="muted">{error}</p>
            </div>
        }
        .into_any()
    }
}

#[component]
pub fn Skeleton(#[prop(default = 3)] rows: usize) -> impl IntoView {
    view! {
        <div class="skeleton" aria-busy="true" aria-label="Loading">
            {(0..rows).map(|_| view! { <div class="skeleton-row"></div> }).collect_view()}
        </div>
    }
}

/// Income and spending per month as paired bars. Clicking a month calls `on_pick`.
#[component]
pub fn MonthChart(
    flows: Vec<MonthFlow>,
    currency: String,
    on_pick: Callback<String>,
) -> impl IntoView {
    let flows: Vec<MonthFlow> = flows
        .into_iter()
        .filter(|f| f.currency == currency)
        .collect();
    let max = flows
        .iter()
        .flat_map(|f| [f.income, f.spending])
        .max()
        .unwrap_or_default()
        .max(Decimal::ONE);
    let scale = nice_ceiling(max);
    let pct = move |v: Decimal| -> String {
        let p = (v / scale * Decimal::ONE_HUNDRED)
            .round_dp(2)
            .max(Decimal::ZERO);
        format!("{p}%")
    };
    let ticks: Vec<Decimal> = (0..=4)
        .map(|i| scale * Decimal::from(i) / Decimal::from(4))
        .collect();
    let hovered = RwSignal::new(None::<usize>);
    let cur = currency.clone();
    let tooltip_flows = flows.clone();
    view! {
        <div class="chart" on:mouseleave=move |_| hovered.set(None)>
            <div class="chart-grid" aria-hidden="true">
                {ticks
                    .iter()
                    .rev()
                    .map(|t| {
                        view! {
                            <div class="chart-gridline">
                                <span>{format::money_dp(*t, &cur, 0)}</span>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
            <div class="chart-bars">
                {flows
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let month = f.month.clone();
                        let label = format!(
                            "{}: income {}, spending {}",
                            format::month_long(&f.month),
                            format::money(f.income, &currency),
                            format::money(f.spending, &currency),
                        );
                        view! {
                            <button
                                class="chart-col"
                                class:hover=move || hovered.get() == Some(i)
                                aria-label=label
                                on:mouseenter=move |_| hovered.set(Some(i))
                                on:focus=move |_| hovered.set(Some(i))
                                on:click=move |_| on_pick.run(month.clone())
                            >
                                <div class="chart-pair">
                                    <div class="bar income" style:height=pct(f.income)></div>
                                    <div class="bar spending" style:height=pct(f.spending)></div>
                                </div>
                                <span class="chart-label">{format::month_short(&f.month)}</span>
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            {move || {
                hovered
                    .get()
                    .and_then(|i| tooltip_flows.get(i).cloned().map(|f| (i, f)))
                    .map(|(i, f)| {
                        let n = tooltip_flows.len().max(1) as f64;
                        let centre = (i as f64 + 0.5) / n;
                        // Beside the column, on the side with more room.
                        let left = format!("calc(64px + (100% - 64px) * {centre:.4})");
                        let flip = centre > 0.5;
                        let net = f.income - f.spending;
                        view! {
                            <div class="chart-tip" class:flip=flip style:left=left>
                                <strong>{format::month_long(&f.month)}</strong>
                                <div class="tip-row">
                                    <span class="swatch income"></span>
                                    "Income"
                                    <Amount amount=f.income currency=currency.clone() />
                                </div>
                                <div class="tip-row">
                                    <span class="swatch spending"></span>
                                    "Spending"
                                    <Amount amount=f.spending currency=currency.clone() />
                                </div>
                                <div class="tip-row net">
                                    "Net"
                                    <Amount amount=net currency=currency.clone() signed=true tone=true />
                                </div>
                                <div class="tip-hint">"Click to see transactions"</div>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// Round `value` up to 1, 2 or 5 × 10ⁿ so the axis has readable ticks.
fn nice_ceiling(value: Decimal) -> Decimal {
    let mut magnitude = Decimal::ONE;
    while magnitude * Decimal::TEN <= value {
        magnitude *= Decimal::TEN;
    }
    for step in [1, 2, 4, 5, 10] {
        let candidate = magnitude * Decimal::from(step);
        if candidate >= value {
            return candidate;
        }
    }
    magnitude * Decimal::TEN
}
