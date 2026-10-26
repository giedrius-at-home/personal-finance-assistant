//! The Overview section (personal-site `owner-monthly-overview.tsx`,
//! `monthly-budget.tsx`, `overview-money.tsx`).

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Overview() -> impl IntoView {
    view! { <Card><EmptyState title="Overview is being rebuilt" /></Card> }
}

/// The monthly budget panel, shared with Spending → Plan & patterns.
pub fn budget_panel() -> impl IntoView {
    view! { <Card><EmptyState title="Monthly budget is being rebuilt" /></Card> }
}
