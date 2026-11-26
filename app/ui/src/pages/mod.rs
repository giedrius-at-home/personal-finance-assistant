//! One module per primary section, plus what they share.

pub mod adjustments;
pub mod brief_popup;
pub mod category_trends;
pub mod closure;
pub mod events;
pub mod goals;
pub mod manage;
pub mod organise;
pub mod overview;
pub mod payment_pick;
pub mod plan;
pub mod spending;
pub mod transactions;
pub mod wishes;

use finance_types::report::Dashboard;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ds::patterns::{CARD_GRID, MUTED, SKELETON_CARD, STACK, TITLE, TITLE_BLOCK};
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
                <div class=STACK>
                    <Skeleton class="h-11 w-full max-w-md rounded-pill" />
                    <div class=CARD_GRID>
                        <Skeleton class=SKELETON_CARD />
                        <Skeleton class=SKELETON_CARD />
                    </div>
                </div>
            }
            .into_any(),
            Some(Err(e)) => view! { <Problem error=e.clone() /> }.into_any(),
            Some(Ok(report)) => f(report).into_any(),
        })
    }
}

/// Whether the loaded report passes `f`; nothing that asks draws before it loads.
pub fn report_is(f: impl FnOnce(&Dashboard) -> bool) -> bool {
    crate::state()
        .report
        .data
        .with(|d| matches!(d, Some(Ok(r)) if f(r)))
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
        <Card tone=CardTone::Muted class=TITLE_BLOCK>
            <h2 class=TITLE>{title}</h2>
            <p role="alert" class=MUTED>{detail}</p>
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
