use finance_types::CategoryTotal;
use leptos::prelude::*;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::load;
use crate::pages::overview::{CategoryBars, primary_currency};
use crate::widgets::{Amount, Problem, Skeleton};
use crate::{format, icons, state};

#[derive(Serialize)]
struct Args {
    month: String,
}

#[component]
pub fn Categories() -> impl IntoView {
    let app = state();
    let bounds = move || {
        app.facets.data.get().and_then(Result::ok).map(|f| {
            (
                f.first_date.map(|d| d[..7].to_owned()).unwrap_or_default(),
                f.last_date
                    .map(|d| d[..7].to_owned())
                    .unwrap_or_else(|| format::today()[..7].to_owned()),
            )
        })
    };
    let month = RwSignal::new(format::today()[..7].to_owned());
    // Start on the latest month that has data.
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        match bounds() {
            Some((_, last)) => {
                month.set(last);
                true
            }
            None => false,
        }
    });
    let totals =
        load::<Vec<CategoryTotal>, _>("category_totals", move || Args { month: month.get() });
    let can_go = move |delta: i32| {
        bounds().is_some_and(|(first, last)| {
            let next = format::add_months(&month.get(), delta);
            next >= first && next <= last
        })
    };

    view! {
        <div class="month-stepper">
            <button
                class="icon-button"
                title="Previous month"
                disabled=move || !can_go(-1)
                on:click=move |_| month.update(|m| *m = format::add_months(m, -1))
            >
                {icons::chevron_left()}
            </button>
            <h2>{move || format::month_long(&month.get())}</h2>
            <button
                class="icon-button"
                title="Next month"
                disabled=move || !can_go(1)
                on:click=move |_| month.update(|m| *m = format::add_months(m, 1))
            >
                {icons::chevron_right()}
            </button>
            {move || {
                totals
                    .data
                    .get()
                    .and_then(Result::ok)
                    .map(|t| {
                        let currency = primary_currency(
                            &t
                                .iter()
                                .map(|c| finance_types::Money { amount: c.amount, currency: c.currency.clone() })
                                .collect::<Vec<_>>(),
                        );
                        let sum: Decimal = t.iter().filter(|c| c.currency == currency).map(|c| c.amount).sum();
                        view! {
                            <span class="stepper-total">
                                <span class="muted">"Spent "</span>
                                <Amount amount=sum currency=currency />
                            </span>
                        }
                    })
            }}
        </div>
        <section class="card panel" class:loading=move || totals.loading.get()>
            {move || match totals.data.get() {
                None => view! { <Skeleton rows=8 /> }.into_any(),
                Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
                Some(Ok(t)) => view! { <CategoryBars totals=t month=month.get_untracked() /> }.into_any(),
            }}
        </section>
    }
}
