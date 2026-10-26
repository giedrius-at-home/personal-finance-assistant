//! One module per primary section, plus what they share.

pub mod category_trends;
pub mod closure;
pub mod manage;
pub mod organise;
pub mod overview;
pub mod plan;
pub mod spending;
pub mod transactions;

use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ds::{Card, CardTone, Skeleton};
use crate::state;

/// Render `f` with the current dashboard; a skeleton while it first loads
/// and the error when it fails.
pub fn with_report<V: IntoView + 'static>(
    f: impl Fn(&Dashboard) -> V + Send + Sync + 'static,
) -> impl IntoView {
    let app = state();
    move || {
        app.report.data.with(|data| match data {
            None => view! {
                <div class="grid gap-4">
                    <Skeleton class="h-11 w-full max-w-md rounded-pill" />
                    <div class="grid gap-4 md:grid-cols-2">
                        <Skeleton class="h-56 rounded-xl" />
                        <Skeleton class="h-56 rounded-xl" />
                    </div>
                </div>
            }
            .into_any(),
            Some(Err(e)) => view! { <Problem error=e.clone() /> }.into_any(),
            Some(Ok(report)) => f(report).into_any(),
        })
    }
}

/// Explains a failed load in plain words.
#[component]
pub fn Problem(#[prop(into)] error: String) -> impl IntoView {
    let (title, detail) = if error == "no-data" {
        (
            "No finance data yet",
            "Copy your data once with finance-migrate, then reopen this window.".to_owned(),
        )
    } else {
        ("Could not load this view", error)
    };
    view! {
        <Card tone=CardTone::Muted class="grid gap-1">
            <h2 class="text-base font-semibold">{title}</h2>
            <p role="alert" class="text-sm text-fg-muted">{detail}</p>
        </Card>
    }
}

/// A decimal as `f64` for chart geometry only (never for money arithmetic).
pub fn f(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

/// `n record` / `n records`.
pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}
