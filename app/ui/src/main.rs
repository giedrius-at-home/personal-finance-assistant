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
mod plots;

use finance_types::report::Dashboard;
use leptos::ev;
use leptos::prelude::*;
use leptos::tachys::reactive_graph::OwnedView;

use crate::api::{Loaded, NoArgs, load};
use crate::ds::arrange::{
    DropTarget, TAB_ALERT, TAB_EDITOR, css_escaped, drop_index, drop_move, focus_on,
};
use crate::ds::patterns::{ERROR_LINE, PHONE_TAB_BAR, TAB_ROW, TAB_SCROLL};
use crate::ds::{
    Button, Eyebrow, Heading, Icon, Lucide, SegmentedControl, Size, TABS_LIST, TABS_TRIGGER,
    TabChip, Variant, button_class, cn,
};
use crate::layout::edit::{self, PageRef, TabView};
use crate::layout::{LayoutState, PageView};

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
        if must_leave(&shown, hidden, prev.as_ref()) {
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

    // ⌘1–9 pick a tab of the bar by position, ⌘R refreshes, Esc leaves Edit layout.
    let handle = window_event_listener(ev::keydown, move |e| {
        if e.key() == "Escape" {
            if layout.editing().get_untracked() && !escape_taken() {
                layout.set_editing(false);
            }
            return;
        }
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
            // The window's drag strip: only in the app window, or it would take taps on a phone's tab bar.
            <div class="fixed inset-x-0 top-0 z-40 hidden h-10 [.glass_&]:block" data-tauri-drag-region=""></div>
            <div class="w-full min-w-0 px-4 pb-[calc(3rem+env(safe-area-inset-bottom))] pt-[calc(3rem+env(safe-area-inset-top))] sm:px-8 sm:pt-[calc(3.5rem+env(safe-area-inset-top))] xl:px-12">
                <Header />
                <Tabs tabs visible />
                {move || (layout.editing().get() && layout.save_failed().get()).then(|| view! {
                    <p role="alert" class=ERROR_LINE>"Could not save the layout. Your changes are kept until you close the app."</p>
                })}
                <div role="tabpanel" class="page-stack mt-0 min-w-0">
                    {move || {
                        owned(|| match app.own_tab.get() {
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
                        })
                    }}
                </div>
            </div>
            <pages::brief_popup::BriefPopup />
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
fn Tabs(tabs: Memo<Vec<TabView>>, visible: Memo<Vec<TabView>>) -> impl IntoView {
    let app = state();
    let layout = expect_context::<LayoutState>();
    let editing = layout.editing();
    let bar = move || {
        visible
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, tab)| {
                let id = StoredValue::new(tab.id);
                let on = move || id.with_value(|id| app.shown_id() == *id);
                let title = if i < 9 {
                    format!("{} (⌘{})", tab.name, i + 1)
                } else {
                    tab.name.clone()
                };
                view! {
                    <button
                        type="button"
                        role="tab"
                        aria-selected=move || on().to_string()
                        data-state=move || if on() { "active" } else { "inactive" }
                        title=title
                        class=cn([TABS_TRIGGER, "shrink-0 justify-center px-3 sm:px-4"])
                        on:click=move |e| {
                            id.with_value(|id| app.go_id(id));
                            // On a phone the bar scrolls: bring the chosen tab and its neighbours into view.
                            let options = web_sys::ScrollIntoViewOptions::new();
                            options.set_inline(web_sys::ScrollLogicalPosition::Center);
                            options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                            event_target::<web_sys::Element>(&e)
                                .scroll_into_view_with_scroll_into_view_options(&options);
                        }
                    >
                        {tab.name}
                    </button>
                }
            })
            .collect_view()
    };
    view! {
        <div class=TAB_ROW>
        {move || if editing.get() {
            view! { <TabEditor tabs /> }.into_any()
        } else {
            view! {
                <div
                    role="tablist"
                    aria-label="Finance sections"
                    class=cn([TABS_LIST, PHONE_TAB_BAR])
                >
                    {bar}
                </div>
            }
            .into_any()
        }}
        // Hidden while nothing could be saved: the layout is loading or its read failed.
        {move || layout.editable().then(|| view! {
            <div class=move || if editing.get() { "mb-3 mt-2 flex justify-end sm:m-0" } else { "mb-3 flex justify-end sm:m-0" }>
                <Button
                    variant=Variant::Secondary
                    size=Size::Sm
                    on_click=Callback::new(move |_| layout.set_editing(!editing.get_untracked()))
                >
                    {move || if editing.get() { "Done" } else { "Edit layout" }}
                </Button>
            </div>
        })}
        </div>
    }
}

/// The tab bar in Edit layout: every tab with its controls, hidden ones last,
/// and "Add a tab". A tab is dragged by its name; a tab or a panel is dropped on one.
#[component]
fn TabEditor(tabs: Memo<Vec<TabView>>) -> impl IntoView {
    let app = state();
    let layout = expect_context::<LayoutState>();
    let listed = Memo::new(move |_| tabs.with(|t| editor_tabs(t)));
    // The own tab whose name is being edited, its text, and a refused name's message.
    let renaming = RwSignal::new(None::<String>);
    let draft = RwSignal::new(String::new());
    let refused = RwSignal::new(None::<String>);
    // A tab deleted (or dropped by "Reset everything") mid-rename ends the rename.
    Effect::new(move |_| {
        let gone = renaming.with(|r| {
            r.as_ref()
                .is_some_and(|r| tabs.with(|t| t.iter().all(|t| &t.id != r)))
        });
        if gone {
            renaming.set(None);
            refused.set(None);
        }
    });
    // The tab being dragged, and the tab under the pointer (with "after").
    let dragging = RwSignal::new(None::<String>);
    let over = RwSignal::new(None::<(String, bool)>);
    let refocus = move |edited: TabEdited, id: &str| {
        focus_on(untrack(|| tab_focus_after(edited, id, &layout.tabs())));
    };
    let chip = move |t: TabView| {
        let id = StoredValue::new(t.id.clone());
        let name = StoredValue::new(t.name.clone());
        let hidden = t.hidden;
        // Where a panel dropped on this tab goes: the page it shows.
        let to = move || {
            id.with_value(|id| PageRef {
                tab: id.clone(),
                tab_name: name.get_value(),
                page: tab_page(id, app.spending.get_untracked(), app.manage.get_untracked()),
            })
        };
        let ends = Memo::new(move |_| tabs.with(|v| id.with_value(|id| tab_moves(v, id))));
        let moved = move |to: Option<usize>, edited: TabEdited| {
            if let Some(to) = to {
                id.with_value(|id| {
                    layout.edit(|l| edit::tab_moved(l, id, to));
                    refocus(edited, id);
                });
            }
        };
        let drop = DropTarget {
            accepts: Signal::derive(move || {
                if layout.carrying() {
                    layout.may_send(&to())
                } else {
                    !hidden
                        && dragging
                            .with(|d| d.as_ref().is_some_and(|d| id.with_value(|id| d != id)))
                }
            }),
            over: Signal::derive(move || {
                over.with(|o| o.as_ref().is_some_and(|(o, _)| id.with_value(|id| o == id)))
            }),
            on_over: Callback::new(move |after: bool| {
                let here = Some((id.get_value(), after));
                if over.with_untracked(|o| *o != here) {
                    over.set(here);
                }
            }),
            on_leave: Callback::new(move |()| {
                if over.with_untracked(|o| {
                    o.as_ref().is_some_and(|(o, _)| id.with_value(|id| o == id))
                }) {
                    over.set(None);
                }
            }),
            on_drop: Callback::new(move |()| {
                if untrack(|| layout.carrying()) {
                    layout.send(&to());
                } else if let (Some(from), Some((_, after))) =
                    (dragging.get_untracked(), over.get_untracked())
                    && let Some(to) =
                        id.with_value(|id| tabs.with_untracked(|v| tab_drop(v, &from, id, after)))
                {
                    layout.edit(|l| edit::tab_moved(l, &from, to));
                }
                dragging.set(None);
                over.set(None);
            }),
        };
        view! {
            <TabChip
                id=t.id.clone()
                name=t.name.clone()
                active=Signal::derive(move || id.with_value(|id| app.shown_id() == *id))
                own=!t.built_in
                hide=(t.built_in && t.id != Tab::Manage.id()).then_some(hidden)
                first=Signal::derive(move || ends.get().0.is_none())
                last=Signal::derive(move || ends.get().1.is_none())
                renaming=Signal::derive(move || renaming.with(|r| id.with_value(|id| r.as_ref() == Some(id))))
                draft
                refused=Signal::derive(move || refused.get().filter(|_| renaming.with(|r| id.with_value(|id| r.as_ref() == Some(id)))))
                drop
                on_grab=Callback::new(move |on: bool| {
                    dragging.set(on.then(|| id.get_value()));
                    over.set(None);
                })
                on_pick=Callback::new(move |()| id.with_value(|id| app.go_id(id)))
                on_left=Callback::new(move |()| moved(ends.get_untracked().0, TabEdited::Left))
                on_right=Callback::new(move |()| moved(ends.get_untracked().1, TabEdited::Right))
                on_hide=Callback::new(move |h: bool| id.with_value(|id| {
                    layout.edit(|l| edit::tab_hidden_set(l, id, h));
                    refocus(if h { TabEdited::Hidden } else { TabEdited::Shown }, id);
                }))
                on_rename=Callback::new({
                    let name = t.name.clone();
                    move |()| {
                        draft.set(name.clone());
                        refused.set(None);
                        renaming.set(Some(id.get_value()));
                        id.with_value(|id| focus_on(in_tab(id, &format!("Name of {name}"))));
                    }
                })
                on_save=Callback::new(move |()| id.with_value(|id| {
                    // Checked first: a refused name writes (and saves) nothing.
                    match edit::tab_name(&draft.get_untracked()) {
                        Ok(name) => {
                            layout.edit(|l| {
                                let _ = edit::tab_renamed(l, id, &name);
                            });
                            renaming.set(None);
                            refused.set(None);
                            refocus(TabEdited::Named, id);
                        }
                        Err(why) => refused.set(Some(why)),
                    }
                }))
                on_cancel=Callback::new(move |()| {
                    renaming.set(None);
                    refused.set(None);
                    id.with_value(|id| refocus(TabEdited::Named, id));
                })
                on_delete=Callback::new(move |()| id.with_value(|id| {
                    layout.edit(|l| edit::tab_deleted(l, id));
                    refocus(TabEdited::Deleted, id);
                }))
            />
        }
    };
    let add = move |_| {
        // Checked first: a refused add writes (and saves) nothing.
        if let Err(why) = untrack(|| edit::tab_room(&layout.tabs())) {
            refused.set(Some(why));
            return;
        }
        // The name is fixed and there is room, so this is Ok.
        let mut id = String::new();
        layout.edit(|l| id = edit::tab_added(l, "New tab").unwrap_or_default());
        app.go_own(&id);
        draft.set("New tab".into());
        refused.set(None);
        renaming.set(Some(id.clone()));
        refocus(TabEdited::Added, &id);
    };
    view! {
        <div role="group" aria-label="Edit tabs" class=TAB_EDITOR>
            // Keyed, so a chip (and an open Delete question) outlives edits of other tabs;
            // what the chip is built from is in the key.
            <For
                each=move || listed.get()
                key=|t| (t.id.clone(), t.name.clone(), t.hidden)
                children=chip
            />
            <Button variant=Variant::Ghost size=Size::IconSm label="Add a tab" on_click=Callback::new(add)>
                <Lucide icon=Icon::Plus size=14 />
            </Button>
            // A refused add; a refused name shows in its chip.
            {move || refused.get().filter(|_| renaming.with(Option::is_none)).map(|why| view! {
                <p role="alert" class=cn([ERROR_LINE, TAB_ALERT])>{why}</p>
            })}
        </div>
    }
}

/// Esc belongs to something else: a popup is open or focus is in a text field.
fn escape_taken() -> bool {
    let doc = document();
    let popup = doc
        .query_selector("[role=dialog], [role=alertdialog]")
        .ok()
        .flatten()
        .is_some();
    let typing = doc.active_element().is_some_and(|el| {
        matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
            || el
                .get_attribute("contenteditable")
                .is_some_and(|v| v != "false")
    });
    popup || typing
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
        {move || owned(|| match app.spending.get() {
            SpendingSection::PlanPatterns => pages::spending::PlanPatterns().into_any(),
            SpendingSection::Organise => pages::spending::Organise().into_any(),
            SpendingSection::History => pages::spending::History().into_any(),
        })}
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
        {move || owned(|| match app.manage.get() {
            ManageSection::Banks => pages::manage::Banks().into_any(),
            ManageSection::CloseMonth => pages::manage::CloseMonth().into_any(),
            ManageSection::Import => pages::manage::Import().into_any(),
            ManageSection::Status => pages::manage::Status().into_any(),
            ManageSection::Settings => pages::manage::Settings().into_any(),
        })}
    }
}

/// What the tab bar shows: every tab that is not hidden, plus a hidden one while it is on screen.
pub fn visible_tabs(tabs: &[TabView], shown: &str) -> Vec<TabView> {
    tabs.iter()
        .filter(|t| !t.hidden || t.id == shown)
        .cloned()
        .collect()
}

/// The shown tab must be left: it is gone (`hidden` None), or it became hidden
/// while it was already shown. `prev` is the last (shown id, hidden) seen.
pub fn must_leave(shown: &str, hidden: Option<bool>, prev: Option<&(String, bool)>) -> bool {
    match hidden {
        None => true,
        Some(h) => h && prev.is_some_and(|(p, was)| p == shown && !was),
    }
}

/// The tab editor's order: the tabs that are not hidden, then the hidden ones.
pub fn editor_tabs(tabs: &[TabView]) -> Vec<TabView> {
    let (shown, hidden): (Vec<_>, Vec<_>) = tabs.iter().cloned().partition(|t| !t.hidden);
    shown.into_iter().chain(hidden).collect()
}

/// The `to` for `tab_moved` of "Move left" and "Move right" of tab `id`: past
/// the nearest tab that is not hidden; None at an end of the bar.
pub fn tab_moves(tabs: &[TabView], id: &str) -> (Option<usize>, Option<usize>) {
    let Some(from) = tabs.iter().position(|t| t.id == id) else {
        return (None, None);
    };
    let left = tabs[..from]
        .iter()
        .rposition(|t| !t.hidden)
        .map(|o| drop_index(from, o, false));
    let right = tabs[from + 1..]
        .iter()
        .position(|t| !t.hidden)
        .map(|o| drop_index(from, from + 1 + o, true));
    (left, right)
}

/// The `to` for `tab_moved` of tab `from` dropped on tab `over` (`after`: on
/// its right half). Decided over the tabs shown in the bar, so hidden tabs keep
/// their places; None when it would stay where it is.
pub fn tab_drop(tabs: &[TabView], from: &str, over: &str, after: bool) -> Option<usize> {
    let shown: Vec<&str> = tabs
        .iter()
        .filter(|t| !t.hidden)
        .map(|t| t.id.as_str())
        .collect();
    let at = |id: &str| shown.iter().position(|s| *s == id);
    let to = drop_move(at(from)?, at(over)?, after)?;
    // In the list without `from`: right after the shown tab before it, or before the first.
    let rest: Vec<&str> = tabs
        .iter()
        .map(|t| t.id.as_str())
        .filter(|t| *t != from)
        .collect();
    let shown_rest: Vec<&str> = shown.into_iter().filter(|s| *s != from).collect();
    let full = |id: &str| rest.iter().position(|t| *t == id);
    match to.checked_sub(1) {
        None => full(shown_rest.first()?),
        Some(p) => full(shown_rest.get(p)?).map(|i| i + 1),
    }
}

/// The page tab `id` shows: for Spending and Manage the selected sub-section.
pub fn tab_page(id: &str, spending: SpendingSection, manage: ManageSection) -> String {
    let page = match id {
        "spending" => match spending {
            SpendingSection::PlanPatterns => "spending.habits",
            SpendingSection::Organise => "spending.organise",
            SpendingSection::History => "spending.history",
        },
        "manage" => match manage {
            ManageSection::Banks => "manage.banks",
            ManageSection::CloseMonth => "manage.close",
            ManageSection::Import => "manage.import",
            ManageSection::Status => "manage.status",
            ManageSection::Settings => "manage.settings",
        },
        _ => id,
    };
    page.into()
}

/// What was just done to a tab, for `tab_focus_after`.
#[derive(Clone, Copy, Debug)]
pub enum TabEdited {
    Left,
    Right,
    Hidden,
    Shown,
    /// Renamed, or the rename was cancelled.
    Named,
    Added,
    Deleted,
}

/// The selector of the control labelled `label` in the chip of tab `id`:
/// names repeat ("New tab"), ids do not.
pub fn in_tab(id: &str, label: &str) -> String {
    let (id, label) = (css_escaped(id), css_escaped(label));
    format!(
        "[data-tab=\"{id}\"] button[aria-label=\"{label}\"], [data-tab=\"{id}\"] input[aria-label=\"{label}\"]"
    )
}

/// The selector of what takes focus after an edit of tab `id`, from the tabs
/// as the edit left them: a control in its chip, or (once it is gone) the tab now shown.
pub fn tab_focus_after(edited: TabEdited, id: &str, tabs: &[TabView]) -> String {
    let Some(name) = tabs.iter().find(|t| t.id == id).map(|t| &t.name) else {
        return "button[aria-current=page]".into();
    };
    let (left, right) = tab_moves(tabs, id);
    let label = match edited {
        TabEdited::Left if left.is_none() => format!("Move {name} right"),
        TabEdited::Left => format!("Move {name} left"),
        TabEdited::Right if right.is_none() => format!("Move {name} left"),
        TabEdited::Right => format!("Move {name} right"),
        TabEdited::Hidden => format!("Show {name}"),
        TabEdited::Shown => format!("Hide {name}"),
        TabEdited::Named => format!("Rename {name}"),
        TabEdited::Added => format!("Name of {name}"),
        TabEdited::Deleted => return "button[aria-current=page]".into(),
    };
    in_tab(id, &label)
}

/// Renders `f` under its own owner: the contexts a page provides (Overview's
/// `PanelState`, …) end with it instead of reaching the next page.
fn owned<V>(f: impl FnOnce() -> V) -> OwnedView<V> {
    let owner = Owner::new();
    let view = owner.with(f);
    OwnedView::new_with_owner(view, owner)
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

    #[test]
    fn in_tab_escapes_backslash_and_quote() {
        let s = in_tab(r#"own-1"#, r#"Rename a\b"c"#);
        assert!(s.contains(r#"button[aria-label="Rename a\\b\"c"]"#), "{s}");
    }

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
    fn following_a_link_to_a_hidden_tab_stays_there() {
        let prev = ("overview".to_string(), false);
        assert!(!must_leave("plan", Some(true), Some(&prev)));
    }

    #[test]
    fn hiding_the_shown_tab_moves_away() {
        let prev = ("plan".to_string(), false);
        assert!(must_leave("plan", Some(true), Some(&prev)));
        assert!(must_leave("own-1", None, Some(&prev)));
    }

    fn own(id: &str, name: &str) -> TabView {
        TabView {
            id: id.into(),
            name: name.into(),
            built_in: false,
            hidden: false,
        }
    }

    #[test]
    fn hidden_tabs_come_last_in_the_editor() {
        let tabs = [tab("a", false), tab("b", true), tab("c", false)];
        assert_eq!(ids(&editor_tabs(&tabs)), ["a", "c", "b"]);
    }

    #[test]
    fn move_left_and_right_skip_hidden_tabs() {
        // Full order a, b(hidden), c, d.
        let tabs = [
            tab("a", false),
            tab("b", true),
            tab("c", false),
            tab("d", false),
        ];
        assert_eq!(tab_moves(&tabs, "a"), (None, Some(2)));
        // c left: before a; c right: after d.
        assert_eq!(tab_moves(&tabs, "c"), (Some(0), Some(3)));
        assert_eq!(tab_moves(&tabs, "d"), (Some(2), None));
        assert_eq!(tab_moves(&tabs, "gone"), (None, None));
    }

    #[test]
    fn a_tab_dropped_on_a_tab_lands_by_the_side_it_was_dropped_on() {
        let tabs = [tab("a", false), tab("b", false), tab("c", false)];
        assert_eq!(tab_drop(&tabs, "c", "a", false), Some(0));
        assert_eq!(tab_drop(&tabs, "c", "a", true), Some(1));
        assert_eq!(tab_drop(&tabs, "a", "c", true), Some(2));
        assert_eq!(tab_drop(&tabs, "a", "b", false), None);
        assert_eq!(tab_drop(&tabs, "b", "a", true), None);
        assert_eq!(tab_drop(&tabs, "a", "gone", true), None);
    }

    #[test]
    fn a_drop_next_to_a_hidden_tab_is_decided_by_the_tabs_shown() {
        // a, b(hidden), c: the bar shows a c.
        let tabs = [tab("a", false), tab("b", true), tab("c", false)];
        assert_eq!(tab_drop(&tabs, "c", "a", true), None);
        assert_eq!(tab_drop(&tabs, "a", "c", false), None);
        assert_eq!(tab_drop(&tabs, "c", "a", false), Some(0));
        // a after c: the end, so b keeps its place before c.
        assert_eq!(tab_drop(&tabs, "a", "c", true), Some(2));
        // A hidden tab is neither dragged nor dropped on.
        assert_eq!(tab_drop(&tabs, "b", "a", false), None);
    }

    #[test]
    fn a_panel_dropped_on_a_tab_goes_to_the_page_it_shows() {
        let (s, m) = (SpendingSection::History, ManageSection::CloseMonth);
        assert_eq!(tab_page("spending", s, m), "spending.history");
        assert_eq!(tab_page("manage", s, m), "manage.close");
        assert_eq!(tab_page("plan", s, m), "plan");
        assert_eq!(tab_page("own-2", s, m), "own-2");
    }

    #[test]
    fn focus_after_a_tab_edit() {
        let tabs = [
            own("own-1", "Trips"),
            tab("plan", false),
            tab("wishes", true),
        ];
        let f = |e, id| tab_focus_after(e, id, &tabs);
        assert_eq!(
            f(TabEdited::Left, "own-1"),
            in_tab("own-1", "Move Trips right")
        );
        assert_eq!(
            f(TabEdited::Right, "own-1"),
            in_tab("own-1", "Move Trips right")
        );
        // plan is the last tab shown: hidden wishes does not count.
        assert_eq!(
            f(TabEdited::Right, "plan"),
            in_tab("plan", "Move plan left")
        );
        assert_eq!(f(TabEdited::Left, "plan"), in_tab("plan", "Move plan left"));
        assert_eq!(
            f(TabEdited::Hidden, "wishes"),
            in_tab("wishes", "Show wishes")
        );
        assert_eq!(f(TabEdited::Shown, "plan"), in_tab("plan", "Hide plan"));
        assert_eq!(
            f(TabEdited::Named, "own-1"),
            in_tab("own-1", "Rename Trips")
        );
        assert_eq!(
            f(TabEdited::Added, "own-1"),
            in_tab("own-1", "Name of Trips")
        );
        assert_eq!(f(TabEdited::Deleted, "own-2"), "button[aria-current=page]");
    }

    #[test]
    fn focus_finds_the_edited_tab_when_names_repeat() {
        let tabs = [
            own("own-1", "New tab"),
            own("own-2", "New tab"),
            tab("manage", false),
        ];
        let got = tab_focus_after(TabEdited::Right, "own-2", &tabs);
        assert_ne!(got, tab_focus_after(TabEdited::Right, "own-1", &tabs));
        assert!(got.starts_with("[data-tab=\"own-2\"] button[aria-label=\"Move New tab right\"]"));
    }

    #[test]
    fn tab_ids_round_trip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_id(t.id()), Some(t));
        }
        assert_eq!(Tab::from_id("own-1"), None);
    }
}
