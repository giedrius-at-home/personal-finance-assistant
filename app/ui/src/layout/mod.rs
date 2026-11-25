//! Panel registry types and the pure logic that turns a saved layout into the
//! panels a page shows.

use finance_types::plot::{PlotSpec, ready_made};
use finance_types::report::Dashboard;
use finance_types::ui::{PageLayout, PanelItem, SavedPlot, UiLayout};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::api::{self, NoArgs};
use crate::ds::arrange::{Edited, drop_move, focus_after, focus_on, in_panel, moves};
use crate::ds::patterns::{ACTIONS, PLOT_UNREADABLE};
use crate::ds::{
    Button, Card, CardHeader, CardTitle, Confirm, DropTarget, EditBar, EmptyState, Gallery,
    PanelAction, PanelFrame, PanelList, Size, Variant,
};
use crate::plots::PlotView;
use crate::plots::builder::{FilterOptions, PlotBuilder, Saving};
use crate::plots::choice::{Closed, NEW_PLOT, copy_title, edit_label, focus_after_builder};
use edit::PageRef;

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

/// A registry panel's title, or a saved plot's.
fn panel_title(l: &UiLayout, panel: &str) -> Option<String> {
    match edit::plot_id_of(panel) {
        Some(id) => l.plots.iter().find(|p| p.id == id).map(|p| p.title.clone()),
        None => registry().find(|d| d.id == panel).map(|d| d.title.into()),
    }
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
/// page) is kept while the registry knows it and it is movable; a saved plot's
/// panel while `plots` has that plot.
pub fn resolve(
    all: &[&PanelDef],
    plots: &[SavedPlot],
    page: &str,
    saved: Option<&PageLayout>,
) -> Vec<PanelItem> {
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
        .filter(|i| match edit::plot_id_of(&i.panel) {
            Some(id) => edit::has_plot(plots, id),
            None => all
                .iter()
                .any(|d| d.id == i.panel && (d.page == page || d.movable)),
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

/// The page's entry. The whole bar is written first when its order is not
/// stored yet or the tab is missing: `page_mut` would append the tab at the end.
fn page_entry<'a>(
    l: &'a mut UiLayout,
    tab: &str,
    tab_name: &str,
    page: &str,
) -> &'a mut PageLayout {
    if !l.tabs_ordered || !l.tabs.iter().any(|t| t.id == tab) {
        edit::materialised(l);
    }
    l.page_mut(tab, tab_name, page)
}

/// Write the page's full resolved list, changed by `f`, into `layout`; nothing
/// when the tab is gone (a fold on a deleted own tab must not recreate it).
fn write_page(
    layout: &mut UiLayout,
    defs: &[&PanelDef],
    (tab, tab_name, page): (&str, &str, &str),
    f: impl FnMut(&mut PanelItem),
) {
    if !edit::tab_exists(layout, tab) {
        return;
    }
    let mut items = resolve(defs, &layout.plots, page, layout.page(page));
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
    editing: RwSignal<bool>,
    save_failed: RwSignal<bool>,
    /// Why the last save was refused.
    save_error: RwSignal<Option<String>>,
    /// A save is under way.
    busy: RwSignal<bool>,
    /// The panel being dragged, with the page it is on.
    carried: RwSignal<Option<(PageRef, String)>>,
}

impl LayoutState {
    /// Reads `ui_layout` once. After a failed read the default shows and changes are not saved.
    pub fn load() -> Self {
        let state = Self {
            layout: RwSignal::new(UiLayout::default()),
            queue: StoredValue::new(SaveQueue::default()),
            load: RwSignal::new(Load::Loading),
            editing: RwSignal::new(false),
            save_failed: RwSignal::new(false),
            save_error: RwSignal::new(None),
            busy: RwSignal::new(false),
            carried: RwSignal::new(None),
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

    /// Edit layout is on.
    pub fn editing(&self) -> ReadSignal<bool> {
        self.editing.read_only()
    }

    pub fn set_editing(&self, on: bool) {
        self.editing.set(on);
    }

    /// The layout was read, so edits can be saved; reactive.
    pub fn editable(&self) -> bool {
        self.load.get().may_save()
    }

    /// Saves a plot over `id`, or as a new one; returns its id (see `edit::plot_saved`).
    pub fn save_plot(
        &self,
        id: Option<String>,
        title: String,
        spec: PlotSpec,
    ) -> Result<String, String> {
        if !self.load.get_untracked().may_change() {
            return Err("The layout is still loading".to_string());
        }
        // Edited on a copy, so a refused save changes and saves nothing.
        let mut next = self.layout.get_untracked();
        let out = edit::plot_saved(&mut next, id.as_deref(), &title, &spec)?;
        self.change(|l| *l = next);
        Ok(out)
    }

    /// Deletes a plot and its panel from every page.
    pub fn delete_plot(&self, id: &str) {
        self.change(|l| edit::plot_deleted(l, id));
    }

    /// Applies `f`, then saves through the queue (same rules as `toggle`).
    pub fn edit(&self, f: impl FnOnce(&mut UiLayout)) {
        self.change(f);
    }

    /// True after a save returned Err; cleared by the next Ok.
    pub fn save_failed(&self) -> ReadSignal<bool> {
        self.save_failed.read_only()
    }

    /// A panel is being dragged; reactive.
    pub fn carrying(&self) -> bool {
        self.carried.with(Option::is_some)
    }

    /// The dragged panel may be dropped on the tab whose shown page is `to`; reactive.
    pub fn may_send(&self, to: &PageRef) -> bool {
        self.carried.with(|c| {
            c.as_ref().is_some_and(|(from, panel)| {
                self.layout
                    .with(|l| edit::sendable(l, &defs(), from, to, panel))
            })
        })
    }

    /// Sends the dragged panel to `to` and ends the drag.
    pub fn send(&self, to: &PageRef) {
        if let Some((from, panel)) = self.carried.get_untracked() {
            self.edit(|l| edit::panel_sent(l, &defs(), &from, to, &panel));
        }
        self.carried.set(None);
    }

    pub fn toggle(&self, tab: &str, tab_name: &str, page: &str, panel: &str) {
        self.change(|l| toggled(l, &defs(), (tab, tab_name, page), panel));
    }

    /// Unfolds `panel`, for a link that points at it; nothing when it is open.
    pub fn open(&self, tab: &str, tab_name: &str, page: &str, panel: &str) {
        let defs = defs();
        let folded = self.layout.with_untracked(|l| {
            resolve(&defs, &l.plots, page, l.page(page))
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

    /// A failed save keeps the state as clicked and sets `save_failed`.
    fn save(self) {
        self.busy.set(true);
        spawn_local(async move {
            loop {
                let layout = self.layout.get_untracked();
                let saved = api::invoke::<UiLayout>("save_ui_layout", &SaveArgs { layout }).await;
                self.save_failed.try_set(saved.is_err());
                self.save_error.try_set(saved.err());
                if !self
                    .queue
                    .try_update_value(SaveQueue::finished)
                    .unwrap_or(false)
                {
                    break;
                }
            }
            self.busy.try_set(false);
        });
    }
}

/// The plot builder is open on a saved plot (its id) or a new one; `opener`
/// is the selector of the control that opened it.
#[derive(Clone)]
struct Building {
    plot: Option<String>,
    opener: String,
}

/// The builder's Save after the saved layout could not be read.
const NOTHING_SAVED: &str =
    "Nothing can be saved right now: your layout could not be read. Restart the app and try again.";

/// What the builder's filters pick from: the categories, groups and accounts
/// the pages already use; reactive, so it follows the report as it loads.
fn filter_options() -> FilterOptions {
    crate::state().report.data.with(|r| {
        let Some(Ok(r)) = r else {
            return FilterOptions::default();
        };
        let mut groups: Vec<String> = r
            .monthly_budgets
            .iter()
            .flat_map(|b| b.plan.groups.iter().map(|g| g.name.clone()))
            .chain(r.custom_spending_groups.iter().cloned())
            .collect();
        groups.sort_by_key(|n| n.to_lowercase());
        groups.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let named = |n: &String| (n.clone(), n.clone());
        FilterOptions {
            categories: r
                .category_catalog
                .entries
                .iter()
                .map(|e| named(&e.name))
                .collect(),
            groups: groups.iter().map(named).collect(),
            accounts: r
                .cash_overview
                .accounts
                .iter()
                .map(|a| (a.account_id.clone(), a.name.clone()))
                .collect(),
            currencies: r
                .time_trends
                .currencies
                .iter()
                .map(|c| c.currency.code.clone())
                .collect(),
        }
    })
}

/// A saved plot's panel body: the plot under its title, or a muted card when
/// this build can't read its spec. Its insight goes to `insight`.
fn plot_body(
    state: LayoutState,
    id: String,
    title: Memo<String>,
    insight: RwSignal<Option<String>>,
) -> AnyView {
    // The raw JSON first, so other layout changes (folds, moves) don't re-parse the spec.
    let raw = Memo::new(move |_| {
        state
            .layout
            .with(|l| l.plots.iter().find(|p| p.id == id).map(|p| p.spec.clone()))
    });
    let spec =
        Memo::new(move |_| raw.with(|r| serde_json::from_value::<PlotSpec>(r.clone()?).ok()));
    let readable = Memo::new(move |_| spec.with(Option::is_some));
    let spec = Signal::derive(move || spec.get().unwrap_or_default());
    (move || {
        let body = if readable.get() {
            view! { <PlotView spec=spec insight=insight title=title /> }.into_any()
        } else {
            view! { <p class=PLOT_UNREADABLE>"This plot can't be drawn in this version"</p> }
                .into_any()
        };
        view! {
            <Card>
                <CardHeader>
                    <CardTitle>{move || title.get()}</CardTitle>
                </CardHeader>
                {body}
            </Card>
        }
    })
    .into_any()
}

/// A page's panels, as its saved layout says, with "Fold all / Open all"; in
/// Edit layout also each panel's controls, "Add a panel" and "Reset this tab".
#[component]
pub fn PageView(
    #[prop(into)] tab: String,
    #[prop(into)] tab_name: String,
    #[prop(into)] page: String,
) -> impl IntoView {
    let state = expect_context::<LayoutState>();
    let report = crate::state().report;
    // An own tab (not a built-in one) is emptied rather than reset.
    let own = crate::Tab::from_id(&tab).is_none();
    let at = StoredValue::new(PageRef {
        tab,
        tab_name,
        page,
    });
    let items = Memo::new(move |_| {
        at.with_value(|a| {
            state
                .layout
                .with(|l| resolve(&defs(), &l.plots, &a.page, l.page(&a.page)))
        })
    });
    let editing = state.editing;
    // Read inside reactive closures, so `shown` may track signals.
    let visible = move || items.with(|v| drawn(v, is_shown));
    // Edit layout lists every item: one with nothing to show is a muted row.
    let listed = move || {
        if editing.get() {
            items.get()
        } else {
            visible()
        }
    };
    let all_folded = move || visible().iter().all(|i| i.folded);
    // Nothing drawn (e.g. a Manage workspace still loading): no "Fold all".
    // A memo, so only the control redraws when the first panel shows.
    let any_drawn = Memo::new(move |_| !visible().is_empty());
    let edit = move |f: &dyn Fn(&mut UiLayout, &[&PanelDef], &PageRef)| {
        at.with_value(|a| state.edit(|l| f(l, &defs(), a)));
    };
    let position = move |id: &str| items.with(|v| v.iter().position(|i| i.panel == id));
    // Focus goes back to a control after the edit redraws, disables or removes the pressed one.
    let refocus = move |edited: Edited, id: &str| {
        let shown = items.get_untracked();
        let (title, after) = state.layout.with_untracked(|l| {
            let after: Vec<(String, String, bool)> = shown
                .iter()
                .filter_map(|i| Some((i.panel.clone(), panel_title(l, &i.panel)?, i.half)))
                .collect();
            (panel_title(l, id).unwrap_or_default(), after)
        });
        let after: Vec<(&str, &str, bool)> = after
            .iter()
            .map(|(i, t, h)| (i.as_str(), t.as_str(), *h))
            .collect();
        focus_on(focus_after(edited, id, &title, &after));
    };
    // The panel of this page being dragged, and the one under the pointer (with "after").
    let dragging = move || {
        state.carried.with(|c| {
            c.as_ref()
                .filter(|(a, _)| at.with_value(|at| a.page == at.page))
                .map(|(_, id)| id.clone())
        })
    };
    let target = RwSignal::new(None::<(String, bool)>);
    // The plot builder, while open.
    let building = RwSignal::new(None::<Building>);
    // Why "Add a panel" refused the last pick.
    let gallery_error = RwSignal::new(None::<String>);
    let panel = move |item: PanelItem| {
        let def = registry().find(|d| d.id == item.panel);
        let plot = edit::plot_id_of(&item.panel).map(str::to_owned);
        if def.is_none() && plot.is_none() {
            return None;
        }
        let id = StoredValue::new(item.panel);
        let is = move |panel: &str| id.with_value(|id| id == panel);
        let title = Memo::new(move |_| {
            id.with_value(|id| state.layout.with(|l| panel_title(l, id)))
                .unwrap_or_default()
        });
        let folded =
            Signal::derive(move || items.with(|v| v.iter().any(|i| is(&i.panel) && i.folded)));
        // A plot's summary is its insight, once its data has loaded.
        let insight = RwSignal::new(None::<String>);
        let summary = Signal::derive(move || match def {
            Some(def) => {
                let f = def.summary?;
                report.data.with(|d| d.as_ref()?.as_ref().ok().and_then(f))
            }
            None => insight.get(),
        });
        let half = Signal::derive(move || items.with(|v| v.iter().any(|i| is(&i.panel) && i.half)));
        let ends = Memo::new(move |_| {
            let len = items.with(Vec::len);
            id.with_value(|id| position(id))
                .map_or((None, None), |p| moves(p, len))
        });
        let moved = move |to: Option<usize>, edited: Edited| {
            if let Some(to) = to {
                id.with_value(|id| {
                    edit(&|l, d, a| edit::panel_moved(l, d, a, id, to));
                    refocus(edited, id);
                });
            }
        };
        let bar = move || {
            editing.get().then(|| {
                view! {
                    <EditBar
                        title=title.get()
                        first=Signal::derive(move || ends.get().0.is_none())
                        last=Signal::derive(move || ends.get().1.is_none())
                        half=half
                        on_grab=Callback::new(move |on: bool| {
                            state.carried.set(on.then(|| (at.get_value(), id.get_value())));
                            target.set(None);
                        })
                        on_up=Callback::new(move |()| moved(ends.get_untracked().0, Edited::Up))
                        on_down=Callback::new(move |()| moved(ends.get_untracked().1, Edited::Down))
                        on_half=Callback::new(move |()| {
                            let to = !half.get_untracked();
                            id.with_value(|id| {
                                edit(&|l, d, a| edit::panel_half_set(l, d, a, id, to));
                                refocus(Edited::Half, id);
                            });
                        })
                        on_remove=Callback::new(move |()| {
                            id.with_value(|id| {
                                let Some(at) = position(id) else { return };
                                edit(&|l, d, a| edit::panel_removed(l, d, a, id));
                                refocus(Edited::Removed(at), id);
                            });
                        })
                    />
                }
            })
        };
        let drop = DropTarget {
            accepts: Signal::derive(move || dragging().is_some()),
            over: Signal::derive(move || {
                target.with(|t| t.as_ref().is_some_and(|(t, _)| is(t)))
                    && !dragging().is_some_and(|d| is(&d))
            }),
            on_over: Callback::new(move |after: bool| {
                if !target.with_untracked(|t| t.as_ref().is_some_and(|(t, a)| is(t) && *a == after))
                {
                    target.set(Some((id.get_value(), after)));
                }
            }),
            on_leave: Callback::new(move |()| {
                if target.with_untracked(|t| t.as_ref().is_some_and(|(t, _)| is(t))) {
                    target.set(None);
                }
            }),
            on_drop: Callback::new(move |()| {
                let (Some(from), Some((_, after))) = (untrack(dragging), target.get_untracked())
                else {
                    return;
                };
                // A drop where it already is writes nothing.
                if let (Some(f), Some(o)) = (position(&from), id.with_value(|id| position(id)))
                    && let Some(to) = drop_move(f, o, after)
                {
                    edit(&|l, d, a| edit::panel_moved(l, d, a, &from, to));
                }
                state.carried.set(None);
                target.set(None);
            }),
        };
        // Only Edit layout makes a panel quiet, so outside it a frame is never redrawn.
        let quiet = Memo::new(move |_| editing.get() && !id.with_value(|id| is_shown(id)));
        let plot = StoredValue::new(plot);
        // A plot this build can read: only it can be edited.
        let readable = Memo::new(move |_| {
            plot.with_value(|p| {
                p.as_ref().is_some_and(|id| {
                    state.layout.with(|l| {
                        l.plots.iter().any(|p| {
                            p.id == *id
                                && serde_json::from_value::<PlotSpec>(p.spec.clone()).is_ok()
                        })
                    })
                })
            })
        });
        // A plot's "Edit plot", in and out of Edit layout, while the layout can be saved.
        let editable_plot = Signal::derive(move || state.editable() && readable.get());
        let action = move || {
            let plot = plot.get_value().filter(|_| editable_plot.get())?;
            let aria = edit_label(&title.get());
            let opener = id.with_value(|panel| in_panel(panel, &aria));
            Some(view! {
                <PanelAction
                    text="Edit plot"
                    aria=aria
                    on_click=Callback::new(move |()| {
                        building.set(Some(Building { plot: Some(plot.clone()), opener: opener.clone() }));
                    })
                />
            })
        };
        Some(move || {
            view! {
                <PanelFrame
                    panel=id.get_value()
                    title=title
                    summary=summary
                    folded=folded
                    on_toggle=Callback::new(move |()| {
                        id.with_value(|id| at.with_value(|a| state.toggle(&a.tab, &a.tab_name, &a.page, id)))
                    })
                    half=half
                    editing=editing
                    edit=bar
                    action=action
                    has_action=editable_plot
                    drop=drop
                    quiet=quiet.get()
                >
                    {match def {
                        Some(def) => (def.render)(),
                        None => plot_body(state, plot.get_value().unwrap_or_default(), title, insight),
                    }}
                </PanelFrame>
            }
        })
    };
    let addable = Signal::derive(move || {
        at.with_value(|a| state.layout.with(|l| edit::addable(l, &defs(), a)))
    });
    let own_plots = Signal::derive(move || {
        at.with_value(|a| state.layout.with(|l| edit::own_plots(l, &defs(), a)))
    });
    let ready_plots = Signal::derive(move || {
        if state.editable() {
            ready_made()
                .into_iter()
                .map(|(name, _)| name.to_owned())
                .collect()
        } else {
            Vec::new()
        }
    });
    let builder = move || {
        let b = building.get()?;
        let saved = b.plot.as_ref().and_then(|id| {
            state.layout.with_untracked(|l| {
                l.plots.iter().find(|p| p.id == *id).map(|p| {
                    let spec =
                        serde_json::from_value::<PlotSpec>(p.spec.clone()).unwrap_or_default();
                    (p.title.clone(), spec)
                })
            })
        });
        let existing = saved.is_some();
        let options = Memo::new(move |_| filter_options());
        let (name, spec) = saved.unwrap_or_default();
        let heading = if existing {
            name.clone()
        } else {
            NEW_PLOT.to_owned()
        };
        let (plot, opener) = (StoredValue::new(b.plot), StoredValue::new(b.opener));
        // Closed after the callback that closes it has returned.
        let close = move |closed: Closed| {
            set_timeout(move || building.set(None), std::time::Duration::ZERO);
            let editing = editing.get_untracked();
            opener.with_value(|o| focus_on(focus_after_builder(&closed, o, editing)));
        };
        // A save waits for the backend: it closes once written; refused, the
        // layout goes back to how it was and the builder says why.
        let waiting = RwSignal::new(None::<(Closed, UiLayout)>);
        let refused = RwSignal::new(None::<String>);
        Effect::new(move |_| {
            if state.busy.get() || waiting.with(Option::is_none) {
                return;
            }
            let Some((closed, before)) = waiting.try_update(Option::take).flatten() else {
                return;
            };
            match state.save_error.get_untracked() {
                None => close(closed),
                Some(why) => {
                    state.change(|l| *l = before);
                    refused.set(Some(format!("Not saved: {why}")));
                }
            }
        });
        Some(view! {
            <PlotBuilder
                heading=heading
                name=name
                spec=spec
                existing=existing
                filter_options=Signal::from(options)
                save_refused=refused
                on_save=Callback::new(move |(how, title, spec): (Saving, String, PlotSpec)| {
                    if !state.load.get_untracked().may_save() {
                        return Err(NOTHING_SAVED.to_owned());
                    }
                    if waiting.with_untracked(Option::is_some) {
                        return Ok(());
                    }
                    refused.set(None);
                    let before = state.layout.get_untracked();
                    let over = match how {
                        Saving::Over => plot.get_value(),
                        Saving::New => None,
                    };
                    let id = state.save_plot(over.clone(), title.clone(), spec)?;
                    let panel = edit::plot_panel_id(&id);
                    if over.is_none() {
                        edit(&|l, d, a| edit::panel_added(l, d, a, &panel));
                    }
                    waiting.set(Some((Closed::Saved { panel, title: title.trim().to_owned() }, before)));
                    Ok(())
                })
                on_delete=Callback::new(move |()| {
                    if let Some(id) = plot.get_value() {
                        state.delete_plot(&id);
                    }
                    close(Closed::Deleted);
                })
                on_close=Callback::new(move |()| close(Closed::Cancelled))
            />
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
                                at.with_value(|a| state.set_all(&a.tab, &a.tab_name, &a.page, folded));
                            })
                        >
                            {move || if all_folded() { "Open all" } else { "Fold all" }}
                        </Button>
                    </div>
                })}
                <PanelList>
                    <For each=listed key=|i| i.panel.clone() children=panel />
                </PanelList>
                {builder}
                {move || (own && !editing.get() && items.with(Vec::is_empty)).then(|| view! {
                    <EmptyState
                        title="Nothing on this tab yet"
                        description="Turn on Edit layout to add panels."
                    />
                })}
                {move || editing.get().then(|| view! {
                    <Gallery
                        items=addable
                        on_add=Callback::new(move |id: String| {
                            gallery_error.set(None);
                            edit(&|l, d, a| edit::panel_added(l, d, a, &id));
                            refocus(Edited::Added, &id);
                        })
                        plots=own_plots
                        on_delete=Callback::new(move |panel: String| {
                            gallery_error.set(None);
                            if let Some(id) = edit::plot_id_of(&panel) {
                                state.delete_plot(id);
                            }
                            focus_on(focus_after_builder(&Closed::Deleted, "", true));
                        })
                        ready=ready_plots
                        on_ready=Callback::new(move |i: usize| {
                            let Some((name, spec)) = ready_made().into_iter().nth(i) else { return };
                            let taken: Vec<String> =
                                state.layout.with_untracked(|l| l.plots.iter().map(|p| p.title.clone()).collect());
                            match state.save_plot(None, copy_title(name, &taken), spec) {
                                Ok(id) => {
                                    gallery_error.set(None);
                                    let panel = edit::plot_panel_id(&id);
                                    edit(&|l, d, a| edit::panel_added(l, d, a, &panel));
                                    refocus(Edited::Added, &panel);
                                }
                                Err(why) => gallery_error.set(Some(why)),
                            }
                        })
                        error=gallery_error
                        new_label=NEW_PLOT
                        on_new=Callback::new(move |()| building.set(Some(Building {
                            plot: None,
                            opener: focus_after_builder(&Closed::Deleted, "", true),
                        })))
                    />
                    <div class=ACTIONS>
                        <Confirm
                            label=if own { "Empty this tab" } else { "Reset this tab" }
                            question=if own {
                                "Remove every panel from this tab?"
                            } else {
                                "Put this tab back as it shipped?"
                            }
                            on_yes=Callback::new(move |()| edit(&|l, _, a| edit::page_reset(l, a)))
                        />
                        <Confirm
                            label="Reset everything"
                            question="Put every tab back as it shipped and delete your own tabs?"
                            on_yes=Callback::new(move |()| {
                                state.edit(edit::all_reset);
                                // An own tab shown is gone: focus the tab shown instead.
                                focus_on("button[aria-current=page]".into());
                            })
                        />
                    </div>
                })}
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
        assert_eq!(defs.len(), 52); // the spec's 48, plus Today, phone notifications, avoidable spending and buying hours
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
        let got = resolve(&refs, &[], "p", None);
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
        assert_eq!(resolve(&refs, &[], "p", Some(&saved)), saved.items);
    }

    #[test]
    fn an_unknown_saved_panel_is_skipped() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false), it("gone.panel", false)], &[]);
        assert_eq!(
            ids(&resolve(&refs, &[], "p", Some(&saved))),
            [("a", false), ("b", true)]
        );
    }

    #[test]
    fn a_removed_panel_stays_removed() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &["b"]);
        assert_eq!(resolve(&refs, &[], "p", Some(&saved)), saved.items);
    }

    #[test]
    fn an_absent_panel_is_never_inserted() {
        let d = [
            def("a", Initial::Open, false),
            def("c", Initial::Absent, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &[]);
        assert_eq!(resolve(&refs, &[], "p", Some(&saved)), saved.items);
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
        let got = resolve(&refs, &[], "p", Some(&saved));
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
            ids(&resolve(&refs, &[], "p", Some(&saved))),
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
        assert_eq!(resolve(&refs, &[], "p", Some(&saved)), saved.items);
    }

    const AT: (&str, &str, &str) = ("overview", "Overview", "p");

    #[test]
    fn a_toggle_writes_the_full_resolved_page_and_twice_restores_it() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(ids(&l.page("p").unwrap().items), [("a", true), ("b", true)]);
        toggled(&mut l, &refs, AT, "a");
        assert_eq!(l.page("p").unwrap().items, resolve(&refs, &[], "p", None));
    }

    #[test]
    fn open_unfolds_a_folded_panel_and_leaves_an_open_one_open() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        opened(&mut l, &refs, AT, "a");
        assert_eq!(l.page("p").unwrap().items, resolve(&refs, &[], "p", None));
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
    fn a_fold_on_a_deleted_own_tab_changes_nothing() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let mut l = UiLayout::default();
        let at = ("own-1", "Trips", "own-1");
        toggled(&mut l, &refs, at, "a");
        opened(&mut l, &refs, at, "b");
        all_set(&mut l, &refs, at, true);
        assert_eq!(l, UiLayout::default());
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
        l.page_mut("overview", "Overview", "p").items = vec![it("a", false), it("b", false)];
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
