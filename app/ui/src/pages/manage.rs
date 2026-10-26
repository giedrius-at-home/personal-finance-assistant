//! The Manage section: banks, month closing, import, status and settings.

mod banks;
mod import;
mod settings;
mod status;
mod ui;

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Banks() -> impl IntoView {
    banks::Banks()
}

#[component]
pub fn CloseMonth() -> impl IntoView {
    view! { <Card><EmptyState title="Close month" /></Card> }
}

#[component]
pub fn Import() -> impl IntoView {
    import::Import()
}

#[component]
pub fn Status() -> impl IntoView {
    status::Status()
}

#[component]
pub fn Settings() -> impl IntoView {
    settings::Settings()
}
