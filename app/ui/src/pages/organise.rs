//! Spending → Organise categories (personal-site `spending-organization.tsx`,
//! `owner-category-groups.tsx`, `category-consolidation.tsx`).

use leptos::prelude::*;

use crate::ds::{Card, EmptyState};

#[component]
pub fn Organise() -> impl IntoView {
    view! { <Card><EmptyState title="Organise categories is being rebuilt" /></Card> }
}
