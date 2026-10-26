//! The Transactions section.

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Transactions() -> impl IntoView {
    view! {
        <Card>
            <EmptyState title="Transactions is being rebuilt" />
        </Card>
    }
}
