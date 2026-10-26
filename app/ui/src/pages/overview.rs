//! The Overview section.

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Overview() -> impl IntoView {
    view! {
        <Card>
            <EmptyState title="Overview is being rebuilt" />
        </Card>
    }
}
