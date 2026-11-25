//! The Manage section: banks, month closing, import, status and settings.

mod ai;
mod banks;
mod brief;
mod email;
mod import;
mod nudges;
mod push;
mod settings;
mod status;
mod ui;

use leptos::prelude::*;

use crate::layout::PanelDef;

/// Every Manage section's panels, in section order.
pub fn panels() -> impl Iterator<Item = &'static PanelDef> {
    banks::PANELS
        .iter()
        .chain(crate::pages::closure::PANELS)
        .chain(import::PANELS)
        .chain(status::PANELS)
        .chain(settings::PANELS)
}

#[component]
pub fn Banks() -> impl IntoView {
    banks::Banks()
}

#[component]
pub fn CloseMonth() -> impl IntoView {
    crate::pages::closure::MonthClosure()
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
