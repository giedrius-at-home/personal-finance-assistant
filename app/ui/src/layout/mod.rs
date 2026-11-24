//! Panel registry types and the pure logic that turns a saved layout into the
//! panels a page shows.

use finance_types::report::Dashboard;
use finance_types::ui::{PageLayout, PanelItem, UiLayout};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::api::{self, NoArgs};
use crate::ds::{Button, PanelFrame, PanelList, Size, Variant};

pub mod edit;

/// State of a panel on its home page on first run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initial {
    Open,
    Folded,
    Absent,
}

pub struct PanelDef {
    pub id: &'static str,
    pub title: &'static str,
    /// Home page id.
    pub page: &'static str,
    /// May be added to other tabs.
    pub movable: bool,
    pub initial: Initial,
    pub half: bool,
    pub render: fn() -> AnyView,
    pub summary: Option<fn(&Dashboard) -> Option<String>>,
    /// False: the panel draws nothing (no row, no button) and keeps its saved
    /// place. Runs reactively; may read signals and `use_context`. None: always.
    pub shown: Option<fn() -> bool>,
}

/// The items to draw: a hidden panel draws nothing but stays in the layout.
fn drawn(items: &[PanelItem], shown: impl Fn(&str) -> bool) -> Vec<PanelItem> {
    items.iter().filter(|i| shown(&i.panel)).cloned().collect()
}

/// A panel's `shown` answer; an unknown panel is not drawn anyway.
fn is_shown(id: &str) -> bool {
    registry()
        .find(|d| d.id == id)
        .and_then(|d| d.shown)
        .is_none_or(|f| f())
}

fn item_from_def(d: &PanelDef, folded: bool) -> PanelItem {
    PanelItem {
        panel: d.id.to_string(),
        folded,
        half: d.half,
    }
}

/// The panels page `page` shows, in order. `all` is the registry: its entries
/// whose home is `page` are the page's own; a saved guest (a panel from another
/// page) is kept while the registry knows it and it is movable.
pub fn resolve(all: &[&PanelDef], page: &str, saved: Option<&PageLayout>) -> Vec<PanelItem> {
    let defs: Vec<&PanelDef> = all.iter().copied().filter(|d| d.page == page).collect();
    let Some(saved) = saved else {
        return defs
            .iter()
            .filter(|d| d.initial != Initial::Absent)
            .map(|d| item_from_def(d, d.initial == Initial::Folded))
            .collect();
    };
    let mut items: Vec<PanelItem> = saved
        .items
        .iter()
        .filter(|i| {
            all.iter()
                .any(|d| d.id == i.panel && (d.page == page || d.movable))
        })
        .cloned()
        .collect();
    for (n, d) in defs.iter().enumerate() {
        let known =
            saved.items.iter().any(|i| i.panel == d.id) || saved.removed.iter().any(|r| r == d.id);
        if d.initial == Initial::Absent || known {
            continue;
        }
        let at = defs[..n]
            .iter()
            .rev()
            .find_map(|e| items.iter().position(|i| i.panel == e.id))
            .map_or(0, |p| p + 1);
        items.insert(at, item_from_def(d, true));
    }
    items
}

/// Coalesces saves: at most one in flight, the latest state sent after it.
#[derive(Default)]
pub struct SaveQueue {
    in_flight: bool,
    dirty: bool,
}

impl SaveQueue {
    /// True: start a save now.
    pub fn changed(&mut self) -> bool {
        if self.in_flight {
            self.dirty = true;
            false
        } else {
            self.in_flight = true;
            true
        }
    }

    /// True: start another save with the current state.
    pub fn finished(&mut self) -> bool {
        self.in_flight = std::mem::take(&mut self.dirty);
        self.in_flight
    }
}

/// The page ids of every tab.
#[cfg(test)]
pub const PAGES: [&str; 13] = [
    "overview",
    "transactions",
    "spending.habits",
    "spending.organise",
    "spending.history",
    "plan",
    "wishes",
    "events",
    "manage.banks",
    "manage.close",
    "manage.import",
    "manage.status",
    "manage.settings",
];

/// Every page's panels, in registry order.
pub fn registry() -> impl Iterator<Item = &'static PanelDef> {
    crate::pages::overview::PANELS
        .iter()
        .chain(crate::pages::transactions::PANELS)
        .chain(crate::pages::spending::PANELS)
        .chain(crate::pages::organise::PANELS)
        .chain(crate::pages::plan::PANELS)
        .chain(crate::pages::wishes::PANELS)
        .chain(crate::pages::events::PANELS)
        .chain(crate::pages::manage::panels())
}

/// The registry, as the slice `resolve` and the edits take.
fn defs() -> Vec<&'static PanelDef> {
    registry().collect()
}

/// The page's entry. A tab not saved yet is written in with the whole bar
/// first: `page_mut` would append it, moving it to the end of the bar.
fn page_entry<'a>(
    l: &'a mut UiLayout,
    tab: &str,
    tab_name: &str,
    page: &str,
) -> &'a mut PageLayout {
    if !l.tabs.iter().any(|t| t.id == tab) {
        edit::materialised(l);
    }
    l.page_mut(tab, tab_name, page)
}

/// Write the page's full resolved list, changed by `f`, into `layout`.
fn write_page(
    layout: &mut UiLayout,
    defs: &[&PanelDef],
    (tab, tab_name, page): (&str, &str, &str),
    f: impl FnMut(&mut PanelItem),
) {
    let mut items = resolve(defs, page, layout.page(page));
    items.iter_mut().for_each(f);
    page_entry(layout, tab, tab_name, page).items = items;
}

/// `layout` with one panel's fold state flipped.
fn toggled(layout: &mut UiLayout, defs: &[&PanelDef], at: (&str, &str, &str), panel: &str) {
    write_page(layout, defs, at, |i| {
        if i.panel == panel {
            i.folded = !i.folded;
        }
    });
}

/// `layout` with one panel unfolded.
fn opened(layout: &mut UiLayout, defs: &[&PanelDef], at: (&str, &str, &str), panel: &str) {
    write_page(layout, defs, at, |i| {
        if i.panel == panel {
            i.folded = false;
        }
    });
}

/// `layout` with every panel on the page folded (or opened).
fn all_set(layout: &mut UiLayout, defs: &[&PanelDef], at: (&str, &str, &str), folded: bool) {
    write_page(layout, defs, at, |i| i.folded = folded);
}

#[derive(Serialize)]
struct SaveArgs {
    layout: UiLayout,
}

/// How far the `ui_layout` read got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Load {
    Loading,
    Loaded,
    /// The saved state is unknown: saving would write over it.
    Failed,
}

impl Load {
    /// A click may change the screen.
    fn may_change(self) -> bool {
        self != Self::Loading
    }

    /// A change may be saved.
    fn may_save(self) -> bool {
        self == Self::Loaded
    }
}

/// The saved layout, shared by every page; changes are saved in the background.
#[derive(Clone, Copy)]
pub struct LayoutState {
    layout: RwSignal<UiLayout>,
    queue: StoredValue<SaveQueue>,
    load: RwSignal<Load>,
}

impl LayoutState {
    /// Reads `ui_layout` once. After a failed read the default shows and changes are not saved.
    pub fn load() -> Self {
        let state = Self {
            layout: RwSignal::new(UiLayout::default()),
            queue: StoredValue::new(SaveQueue::default()),
            load: RwSignal::new(Load::Loading),
        };
        spawn_local(async move {
            match api::invoke::<UiLayout>("ui_layout", &NoArgs {}).await {
                Ok(layout) => {
                    state.layout.set(layout);
                    state.load.set(Load::Loaded);
                }
                Err(_) => state.load.set(Load::Failed),
            }
        });
        state
    }

    /// The tab bar's tabs; reactive.
    pub fn tabs(&self) -> Vec<edit::TabView> {
        self.layout.with(edit::tabs)
    }

    pub fn toggle(&self, tab: &str, tab_name: &str, page: &str, panel: &str) {
        self.change(|l| toggled(l, &defs(), (tab, tab_name, page), panel));
    }

    /// Unfolds `panel`, for a link that points at it; nothing when it is open.
    pub fn open(&self, tab: &str, tab_name: &str, page: &str, panel: &str) {
        let defs = defs();
        let folded = self.layout.with_untracked(|l| {
            resolve(&defs, page, l.page(page))
                .iter()
                .any(|i| i.panel == panel && i.folded)
        });
        if folded {
            self.change(|l| opened(l, &defs, (tab, tab_name, page), panel));
        }
    }

    pub fn set_all(&self, tab: &str, tab_name: &str, page: &str, folded: bool) {
        self.change(|l| all_set(l, &defs(), (tab, tab_name, page), folded));
    }

    fn change(&self, f: impl FnOnce(&mut UiLayout)) {
        let load = self.load.get_untracked();
        if !load.may_change() {
            return;
        }
        self.layout.update(f);
        if load.may_save()
            && self
                .queue
                .try_update_value(SaveQueue::changed)
                .unwrap_or(false)
        {
            self.save();
        }
    }

    /// A failed save is ignored on screen; the state stays as clicked.
    fn save(self) {
        spawn_local(async move {
            loop {
                let layout = self.layout.get_untracked();
                let _ = api::invoke::<UiLayout>("save_ui_layout", &SaveArgs { layout }).await;
                if !self
                    .queue
                    .try_update_value(SaveQueue::finished)
                    .unwrap_or(false)
                {
                    break;
                }
            }
        });
    }
}

/// A page's panels, as its saved layout says, with "Fold all / Open all".
#[component]
pub fn PageView(
    #[prop(into)] tab: String,
    #[prop(into)] tab_name: String,
    #[prop(into)] page: String,
) -> impl IntoView {
    let state = expect_context::<LayoutState>();
    let report = crate::state().report;
    let at = StoredValue::new((tab, tab_name, page));
    let items = Memo::new(move |_| {
        at.with_value(|(_, _, page)| state.layout.with(|l| resolve(&defs(), page, l.page(page))))
    });
    // Read inside reactive closures, so `shown` may track signals.
    let visible = move || items.with(|v| drawn(v, is_shown));
    let all_folded = move || visible().iter().all(|i| i.folded);
    // Nothing drawn (e.g. a Manage workspace still loading): no "Fold all".
    // A memo, so only the control redraws when the first panel shows.
    let any_drawn = Memo::new(move |_| !visible().is_empty());
    let panel = move |item: PanelItem| {
        let def = registry().find(|d| d.id == item.panel)?;
        let id = def.id;
        let folded =
            Signal::derive(move || items.with(|v| v.iter().any(|i| i.panel == id && i.folded)));
        let summary = Signal::derive(move || {
            let f = def.summary?;
            report.data.with(|d| d.as_ref()?.as_ref().ok().and_then(f))
        });
        let render = def.render;
        Some(view! {
            <PanelFrame
                title=def.title
                summary=summary
                folded=folded
                on_toggle=Callback::new(move |()| at.with_value(|(t, n, p)| state.toggle(t, n, p, id)))
                half=item.half
            >
                {render()}
            </PanelFrame>
        })
    };
    // Panels wait for the saved layout, so one saved folded never mounts.
    let ready = move || state.load.get() != Load::Loading;
    move || {
        ready().then(|| {
            view! {
                {move || any_drawn.get().then(|| view! {
                    <div class="flex justify-end">
                        <Button
                            variant=Variant::Ghost
                            size=Size::Sm
                            on_click=Callback::new(move |_| {
                                let folded = !all_folded();
                                at.with_value(|(t, n, p)| state.set_all(t, n, p, folded));
                            })
                        >
                            {move || if all_folded() { "Open all" } else { "Fold all" }}
                        </Button>
                    </div>
                })}
                <PanelList>
                    <For each=visible key=|i| (i.panel.clone(), i.half) children=panel />
                </PanelList>
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::IntoAny;

    #[test]
    fn registry_is_sound() {
        let defs: Vec<_> = registry().collect();
        let ids: std::collections::HashSet<_> = defs.iter().map(|d| d.id).collect();
        assert_eq!(ids.len(), defs.len()); // ids are unique
        assert_eq!(defs.len(), 48); // every panel of the spec's tables
        for d in &defs {
            assert!(
                d.id.starts_with(d.page) && PAGES.contains(&d.page),
                "{}",
                d.id
            );
        }
        for page in PAGES {
            assert!(
                defs.iter()
                    .any(|d| d.page == page && d.initial == Initial::Open),
                "{page} opens empty"
            );
        }
    }

    fn def(id: &'static str, initial: Initial, half: bool) -> PanelDef {
        PanelDef {
            id,
            title: id,
            page: "p",
            movable: false,
            initial,
            half,
            render: || ().into_any(),
            summary: None,
            shown: None,
        }
    }

    fn it(panel: &str, folded: bool) -> PanelItem {
        PanelItem {
            panel: panel.into(),
            folded,
            half: false,
        }
    }

    fn page(items: Vec<PanelItem>, removed: &[&str]) -> PageLayout {
        PageLayout {
            id: "p".into(),
            items,
            removed: removed.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn abc() -> [PanelDef; 3] {
        [
            def("a", Initial::Open, false),
            def("b", Initial::Folded, true),
            def("c", Initial::Absent, false),
        ]
    }

    fn ids(items: &[PanelItem]) -> Vec<(&str, bool)> {
        items.iter().map(|i| (i.panel.as_str(), i.folded)).collect()
    }

    #[test]
    fn first_run_uses_the_defaults() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let got = resolve(&refs, "p", None);
        assert_eq!(
            got,
            vec![
                it("a", false),
                PanelItem {
                    panel: "b".into(),
                    folded: true,
                    half: true
                }
            ]
        );
    }

    #[test]
    fn saved_order_and_fold_state_win() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("b", false), it("a", true)], &[]);
        assert_eq!(resolve(&refs, "p", Some(&saved)), saved.items);
    }

    #[test]
    fn an_unknown_saved_panel_is_skipped() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false), it("gone.panel", false)], &[]);
        assert_eq!(
            ids(&resolve(&refs, "p", Some(&saved))),
            [("a", false), ("b", true)]
        );
    }

    #[test]
    fn a_removed_panel_stays_removed() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &["b"]);
        assert_eq!(resolve(&refs, "p", Some(&saved)), saved.items);
    }

    #[test]
    fn an_absent_panel_is_never_inserted() {
        let d = [
            def("a", Initial::Open, false),
            def("c", Initial::Absent, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &[]);
        assert_eq!(resolve(&refs, "p", Some(&saved)), saved.items);
    }

    #[test]
    fn a_new_panel_lands_folded_after_its_registry_neighbour() {
        let d = [
            def("a", Initial::Open, false),
            def("n", Initial::Open, true),
            def("b", Initial::Open, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("b", false), it("a", false)], &[]);
        let got = resolve(&refs, "p", Some(&saved));
        assert_eq!(ids(&got), [("b", false), ("a", false), ("n", true)]);
        assert!(got[2].half);
    }

    #[test]
    fn a_new_first_panel_lands_at_the_start() {
        let d = [
            def("n", Initial::Open, false),
            def("a", Initial::Open, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &[]);
        assert_eq!(
            ids(&resolve(&refs, "p", Some(&saved))),
            [("n", true), ("a", false)]
        );
    }

    #[test]
    fn a_saved_absent_panel_is_kept() {
        let d = [
            def("a", Initial::Open, false),
            def("c", Initial::Absent, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("c", false), it("a", false)], &[]);
        assert_eq!(resolve(&refs, "p", Some(&saved)), saved.items);
    }

    const AT: (&str, &str, &str) = ("t", "T", "p");

    #[test]
    fn a_toggle_writes_the_full_resolved_page_and_twice_restores_it() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(ids(&l.page("p").unwrap().items), [("a", true), ("b", true)]);
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(l.page("p").unwrap().items, resolve(&refs, "p", None));
    }

    #[test]
    fn open_unfolds_a_folded_panel_and_leaves_an_open_one_open() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        opened(&mut l, &refs, AT, "a");
        assert_eq!(l.page("p").unwrap().items, resolve(&refs, "p", None));
        opened(&mut l, &refs, AT, "b");
        assert_eq!(
            ids(&l.page("p").unwrap().items),
            [("a", false), ("b", false)]
        );
    }

    #[test]
    fn set_all_folds_and_opens_every_item() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        all_set(&mut l, &refs, AT, true);
        assert_eq!(ids(&l.page("p").unwrap().items), [("a", true), ("b", true)]);
        all_set(&mut l, &refs, AT, false);
        assert_eq!(
            ids(&l.page("p").unwrap().items),
            [("a", false), ("b", false)]
        );
    }

    #[test]
    fn a_toggle_leaves_other_pages_alone() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        l.page_mut("t", "T", "q").items = vec![it("x", true)];
        let other = l.page("q").cloned();
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(l.page("q").cloned(), other);
    }

    #[test]
    fn saving_a_page_keeps_the_tab_order() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        toggled(&mut l, &refs, ("plan", "Plan", "p"), "a");
        toggled(&mut l, &refs, ("overview", "Overview", "p"), "a");
        let order: Vec<_> = edit::tabs(&l).into_iter().map(|t| t.id).collect();
        let want: Vec<_> = edit::BUILT_IN
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        assert_eq!(order, want);
    }

    #[test]
    fn clicks_wait_for_the_read_and_are_not_saved_after_a_failed_one() {
        let got =
            [Load::Loading, Load::Loaded, Load::Failed].map(|l| (l.may_change(), l.may_save()));
        assert_eq!(got, [(false, false), (true, true), (true, false)]);
    }

    #[test]
    fn a_hidden_panel_is_not_drawn_and_keeps_its_saved_item() {
        let mut d = abc();
        d[1].shown = Some(|| false);
        let refs: Vec<&PanelDef> = d.iter().collect();
        let shown = |id: &str| {
            refs.iter()
                .find(|d| d.id == id)
                .and_then(|d| d.shown)
                .is_none_or(|f| f())
        };
        let mut l = UiLayout::default();
        l.page_mut("t", "T", "p").items = vec![it("a", false), it("b", false)];
        assert_eq!(
            ids(&drawn(&l.page("p").unwrap().items, shown)),
            [("a", false)]
        );
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(
            ids(&l.page("p").unwrap().items),
            [("a", true), ("b", false)]
        );
        all_set(&mut l, &refs, AT, true);
        assert_eq!(ids(&l.page("p").unwrap().items), [("a", true), ("b", true)]);
        assert_eq!(
            ids(&drawn(&l.page("p").unwrap().items, shown)),
            [("a", true)]
        );
    }

    #[test]
    fn queue_coalesces() {
        let mut q = SaveQueue::default();
        assert!(q.changed());
        assert!(!q.changed());
        assert!(!q.changed());
        assert!(q.finished());
        assert!(!q.finished());
        assert!(q.changed());
    }
}
