//! Finance UI: the app shell, shared state and navigation. The layout and
//! look follow the owner's web finance UI (personal-site `integrated-finance.tsx`).

mod api;
// The whole design system is ported so screens can follow the site; not
// every screen uses every part.
#[allow(dead_code)]
mod ds;
mod format;
mod pages;

use finance_types::report::Dashboard;
use leptos::ev;
use leptos::prelude::*;

use crate::api::{Loaded, NoArgs, load};
use crate::ds::{
    Eyebrow, Heading, Icon, Lucide, SegmentedControl, Size, TABS_LIST, TABS_TRIGGER, Variant,
    button_class, cn,
};

/// The five primary sections (the site's top tabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Transactions,
    Spending,
    Plan,
    Manage,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Overview,
        Self::Transactions,
        Self::Spending,
        Self::Plan,
        Self::Manage,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Transactions => "Transactions",
            Self::Spending => "Spending",
            Self::Plan => "Plan",
            Self::Manage => "Manage",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendingSection {
    PlanPatterns,
    Organise,
    History,
}

impl SpendingSection {
    const ALL: [Self; 3] = [Self::PlanPatterns, Self::Organise, Self::History];

    fn label(self) -> &'static str {
        match self {
            Self::PlanPatterns => "Plan & patterns",
            Self::Organise => "Organise categories",
            Self::History => "All spending history",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManageSection {
    Banks,
    CloseMonth,
    Import,
    Status,
    Settings,
}

impl ManageSection {
    const ALL: [Self; 5] = [
        Self::Banks,
        Self::CloseMonth,
        Self::Import,
        Self::Status,
        Self::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Banks => "Banks",
            Self::CloseMonth => "Close month",
            Self::Import => "Import",
            Self::Status => "Status",
            Self::Settings => "Settings",
        }
    }
}

/// Navigation and refresh state every screen can reach.
#[derive(Clone, Copy, Debug)]
pub struct AppState {
    pub tab: RwSignal<Tab>,
    pub spending: RwSignal<SpendingSection>,
    pub manage: RwSignal<ManageSection>,
    /// Bumped after every write so screens reload their data.
    pub revision: RwSignal<u32>,
    /// When the shown data was built (local `YYYY-MM-DD HH:MM`).
    pub updated: RwSignal<Option<String>>,
    pub refresh_failed: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    /// The dashboard report every screen reads; reloaded on `revision`.
    pub report: Loaded<Dashboard>,
}

impl AppState {
    pub fn go(&self, tab: Tab) {
        self.tab.set(tab);
    }

    pub fn go_spending(&self, section: SpendingSection) {
        self.spending.set(section);
        self.tab.set(Tab::Spending);
    }

    pub fn go_manage(&self, section: ManageSection) {
        self.manage.set(section);
        self.tab.set(Tab::Manage);
    }

    /// Re-read the data from disk and reload every screen.
    pub fn refresh(&self) {
        let revision = self.revision;
        self.busy.set(true);
        leptos::task::spawn_local(async move {
            let _ = api::invoke::<()>("refresh", &NoArgs {}).await;
            revision.update(|n| *n += 1);
        });
    }
}

pub fn state() -> AppState {
    expect_context::<AppState>()
}

#[component]
fn App() -> impl IntoView {
    let revision = RwSignal::new(0_u32);
    let app = AppState {
        tab: RwSignal::new(Tab::Overview),
        spending: RwSignal::new(SpendingSection::PlanPatterns),
        manage: RwSignal::new(ManageSection::Banks),
        revision,
        updated: RwSignal::new(None),
        refresh_failed: RwSignal::new(false),
        busy: RwSignal::new(false),
        report: load("dashboard", move || {
            revision.track();
            NoArgs {}
        }),
    };
    provide_context(app);
    Effect::new(move |_| {
        if let Some(result) = app.report.data.get() {
            app.busy.set(app.report.loading.get());
            match result {
                Ok(report) => {
                    app.updated.set(Some(report.generated_at.clone()));
                    app.refresh_failed.set(false);
                }
                Err(_) => app.refresh_failed.set(true),
            }
        }
    });
    if api::in_app()
        && let Some(root) = document().document_element()
    {
        let _ = root.class_list().add_1("glass");
    }

    // A scheduled bank sync wrote new transactions: reload every screen.
    let on_sync = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        revision.try_update(|n| *n += 1);
    });
    api::listen("bank-sync", &on_sync);
    on_sync.forget();
    // The forecast model finished a background run: reload for its forecasts.
    let on_forecast = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        revision.try_update(|n| *n += 1);
    });
    api::listen("daily-forecast", &on_forecast);
    on_forecast.forget();

    // ⌘1–5 switch sections, ⌘R refreshes.
    let handle = window_event_listener(ev::keydown, move |e| {
        if !e.meta_key() || e.shift_key() || e.alt_key() || e.ctrl_key() {
            return;
        }
        let key = e.key();
        if let Some(tab) = key
            .parse::<usize>()
            .ok()
            .and_then(|n| Tab::ALL.get(n.wrapping_sub(1)))
        {
            e.prevent_default();
            app.go(*tab);
        } else if key == "r" {
            e.prevent_default();
            app.refresh();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="app-surface h-full overflow-y-auto font-sans text-fg">
            <div class="fixed inset-x-0 top-0 z-40 h-10" data-tauri-drag-region=""></div>
            <div class="mx-auto w-full min-w-0 max-w-6xl px-4 pb-10 pt-12 sm:px-10 sm:pt-14">
                <Header />
                <Tabs />
                <div role="tabpanel" class="mt-0 min-w-0">
                    {move || match app.tab.get() {
                        Tab::Overview => pages::overview::Overview().into_any(),
                        Tab::Transactions => pages::transactions::Transactions().into_any(),
                        Tab::Spending => view! { <SpendingTabs /> }.into_any(),
                        Tab::Plan => pages::plan::Plan().into_any(),
                        Tab::Manage => view! { <ManageTabs /> }.into_any(),
                    }}
                </div>
            </div>
        </div>
    }
}

#[component]
fn Header() -> impl IntoView {
    let app = state();
    view! {
        <div class="mb-6 flex items-start justify-between gap-4 sm:mb-7 sm:items-center" data-tauri-drag-region="">
            <div class="min-w-0" data-tauri-drag-region="">
                <Eyebrow level=Heading::P>"Finance"</Eyebrow>
                <h1 class="mt-1.5 text-display font-semibold leading-none tracking-tight">"Your money"</h1>
                <p class="mt-2 flex items-center gap-2 text-xs text-fg-muted">
                    <span
                        aria-hidden="true"
                        class=move || cn([
                            "size-1.5 shrink-0 rounded-pill",
                            if app.refresh_failed.get() { "bg-warning" } else { "bg-positive" },
                        ])
                    ></span>
                    <span>{move || app.updated.get().map(|u| format!("Updated {}", format::stamp(&u))).unwrap_or_else(|| "Loading…".into())}</span>
                </p>
            </div>
            <div class="flex shrink-0 items-center gap-2">
                <button
                    type="button"
                    class=cn([button_class(Variant::Secondary, Size::Md).as_str(), "size-9 px-0 sm:size-auto sm:h-10 sm:px-4"])
                    disabled=move || app.busy.get()
                    aria-label=move || if app.busy.get() { "Refreshing finance report" } else { "Refresh finance report" }
                    on:click=move |_| app.refresh()
                >
                    <span class=move || if app.busy.get() { "inline-flex motion-safe:animate-spin" } else { "inline-flex" }>
                        <Lucide icon=Icon::RefreshCw size=15 />
                    </span>
                    <span class="sr-only sm:not-sr-only">{move || if app.busy.get() { "Refreshing…" } else { "Refresh" }}</span>
                </button>
            </div>
        </div>
    }
}

#[component]
fn Tabs() -> impl IntoView {
    let app = state();
    view! {
        <div
            role="tablist"
            aria-label="Finance sections"
            class=cn([TABS_LIST, "glass-bar mb-4 flex w-full gap-0.5 sm:mb-6 sm:inline-flex sm:w-auto sm:gap-1"])
        >
            {Tab::ALL
                .into_iter()
                .enumerate()
                .map(|(i, tab)| {
                    view! {
                        <button
                            type="button"
                            role="tab"
                            aria-selected=move || (app.tab.get() == tab).to_string()
                            data-state=move || if app.tab.get() == tab { "active" } else { "inactive" }
                            title=format!("{} (⌘{})", tab.label(), i + 1)
                            class=cn([TABS_TRIGGER, "min-w-0 grow justify-center truncate px-1.5 text-eyebrow sm:grow-0 sm:px-4 sm:text-xs"])
                            on:click=move |_| app.go(tab)
                        >
                            {tab.label()}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn SpendingTabs() -> impl IntoView {
    let app = state();
    view! {
        <div class="mb-4 max-w-full overflow-x-auto sm:mb-5">
            <SegmentedControl
                label="Spending workspace"
                class="w-max"
                value=Signal::derive(move || app.spending.get().label().to_owned())
                on_change=Callback::new(move |v: String| {
                    if let Some(s) = SpendingSection::ALL.into_iter().find(|s| s.label() == v) {
                        app.spending.set(s);
                    }
                })
                items=SpendingSection::ALL.iter().map(|s| (s.label().to_owned(), s.label().to_owned())).collect()
            />
        </div>
        {move || match app.spending.get() {
            SpendingSection::PlanPatterns => pages::spending::PlanPatterns().into_any(),
            SpendingSection::Organise => pages::spending::Organise().into_any(),
            SpendingSection::History => pages::spending::History().into_any(),
        }}
    }
}

#[component]
fn ManageTabs() -> impl IntoView {
    let app = state();
    view! {
        <div class="mb-4 max-w-full overflow-x-auto sm:mb-5">
            <SegmentedControl
                label="Manage finance"
                class="w-max"
                value=Signal::derive(move || app.manage.get().label().to_owned())
                on_change=Callback::new(move |v: String| {
                    if let Some(s) = ManageSection::ALL.into_iter().find(|s| s.label() == v) {
                        app.manage.set(s);
                    }
                })
                items=ManageSection::ALL.iter().map(|s| (s.label().to_owned(), s.label().to_owned())).collect()
            />
        </div>
        {move || match app.manage.get() {
            ManageSection::Banks => pages::manage::Banks().into_any(),
            ManageSection::CloseMonth => pages::manage::CloseMonth().into_any(),
            ManageSection::Import => pages::manage::Import().into_any(),
            ManageSection::Status => pages::manage::Status().into_any(),
            ManageSection::Settings => pages::manage::Settings().into_any(),
        }}
    }
}

fn main() {
    console_error_panic_hook::set_once();
    api::keep_details_open();
    leptos::mount::mount_to_body(App);
}
