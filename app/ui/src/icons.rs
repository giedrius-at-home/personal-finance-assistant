//! Line icons (24 px grid, 1.75 stroke), drawn inline so they follow `currentColor`.

use leptos::prelude::*;

use crate::Page;

fn icon(paths: &'static str) -> impl IntoView {
    view! {
        <svg
            class="icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.75"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            inner_html=paths
        ></svg>
    }
}

pub fn page(page: Page) -> impl IntoView {
    icon(match page {
        Page::Overview => {
            r#"<rect x="3.5" y="3.5" width="7" height="8" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="5" rx="1.5"/><rect x="13.5" y="11.5" width="7" height="9" rx="1.5"/><rect x="3.5" y="14.5" width="7" height="6" rx="1.5"/>"#
        }
        Page::Transactions => {
            r#"<path d="M4 7h13M14 4l3 3-3 3"/><path d="M20 17H7M10 14l-3 3 3 3"/>"#
        }
        Page::Accounts => {
            r#"<rect x="3" y="5.5" width="18" height="13" rx="2.5"/><path d="M3 10h18M7 15h3"/>"#
        }
        Page::Categories => {
            r#"<path d="M12 3.5a8.5 8.5 0 1 0 8.5 8.5H12z"/><path d="M15 3.8A8.5 8.5 0 0 1 20.2 9H15z"/>"#
        }
        Page::Data => {
            r#"<ellipse cx="12" cy="6" rx="7.5" ry="2.75"/><path d="M4.5 6v6c0 1.5 3.4 2.75 7.5 2.75S19.5 13.5 19.5 12V6"/><path d="M4.5 12v6c0 1.5 3.4 2.75 7.5 2.75S19.5 19.5 19.5 18v-6"/>"#
        }
    })
}

pub fn search() -> impl IntoView {
    icon(r#"<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>"#)
}

pub fn close() -> impl IntoView {
    icon(r#"<path d="M6 6l12 12M18 6 6 18"/>"#)
}

pub fn chevron_left() -> impl IntoView {
    icon(r#"<path d="m14.5 6-6 6 6 6"/>"#)
}

pub fn chevron_right() -> impl IntoView {
    icon(r#"<path d="m9.5 6 6 6-6 6"/>"#)
}

pub fn arrow_right() -> impl IntoView {
    icon(r#"<path d="M5 12h14M13 6l6 6-6 6"/>"#)
}
