use finance_types::AccountSummary;
use leptos::prelude::*;

use crate::api::{NoArgs, load};
use crate::pages::overview::role_label;
use crate::widgets::{Problem, Skeleton, amounts};
use crate::{Filters, format, state};

#[component]
pub fn Accounts() -> impl IntoView {
    let data = load::<Vec<AccountSummary>, _>("accounts", || NoArgs {});
    move || match data.data.get() {
        None => view! { <Skeleton rows=4 /> }.into_any(),
        Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
        Some(Ok(list)) => view! { <Grid list=list /> }.into_any(),
    }
}

#[component]
fn Grid(list: Vec<AccountSummary>) -> impl IntoView {
    let app = state();
    let mut total = std::collections::BTreeMap::<String, rust_decimal::Decimal>::new();
    for m in list.iter().flat_map(|a| &a.balances) {
        *total.entry(m.currency.clone()).or_default() += m.amount;
    }
    let total: Vec<_> = total
        .into_iter()
        .map(|(currency, amount)| finance_types::Money { amount, currency })
        .collect();
    view! {
        <div class="accounts-head">
            <p class="muted">"Click an account to see its transactions."</p>
            <span class="stepper-total"><span class="muted">"Total "</span>{amounts(&total, false)}</span>
        </div>
        <div class="account-grid">
            {list
                .into_iter()
                .map(|a| {
                    let id = a.id;
                    view! {
                        <button
                            class="account card"
                            on:click=move |_| app.show_transactions(Filters { account: Some(id), ..Filters::default() })
                        >
                            <span class="account-top">
                                <span class="row-title">{a.name}</span>
                                <span class="chip quiet">{a.role.map(role_label).unwrap_or("Account")}</span>
                            </span>
                            <span class="account-balance">{amounts(&a.balances, false)}</span>
                            <span class="row-sub">
                                {a.iban_tail.map(|t| format!("IBAN ••{t} · ")).unwrap_or_default()}
                                {format!("{} transactions", format::count(a.transaction_count))}
                                {a.last_activity.map(|d| format!(" · last {}", format::date(&d))).unwrap_or_default()}
                            </span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
