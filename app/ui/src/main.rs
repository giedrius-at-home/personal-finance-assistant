//! Finance UI: the app shell, shared state and navigation. The layout and
//! look follow the owner's web finance UI (personal-site `integrated-finance.tsx`).

mod api;
// The whole design system is ported so screens can follow the site; not
// every screen uses every part.
#[allow(dead_code)]
mod ds;
mod format;
mod layout;
mod pages;

use finance_types::report::Dashboard;
use leptos::ev;
use leptos::prelude::*;
use leptos::tachys::reactive_graph::OwnedView;

use crate::api::{Loaded, NoArgs, load};
use crate::ds::patterns::TAB_SCROLL;
use crate::ds::{
    Eyebrow, Heading, Icon, Lucide, SegmentedControl, Size, TABS_LIST, TABS_TRIGGER, Variant,
    button_class, cn,
};
use crate::layout::PageView;
use crate::layout::edit::TabView;

/// The seven primary sections (the site's top tabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Transactions,
    Spending,
    Plan,
    Wishes,
    Events,
    Manage,
}

impl Tab {
    const ALL: [Self; 7] = [
        Self::Overview,
        Self::Transactions,
        Self::Spending,
        Self::Plan,
        Self::Wishes,
        Self::Events,
        Self::Manage,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Transactions => "transactions",
            Self::Spending => "spending",
            Self::Plan => "plan",
            Self::Wishes => "wishes",
            Self::Events => "events",
            Self::Manage => "manage",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
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
            Self::PlanPatterns => "Habits & goals",
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
    /// Some(id): an own tab is shown instead of `tab`.
    pub own_tab: RwSignal<Option<String>>,
    pub busy: RwSignal<bool>,
    /// The dashboard report every screen reads; reloaded on `revision`.
    pub report: Loaded<Dashboard>,
}

impl AppState {
    pub fn go(&self, tab: Tab) {
        self.own_tab.set(None);
        self.tab.set(tab);
    }

    pub fn go_own(&self, id: &str) {
        self.own_tab.set(Some(id.into()));
    }

    /// The id of what is on screen ("plan", "own-2", …); reactive.
    pub fn shown_id(&self) -> String {
        self.own_tab
            .get()
            .unwrap_or_else(|| self.tab.get().id().into())
    }

    /// Shows the tab `id`, built-in or own.
    fn go_id(&self, id: &str) {
        match Tab::from_id(id) {
            Some(tab) => self.go(tab),
            None => self.go_own(id),
        }
    }

    pub fn go_spending(&self, section: SpendingSection) {
        self.spending.set(section);
        self.go(Tab::Spending);
    }

    pub fn go_manage(&self, section: ManageSection) {
        self.manage.set(section);
        self.go(Tab::Manage);
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
        own_tab: RwSignal::new(None),
        busy: RwSignal::new(false),
        report: load("dashboard", move || {
            revision.track();
            NoArgs {}
        }),
    };
    provide_context(app);
    // Separate from `AppState`: a layout change never bumps `revision`.
    let layout = layout::LayoutState::load();
    provide_context(layout);
    // The default layout while it loads: the seven built-in tabs.
    let tabs = Memo::new(move |_| layout.tabs());
    let visible = Memo::new(move |_| tabs.with(|t| visible_tabs(t, &app.shown_id())));
    // What is on screen was deleted, or hidden by an edit while it was shown.
    Effect::new(move |prev: Option<(String, bool)>| {
        let shown = app.shown_id();
        let hidden = tabs.with(|t| t.iter().find(|v| v.id == shown).map(|v| v.hidden));
        let gone = match hidden {
            None => true,
            Some(h) => h && prev.is_some_and(|(p, was)| p == shown && !was),
        };
        if gone {
            app.go_id(&tabs.with_untracked(|t| fallback_tab(t, &shown)));
        }
        (shown, hidden.unwrap_or(true))
    });
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
    // A suggestion pass stored proposals or saved categories.
    api::listen("ai-suggestions", &on_sync);
    on_sync.forget();
    // The forecast model finished a background run: reload for its forecasts.
    let on_forecast = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        revision.try_update(|n| *n += 1);
    });
    api::listen("daily-forecast", &on_forecast);
    on_forecast.forget();

    // ⌘1–9 pick a tab of the bar by position, ⌘R refreshes.
    let handle = window_event_listener(ev::keydown, move |e| {
        if !e.meta_key() || e.shift_key() || e.alt_key() || e.ctrl_key() {
            return;
        }
        let key = e.key();
        if let Some(tab) = key
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=9).contains(n))
            .and_then(|n| visible.with_untracked(|v| v.get(n - 1).cloned()))
        {
            e.prevent_default();
            app.go_id(&tab.id);
        } else if key == "r" {
            e.prevent_default();
            app.refresh();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="app-surface h-full overflow-y-auto font-sans text-fg">
            <div class="fixed inset-x-0 top-0 z-40 h-10" data-tauri-drag-region=""></div>
            <div class="w-full min-w-0 px-5 pb-12 pt-12 sm:px-8 sm:pt-14 xl:px-12">
                <Header />
                <Tabs visible />
                <div role="tabpanel" class="page-stack mt-0 min-w-0">
                    {move || {
                        // A page's own owner: the contexts it provides (Overview's
                        // `PanelState`, …) end with it instead of reaching the next page.
                        let owner = Owner::new();
                        let page = owner.with(|| match app.own_tab.get() {
                            Some(id) => {
                                let name = tabs.with_untracked(|t| {
                                    t.iter().find(|v| v.id == id).map(|v| v.name.clone())
                                });
                                let name = name.unwrap_or_default();
                                view! { <PageView tab=id.clone() tab_name=name page=id /> }.into_any()
                            }
                            None => match app.tab.get() {
                                Tab::Overview => pages::overview::Overview().into_any(),
                                Tab::Transactions => pages::transactions::Transactions().into_any(),
                                Tab::Spending => view! { <SpendingTabs /> }.into_any(),
                                Tab::Plan => pages::plan::Plan().into_any(),
                                Tab::Wishes => pages::wishes::Wishes().into_any(),
                                Tab::Events => pages::events::Events().into_any(),
                                Tab::Manage => view! { <ManageTabs /> }.into_any(),
                            },
                        });
                        OwnedView::new_with_owner(page, owner)
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
fn Tabs(visible: Memo<Vec<TabView>>) -> impl IntoView {
    let app = state();
    view! {
        <div
            role="tablist"
            aria-label="Finance sections"
            class=cn([TABS_LIST, "glass-bar sticky top-10 z-30 mb-4 flex w-full gap-0.5 sm:mb-6 sm:inline-flex sm:w-auto sm:gap-1"])
        >
            {move || visible
                .get()
                .into_iter()
                .enumerate()
                .map(|(i, tab)| {
                    let id = StoredValue::new(tab.id);
                    let on = move || id.with_value(|id| app.shown_id() == *id);
                    let title = if i < 9 { format!("{} (⌘{})", tab.name, i + 1) } else { tab.name.clone() };
                    view! {
                        <button
                            type="button"
                            role="tab"
                            aria-selected=move || on().to_string()
                            data-state=move || if on() { "active" } else { "inactive" }
                            title=title
                            class=cn([TABS_TRIGGER, "min-w-0 grow justify-center truncate px-1.5 text-eyebrow sm:grow-0 sm:px-4 sm:text-xs"])
                            on:click=move |_| id.with_value(|id| app.go_id(id))
                        >
                            {tab.name}
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
        <div class=TAB_SCROLL>
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
        <div class=TAB_SCROLL>
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

/// What the tab bar shows: every tab that is not hidden, plus a hidden one while it is on screen.
pub fn visible_tabs(tabs: &[TabView], shown: &str) -> Vec<TabView> {
    tabs.iter()
        .filter(|t| !t.hidden || t.id == shown)
        .cloned()
        .collect()
}

/// The tab to show after `gone` disappears (deleted or hidden while on screen): the first visible one.
pub fn fallback_tab(tabs: &[TabView], gone: &str) -> String {
    tabs.iter()
        .find(|t| !t.hidden && t.id != gone)
        // Manage is never hidden, so this is only a guard.
        .map_or_else(|| Tab::Manage.id().into(), |t| t.id.clone())
}

fn main() {
    console_error_panic_hook::set_once();
    api::keep_details_open();
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, hidden: bool) -> TabView {
        TabView {
            id: id.into(),
            name: id.into(),
            built_in: true,
            hidden,
        }
    }

    fn ids(tabs: &[TabView]) -> Vec<&str> {
        tabs.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_hidden_tab_is_not_in_the_bar() {
        let tabs = [
            tab("overview", false),
            tab("plan", true),
            tab("manage", false),
        ];
        assert_eq!(
            ids(&visible_tabs(&tabs, "overview")),
            ["overview", "manage"]
        );
    }

    #[test]
    fn a_hidden_tab_shows_while_it_is_open() {
        let tabs = [
            tab("overview", false),
            tab("plan", true),
            tab("manage", false),
        ];
        assert_eq!(
            ids(&visible_tabs(&tabs, "plan")),
            ["overview", "plan", "manage"]
        );
    }

    #[test]
    fn the_fallback_is_the_first_visible_tab() {
        let tabs = [tab("overview", false), tab("manage", false)];
        assert_eq!(fallback_tab(&tabs, "own-1"), "overview");
    }

    #[test]
    fn the_fallback_skips_hidden_tabs() {
        let tabs = [
            tab("overview", true),
            tab("plan", true),
            tab("manage", false),
        ];
        assert_eq!(fallback_tab(&tabs, "plan"), "manage");
    }

    #[test]
    fn tab_ids_round_trip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_id(t.id()), Some(t));
        }
        assert_eq!(Tab::from_id("own-1"), None);
    }
}
