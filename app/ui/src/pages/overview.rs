use finance_types::{Money, Overview as Data};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::api::{NoArgs, load};
use crate::widgets::{Amount, MonthChart, Problem, Skeleton, Stat, amounts};
use crate::{CategoryFilter, Filters, Page, Period, format, icons, state};

/// The currency most of the money is in.
pub fn primary_currency(net_worth: &[Money]) -> String {
    net_worth
        .iter()
        .max_by_key(|m| m.amount.abs())
        .map(|m| m.currency.clone())
        .unwrap_or_else(|| "EUR".to_owned())
}

#[component]
pub fn Overview() -> impl IntoView {
    let data = load::<Data, _>("overview", || NoArgs {});
    move || match data.data.get() {
        None => view! { <Skeleton rows=6 /> }.into_any(),
        Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
        Some(Ok(o)) => view! { <Loaded o=o /> }.into_any(),
    }
}

#[component]
fn Loaded(o: Data) -> impl IntoView {
    let app = state();
    let currency = primary_currency(&o.net_worth);
    let months: Vec<_> = o
        .months
        .iter()
        .filter(|m| m.currency == currency)
        .cloned()
        .collect();
    let current = months.last().cloned();
    let previous = months
        .len()
        .checked_sub(2)
        .and_then(|i| months.get(i))
        .cloned();
    let is_this_month = current
        .as_ref()
        .is_some_and(|m| format::today().starts_with(&m.month));
    let month_name = current
        .as_ref()
        .map(|m| format::month_long(&m.month))
        .unwrap_or_default();
    let month_title = if is_this_month {
        format!("{month_name} so far")
    } else {
        month_name
    };

    let spending_hint = match (&current, &previous) {
        (Some(c), Some(p)) if !p.spending.is_zero() => {
            let change = ((c.spending - p.spending) / p.spending * Decimal::ONE_HUNDRED).round();
            let sign = if change > Decimal::ZERO {
                "+"
            } else if change < Decimal::ZERO {
                "\u{2212}"
            } else {
                ""
            };
            Some(format!(
                "{sign}{}% vs {}",
                change.abs(),
                format::month(&p.month)
            ))
        }
        _ => None,
    };
    let net = current
        .as_ref()
        .map(|c| c.income - c.spending)
        .unwrap_or_default();
    let savings_hint = current.as_ref().filter(|c| !c.income.is_zero()).map(|c| {
        let kept = ((c.income - c.spending) / c.income * Decimal::ONE_HUNDRED).round();
        if kept.is_sign_negative() {
            format!("Spent {}% more than came in", kept.abs())
        } else {
            format!("{kept}% of income kept")
        }
    });
    let own_count = o.accounts.len();
    let income = current.as_ref().map(|c| c.income).unwrap_or_default();
    let spending = current.as_ref().map(|c| c.spending).unwrap_or_default();
    let (c1, c2, c3) = (currency.clone(), currency.clone(), currency.clone());

    let pick_month = Callback::new(move |month: String| {
        app.show_transactions(Filters {
            period: Period::Month(month),
            ..Filters::default()
        });
    });

    view! {
        <section class="stats">
            <Stat label="Net worth" hint=format!("Across {own_count} accounts")>
                {amounts(&o.net_worth, false)}
            </Stat>
            <Stat label=format!("Income · {month_title}")>
                <Amount amount=income currency=c1 />
            </Stat>
            <Stat label=format!("Spending · {month_title}") hint=spending_hint.unwrap_or_default()>
                <Amount amount=spending currency=c2 />
            </Stat>
            <Stat label=format!("Net · {month_title}") hint=savings_hint.unwrap_or_default()>
                <Amount amount=net currency=c3 signed=true tone=true />
            </Stat>
        </section>

        <section class="card panel">
            <div class="panel-head">
                <h2>"Income and spending"</h2>
                <div class="legend">
                    <span><span class="swatch income"></span>"Income"</span>
                    <span><span class="swatch spending"></span>"Spending"</span>
                </div>
            </div>
            <MonthChart flows=o.months.clone() currency=currency.clone() on_pick=pick_month />
        </section>

        <div class="columns">
            <section class="card panel">
                <div class="panel-head">
                    <h2>"Accounts"</h2>
                    <button class="link" on:click=move |_| app.page.set(Page::Accounts)>
                        "All accounts" {icons::arrow_right()}
                    </button>
                </div>
                <ul class="list">
                    {o
                        .accounts
                        .iter()
                        .take(8)
                        .map(|a| {
                            let id = a.id;
                            view! {
                                <li>
                                    <button
                                        class="list-row"
                                        on:click=move |_| {
                                            app.show_transactions(Filters { account: Some(id), ..Filters::default() })
                                        }
                                    >
                                        <span class="row-main">
                                            <span class="row-title">{a.name.clone()}</span>
                                            <span class="row-sub">
                                                {a.role.clone().map(role_label).unwrap_or("Account")}
                                                {a.iban_tail.clone().map(|t| format!(" · ••{t}"))}
                                            </span>
                                        </span>
                                        <span class="row-value">{amounts(&a.balances, false)}</span>
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </section>

            <section class="card panel">
                <div class="panel-head">
                    <h2>
                        "Spending by category"
                        <span class="muted small">
                            {o.category_month.as_deref().map(|m| format!(" · {}", format::month(m)))}
                        </span>
                    </h2>
                    <button class="link" on:click=move |_| app.page.set(Page::Categories)>
                        "All categories" {icons::arrow_right()}
                    </button>
                </div>
                <CategoryBars totals=o.categories.clone() month=o.category_month.clone().unwrap_or_default() limit=7 />
            </section>
        </div>
    }
}

pub fn plural(n: u64, word: &str) -> String {
    format!(
        "{} {word}{}",
        format::count(n),
        if n == 1 { "" } else { "s" }
    )
}

pub fn role_label(role: String) -> &'static str {
    match role.as_str() {
        "defaultAsset" => "Main account",
        "savingAsset" => "Savings",
        "sharedAsset" => "Shared",
        "ccAsset" => "Credit card",
        "cashWalletAsset" => "Cash",
        _ => "Account",
    }
}

/// Horizontal bars of category spending; rows open the matching transactions.
#[component]
pub fn CategoryBars(
    totals: Vec<finance_types::CategoryTotal>,
    month: String,
    #[prop(default = usize::MAX)] limit: usize,
) -> impl IntoView {
    let app = state();
    let currency = primary_currency(
        &totals
            .iter()
            .map(|t| Money {
                amount: t.amount,
                currency: t.currency.clone(),
            })
            .collect::<Vec<_>>(),
    );
    let totals: Vec<_> = totals
        .into_iter()
        .filter(|t| t.currency == currency)
        .collect();
    if totals.is_empty() {
        return view! { <p class="muted pad">"No spending this month yet."</p> }.into_any();
    }
    let sum: Decimal = totals.iter().map(|t| t.amount).sum();
    let top = totals
        .first()
        .map(|t| t.amount)
        .unwrap_or(Decimal::ONE)
        .max(Decimal::ONE);
    let shown = totals.len().min(limit);
    let rest = totals.len() - shown;
    view! {
        <ul class="bars">
            {totals
                .into_iter()
                .take(shown)
                .map(|t| {
                    let width = format!("{}%", (t.amount / top * Decimal::ONE_HUNDRED).round_dp(1).max(Decimal::ZERO));
                    let share = if sum.is_zero() {
                        Decimal::ZERO
                    } else {
                        (t.amount / sum * Decimal::ONE_HUNDRED).round()
                    };
                    let filter = match &t.category {
                        Some(c) => CategoryFilter::Id(c.id),
                        None => CategoryFilter::Uncategorised,
                    };
                    let name = t.category.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| "Uncategorised".into());
                    let month = month.clone();
                    let label = format!(
                        "{name}: {}, {share}% of spending, {}",
                        format::money(t.amount, &t.currency),
                        plural(t.count, "transaction"),
                    );
                    view! {
                        <li>
                            <button
                                class="bar-row"
                                aria-label=label.clone()
                                title=label
                                on:click=move |_| {
                                    app.show_transactions(Filters {
                                        category: filter.clone(),
                                        period: Period::Month(month.clone()),
                                        ..Filters::default()
                                    })
                                }
                            >
                                <span class="bar-name">{name}</span>
                                <span class="bar-track">
                                    <span class="bar-fill" style:width=width></span>
                                </span>
                                <span class="bar-share muted">{format!("{share}%")}</span>
                                <span class="bar-value">
                                    <Amount amount=t.amount currency=t.currency.clone() />
                                </span>
                            </button>
                        </li>
                    }
                })
                .collect_view()}
        </ul>
        {(rest > 0)
            .then(|| {
                view! {
                    <button class="link more-link" on:click=move |_| app.page.set(Page::Categories)>
                        {format!("{rest} more categories")}
                        {icons::arrow_right()}
                    </button>
                }
            })}
    }
    .into_any()
}
