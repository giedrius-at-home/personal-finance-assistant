//! The Spending section: plan & patterns, organising categories, history.

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn PlanPatterns() -> impl IntoView {
    view! { <Card><EmptyState title="Plan & patterns" /></Card> }
}

#[component]
pub fn Organise() -> impl IntoView {
    view! { <Card><EmptyState title="Organise categories" /></Card> }
}

#[component]
pub fn History() -> impl IntoView {
    view! { <Card><EmptyState title="All spending history" /></Card> }
}
