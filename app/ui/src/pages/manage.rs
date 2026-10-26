//! The Manage section: banks, month closing, import, status and settings.

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Banks() -> impl IntoView {
    view! { <Card><EmptyState title="Banks" /></Card> }
}

#[component]
pub fn CloseMonth() -> impl IntoView {
    view! { <Card><EmptyState title="Close month" /></Card> }
}

#[component]
pub fn Import() -> impl IntoView {
    view! { <Card><EmptyState title="Import" /></Card> }
}

#[component]
pub fn Status() -> impl IntoView {
    view! { <Card><EmptyState title="Status" /></Card> }
}

#[component]
pub fn Settings() -> impl IntoView {
    view! { <Card><EmptyState title="Settings" /></Card> }
}
