//! Finance UI: the app shell, shared state and navigation.

mod api;
mod format;
mod icons;
mod pages;
mod widgets;

use finance_types::{Facets, TxKind, TxQuery};
use leptos::ev;
use leptos::prelude::*;

use crate::api::{Loaded, NoArgs, load};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Transactions,
    Accounts,
    Categories,
    Data,
}

impl Page {
    const ALL: [Self; 5] = [
        Self::Overview,
        Self::Transactions,
        Self::Accounts,
        Self::Categories,
        Self::Data,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Transactions => "Transactions",
            Self::Accounts => "Accounts",
            Self::Categories => "Categories",
            Self::Data => "Data",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CategoryFilter {
    #[default]
    Any,
    Uncategorised,
    Id(i64),
}

/// Period filter; presets are resolved against today when the query is built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Period {
    #[default]
    All,
    ThisMonth,
    LastMonth,
    Last3Months,
    ThisYear,
    LastYear,
    Month(String),
    Custom {
        from: String,
        to: String,
    },
}

impl Period {
    pub fn range(&self) -> (Option<String>, Option<String>) {
        let today = format::today();
        let this_month = today[..7].to_owned();
        let year: i32 = today[..4].parse().unwrap_or(1970);
        let some = |a: String, b: String| (Some(a), Some(b));
        match self {
            Self::All => (None, None),
            Self::ThisMonth => some(format!("{this_month}-01"), today),
            Self::LastMonth => {
                let m = format::add_months(&this_month, -1);
                some(format!("{m}-01"), format::month_end(&m))
            }
            Self::Last3Months => some(format!("{}-01", format::add_months(&this_month, -2)), today),
            Self::ThisYear => some(format!("{year}-01-01"), today),
            Self::LastYear => some(format!("{}-01-01", year - 1), format!("{}-12-31", year - 1)),
            Self::Month(m) => some(format!("{m}-01"), format::month_end(m)),
            Self::Custom { from, to } => (
                Some(from.clone()).filter(|s| !s.is_empty()),
                Some(to.clone()).filter(|s| !s.is_empty()),
            ),
        }
    }
}

/// Transaction filters, shared so other screens can open a filtered list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub text: String,
    pub account: Option<i64>,
    pub category: CategoryFilter,
    pub kind: Option<TxKind>,
    pub period: Period,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn query(&self, limit: u32) -> TxQuery {
        let (from, to) = self.period.range();
        TxQuery {
            text: Some(self.text.trim().to_owned()).filter(|t| !t.is_empty()),
            account_id: self.account,
            category_id: match self.category {
                CategoryFilter::Id(id) => Some(id),
                _ => None,
            },
            uncategorised: self.category == CategoryFilter::Uncategorised,
            kind: self.kind,
            from,
            to,
            limit,
            offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AppState {
    pub page: RwSignal<Page>,
    pub filters: RwSignal<Filters>,
    pub facets: Loaded<Facets>,
    /// Bumped to ask the search box to take focus.
    pub focus_search: RwSignal<u32>,
}

impl AppState {
    /// Open Transactions with exactly these filters.
    pub fn show_transactions(&self, filters: Filters) {
        self.filters.set(filters);
        self.page.set(Page::Transactions);
    }
}

pub fn state() -> AppState {
    expect_context::<AppState>()
}

#[component]
fn App() -> impl IntoView {
    let app = AppState {
        page: RwSignal::new(Page::Overview),
        filters: RwSignal::new(Filters::default()),
        facets: load("facets", || NoArgs {}),
        focus_search: RwSignal::new(0),
    };
    provide_context(app);

    // ⌘1–5 switch screens, ⌘F searches transactions.
    let handle = window_event_listener(ev::keydown, move |e| {
        if !e.meta_key() || e.shift_key() || e.alt_key() || e.ctrl_key() {
            return;
        }
        let key = e.key();
        if let Some(page) = key
            .parse::<usize>()
            .ok()
            .and_then(|n| Page::ALL.get(n.wrapping_sub(1)))
        {
            e.prevent_default();
            app.page.set(*page);
        } else if key == "f" {
            e.prevent_default();
            app.page.set(Page::Transactions);
            app.focus_search.update(|n| *n += 1);
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="app">
            <Sidebar />
            <main class="main">
                <header class="topbar" data-tauri-drag-region="">
                    <h1 data-tauri-drag-region="">{move || app.page.get().title()}</h1>
                </header>
                <div class="content">
                    {move || match app.page.get() {
                        Page::Overview => pages::overview::Overview().into_any(),
                        Page::Transactions => pages::transactions::Transactions().into_any(),
                        Page::Accounts => pages::accounts::Accounts().into_any(),
                        Page::Categories => pages::categories::Categories().into_any(),
                        Page::Data => pages::data::Data().into_any(),
                    }}
                </div>
            </main>
        </div>
    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let app = state();
    let status = load::<finance_types::DataStatus, _>("status", || NoArgs {});
    view! {
        <nav class="sidebar" aria-label="Sections">
            <div class="sidebar-drag" data-tauri-drag-region=""></div>
            <ul class="nav">
                {Page::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(i, page)| {
                        view! {
                            <li>
                                <button
                                    class="nav-item"
                                    class:active=move || app.page.get() == page
                                    aria-current=move || (app.page.get() == page).then_some("page")
                                    title=format!("{} (⌘{})", page.title(), i + 1)
                                    on:click=move |_| app.page.set(page)
                                >
                                    {icons::page(page)}
                                    <span class="nav-label">{page.title()}</span>
                                </button>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
            <div class="sidebar-foot">
                {move || {
                    status
                        .data
                        .get()
                        .and_then(Result::ok)
                        .and_then(|s| s.last_date)
                        .map(|d| {
                            view! {
                                <span class="dot"></span>
                                <span class="nav-label">"Data through " {format::date(&d)}</span>
                            }
                        })
                }}
            </div>
        </nav>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
