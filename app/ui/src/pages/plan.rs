//! The Plan section.

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Plan() -> impl IntoView {
    view! {
        <Card>
            <EmptyState title="Plan is being rebuilt" />
        </Card>
    }
}
