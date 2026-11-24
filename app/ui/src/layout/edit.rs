//! Pure edits of the saved layout for Edit layout mode. Panel edits take the
//! registry as `defs` and write the page's full resolved list, as `toggled` does.
//! A saved plot's panel (`plot:<id>`) has no def: the layout's `plots` say it exists.

use finance_types::plot::PlotSpec;
use finance_types::ui::{
    MAX_NAME, MAX_PLOTS, MAX_TABS, PageLayout, PanelItem, SavedPlot, TabLayout, UiLayout,
};

use super::{PanelDef, page_entry, resolve, write_page};

/// Where a page lives: its tab id and name, and the page id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRef {
    pub tab: String,
    pub tab_name: String,
    pub page: String,
}

/// One tab of the tab bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabView {
    pub id: String,
    pub name: String,
    pub built_in: bool,
    pub hidden: bool,
}

/// The built-in tabs (id, name), in default order.
pub const BUILT_IN: [(&str, &str); 7] = [
    ("overview", "Overview"),
    ("transactions", "Transactions"),
    ("spending", "Spending"),
    ("plan", "Plan"),
    ("wishes", "Wishes"),
    ("events", "Events"),
    ("manage", "Manage"),
];

/// False for a deleted own tab: `page_mut` would recreate it as a built-in one.
fn exists(l: &UiLayout, at: &PageRef) -> bool {
    tab_exists(l, &at.tab)
}

pub(super) fn tab_exists(l: &UiLayout, tab: &str) -> bool {
    BUILT_IN.iter().any(|(b, _)| *b == tab) || l.tabs.iter().any(|t| t.id == tab)
}

fn def<'a>(defs: &[&'a PanelDef], panel: &str) -> Option<&'a PanelDef> {
    defs.iter().copied().find(|d| d.id == panel)
}

/// The panel id of saved plot `id`.
pub fn plot_panel_id(id: &str) -> String {
    format!("plot:{id}")
}

/// The saved plot a panel id shows; None for a registry panel.
pub fn plot_id_of(panel: &str) -> Option<&str> {
    panel.strip_prefix("plot:")
}

pub(super) fn has_plot(plots: &[SavedPlot], id: &str) -> bool {
    plots.iter().any(|p| p.id == id)
}

/// May go on any page: a movable registry panel, or a saved plot's panel.
fn movable(l: &UiLayout, defs: &[&PanelDef], panel: &str) -> bool {
    match plot_id_of(panel) {
        Some(id) => has_plot(&l.plots, id),
        None => def(defs, panel).is_some_and(|d| d.movable),
    }
}

/// Saves a plot: over `id` (title and spec replaced), or new with the
/// smallest free `p<n>`. Returns its id.
pub fn plot_saved(
    l: &mut UiLayout,
    id: Option<&str>,
    title: &str,
    spec: &PlotSpec,
) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_NAME {
        return Err(format!("A plot name is 1 to {MAX_NAME} characters"));
    }
    let spec = serde_json::to_value(spec).map_err(|e| e.to_string())?;
    if let Some(id) = id {
        let plot = l.plots.iter_mut().find(|p| p.id == id);
        let plot = plot.ok_or("This plot was deleted")?;
        plot.title = title.into();
        plot.spec = spec;
        return Ok(id.into());
    }
    if l.plots.len() >= MAX_PLOTS {
        return Err(format!("At most {MAX_PLOTS} plots"));
    }
    let id = (1..)
        .map(|n| format!("p{n}"))
        .find(|id| !has_plot(&l.plots, id))
        .expect("an unused id");
    l.plots.push(SavedPlot {
        id: id.clone(),
        title: title.into(),
        spec,
    });
    Ok(id)
}

/// The plot gone, and its panel from every page.
pub fn plot_deleted(l: &mut UiLayout, id: &str) {
    l.plots.retain(|p| p.id != id);
    let panel = plot_panel_id(id);
    for page in l.tabs.iter_mut().flat_map(|t| t.pages.iter_mut()) {
        page.items.retain(|i| i.panel != panel);
        page.removed.retain(|r| *r != panel);
    }
}

/// `panel` out of the page's items; listed as removed when the page is its home.
pub fn panel_removed(l: &mut UiLayout, defs: &[&PanelDef], at: &PageRef, panel: &str) {
    if !exists(l, at) {
        return;
    }
    let mut items = resolve(defs, &l.plots, &at.page, l.page(&at.page));
    items.retain(|i| i.panel != panel);
    let home = def(defs, panel).is_some_and(|d| d.page == at.page);
    let page = page_entry(l, &at.tab, &at.tab_name, &at.page);
    page.items = items;
    if home && !page.removed.iter().any(|r| r == panel) {
        page.removed.push(panel.into());
    }
}

/// `panel` appended open and no longer removed; nothing when it is already
/// there, unknown, or a guest that may not move.
pub fn panel_added(l: &mut UiLayout, defs: &[&PanelDef], at: &PageRef, panel: &str) {
    if !exists(l, at) {
        return;
    }
    let half = match plot_id_of(panel) {
        Some(id) => has_plot(&l.plots, id).then_some(false),
        None => def(defs, panel)
            .filter(|d| d.page == at.page || d.movable)
            .map(|d| d.half),
    };
    let Some(half) = half else {
        return;
    };
    let mut items = resolve(defs, &l.plots, &at.page, l.page(&at.page));
    if items.iter().any(|i| i.panel == panel) {
        return;
    }
    items.push(PanelItem {
        panel: panel.into(),
        folded: false,
        half,
    });
    let page = page_entry(l, &at.tab, &at.tab_name, &at.page);
    page.items = items;
    page.removed.retain(|r| r != panel);
}

/// `panel` moved to index `to` of the list without it (clamped).
pub fn panel_moved(l: &mut UiLayout, defs: &[&PanelDef], at: &PageRef, panel: &str, to: usize) {
    if !exists(l, at) {
        return;
    }
    let mut items = resolve(defs, &l.plots, &at.page, l.page(&at.page));
    let Some(from) = items.iter().position(|i| i.panel == panel) else {
        return;
    };
    let item = items.remove(from);
    items.insert(to.min(items.len()), item);
    page_entry(l, &at.tab, &at.tab_name, &at.page).items = items;
}

pub fn panel_half_set(l: &mut UiLayout, defs: &[&PanelDef], at: &PageRef, panel: &str, half: bool) {
    if !exists(l, at) {
        return;
    }
    write_page(l, defs, (&at.tab, &at.tab_name, &at.page), |i| {
        if i.panel == panel {
            i.half = half;
        }
    });
}

/// `panel` may be sent from `from` to `to`: it is movable (or a plot), both tabs exist,
/// `to` is another tab and the panel is not on its page yet.
pub fn sendable(
    l: &UiLayout,
    defs: &[&PanelDef],
    from: &PageRef,
    to: &PageRef,
    panel: &str,
) -> bool {
    movable(l, defs, panel)
        && from.tab != to.tab
        && exists(l, from)
        && exists(l, to)
        // A hidden tab's chip is muted: no drops on it.
        && !tabs(l).iter().any(|t| t.id == to.tab && t.hidden)
        && !resolve(defs, &l.plots, &to.page, l.page(&to.page))
            .iter()
            .any(|i| i.panel == panel)
}

/// `panel` taken off `from` and added to `to`; nothing unless `sendable`.
pub fn panel_sent(l: &mut UiLayout, defs: &[&PanelDef], from: &PageRef, to: &PageRef, panel: &str) {
    if !sendable(l, defs, from, to, panel) {
        return;
    }
    panel_removed(l, defs, from, panel);
    panel_added(l, defs, to, panel);
}

/// What "Add a panel" offers, as (panel id, title): home panels not on the
/// page, then movable panels of other pages not on it, in registry order,
/// then saved plots not on it.
pub fn addable(l: &UiLayout, defs: &[&PanelDef], at: &PageRef) -> Vec<(String, String)> {
    let items = resolve(defs, &l.plots, &at.page, l.page(&at.page));
    let home = defs.iter().filter(|d| d.page == at.page);
    let guests = defs.iter().filter(|d| d.page != at.page && d.movable);
    let plots = l
        .plots
        .iter()
        .map(|p| (plot_panel_id(&p.id), p.title.clone()));
    home.chain(guests)
        .map(|d| (d.id.to_owned(), d.title.to_owned()))
        .chain(plots)
        .filter(|(id, _)| !items.iter().any(|i| i.panel == *id))
        .collect()
}

/// A tab name as saved (trimmed), or why it is refused.
pub fn tab_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(format!("A tab name is 1 to {MAX_NAME} characters"));
    }
    Ok(name.into())
}

/// A new own tab with one empty page; returns its id.
pub fn tab_added(l: &mut UiLayout, name: &str) -> Result<String, String> {
    let name = tab_name(name)?;
    tab_room(&tabs(l))?;
    let id = (1..)
        .map(|n| format!("own-{n}"))
        .find(|id| !l.tabs.iter().any(|t| &t.id == id))
        .expect("an unused id");
    materialised(l);
    l.tabs.push(TabLayout {
        id: id.clone(),
        name,
        built_in: false,
        hidden: false,
        pages: vec![PageLayout {
            id: id.clone(),
            items: vec![],
            removed: vec![],
        }],
    });
    Ok(id)
}

/// Room for one more tab in the bar `tabs`. Counted with the built-ins
/// `tab_moved` may write out, so `validate` holds.
pub fn tab_room(tabs: &[TabView]) -> Result<(), String> {
    if tabs.len() >= MAX_TABS {
        return Err(format!("At most {MAX_TABS} tabs"));
    }
    Ok(())
}

pub fn tab_renamed(l: &mut UiLayout, id: &str, name: &str) -> Result<(), String> {
    let name = tab_name(name)?;
    let tab = l.tabs.iter_mut().find(|t| t.id == id && !t.built_in);
    tab.ok_or("Only your own tabs can be renamed")?.name = name;
    Ok(())
}

pub fn tab_deleted(l: &mut UiLayout, id: &str) {
    l.tabs.retain(|t| t.id != id || t.built_in);
}

/// Writes the full tab list into `l.tabs` in the bar's order, keeping each
/// saved tab's pages, and marks the order as the owner's.
pub(super) fn materialised(l: &mut UiLayout) {
    let order = tabs(l);
    l.tabs_ordered = true;
    let mut saved = std::mem::take(&mut l.tabs);
    l.tabs = order
        .into_iter()
        .map(|v| match saved.iter().position(|t| t.id == v.id) {
            Some(i) => saved.swap_remove(i),
            None => TabLayout {
                id: v.id,
                name: v.name,
                built_in: true,
                hidden: false,
                pages: vec![],
            },
        })
        .collect();
}

pub fn tab_hidden_set(l: &mut UiLayout, id: &str, hidden: bool) {
    if id == "manage" || !BUILT_IN.iter().any(|(b, _)| *b == id) {
        return;
    }
    materialised(l);
    if let Some(t) = l.tabs.iter_mut().find(|t| t.id == id) {
        t.hidden = hidden;
    }
}

/// The tab `id` moved to index `to` of the full ordered list (clamped).
pub fn tab_moved(l: &mut UiLayout, id: &str, to: usize) {
    materialised(l);
    let Some(from) = l.tabs.iter().position(|t| t.id == id) else {
        return;
    };
    let tab = l.tabs.remove(from);
    l.tabs.insert(to.min(l.tabs.len()), tab);
}

/// The page back to its shipped panels.
pub fn page_reset(l: &mut UiLayout, at: &PageRef) {
    if let Some(t) = l.tabs.iter_mut().find(|t| t.id == at.tab) {
        t.pages.retain(|p| p.id != at.page);
    }
}

/// Saved plots stay: only their panels leave the tabs.
pub fn all_reset(l: &mut UiLayout) {
    *l = UiLayout {
        plots: std::mem::take(&mut l.plots),
        ..Default::default()
    };
}

/// The tab bar: saved tabs in saved order (when `tabs_ordered`; else saved
/// built-ins in default order, then own tabs), then each built-in tab that is
/// not saved, inserted after the nearest earlier built-in (default order) present.
pub fn tabs(l: &UiLayout) -> Vec<TabView> {
    let is_built_in = |t: &&TabLayout| BUILT_IN.iter().any(|(b, _)| *b == t.id);
    let saved: Vec<&TabLayout> = if l.tabs_ordered {
        l.tabs.iter().collect()
    } else {
        BUILT_IN
            .iter()
            .filter_map(|(id, _)| l.tabs.iter().find(|t| t.id == *id))
            .chain(l.tabs.iter().filter(|t| !is_built_in(t)))
            .collect()
    };
    let mut out: Vec<TabView> = saved
        .into_iter()
        .map(|t| TabView {
            id: t.id.clone(),
            name: t.name.clone(),
            built_in: t.built_in,
            hidden: t.hidden,
        })
        .collect();
    for (n, (id, name)) in BUILT_IN.iter().enumerate() {
        if out.iter().any(|t| t.id == *id) {
            continue;
        }
        let at = BUILT_IN[..n]
            .iter()
            .rev()
            .find_map(|(e, _)| out.iter().position(|t| t.id == *e))
            .map_or(0, |p| p + 1);
        out.insert(
            at,
            TabView {
                id: (*id).into(),
                name: (*name).into(),
                built_in: true,
                hidden: false,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Initial;
    use leptos::prelude::IntoAny;

    fn def(id: &'static str, page: &'static str, movable: bool) -> PanelDef {
        PanelDef {
            id,
            title: id,
            page,
            movable,
            initial: Initial::Open,
            half: false,
            render: || ().into_any(),
            summary: None,
            shown: None,
        }
    }

    fn reg() -> [PanelDef; 3] {
        [
            def("a", "p1", true),
            def("b", "p1", false),
            def("m", "p2", true),
        ]
    }

    fn at(tab: &str, page: &str) -> PageRef {
        PageRef {
            tab: tab.into(),
            tab_name: tab.to_uppercase(),
            page: page.into(),
        }
    }

    fn p1() -> PageRef {
        at("overview", "p1")
    }

    fn p2() -> PageRef {
        at("plan", "p2")
    }

    fn ids(l: &UiLayout, page: &str) -> Vec<String> {
        l.page(page)
            .map(|p| p.items.iter().map(|i| i.panel.clone()).collect())
            .unwrap_or_default()
    }

    fn removed(l: &UiLayout, page: &str) -> Vec<String> {
        l.page(page).map(|p| p.removed.clone()).unwrap_or_default()
    }

    fn tab_ids(l: &UiLayout) -> Vec<String> {
        tabs(l).into_iter().map(|t| t.id).collect()
    }

    fn defaults() -> Vec<String> {
        BUILT_IN.iter().map(|(id, _)| id.to_string()).collect()
    }

    #[test]
    fn removing_a_home_panel_lists_it_as_removed() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_removed(&mut l, &d, &p1(), "a");
        assert_eq!(ids(&l, "p1"), ["b"]);
        assert_eq!(removed(&l, "p1"), ["a"]);
        let shown = resolve(&d, &l.plots, "p1", l.page("p1"));
        assert!(!shown.iter().any(|i| i.panel == "a"));
    }

    #[test]
    fn removing_a_guest_panel_just_drops_it() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_added(&mut l, &d, &p1(), "m");
        assert_eq!(ids(&l, "p1"), ["a", "b", "m"]);
        panel_removed(&mut l, &d, &p1(), "m");
        assert_eq!(ids(&l, "p1"), ["a", "b"]);
        assert!(removed(&l, "p1").is_empty());
    }

    #[test]
    fn adding_back_clears_removed_and_appends_open() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_removed(&mut l, &d, &p1(), "a");
        panel_added(&mut l, &d, &p1(), "a");
        assert_eq!(ids(&l, "p1"), ["b", "a"]);
        assert!(removed(&l, "p1").is_empty());
        assert!(!l.page("p1").unwrap().items[1].folded);
    }

    #[test]
    fn adding_twice_is_a_no_op() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_added(&mut l, &d, &p1(), "m");
        let once = l.clone();
        panel_added(&mut l, &d, &p1(), "m");
        assert_eq!(l, once);
        panel_added(&mut l, &d, &p1(), "a");
        assert_eq!(l, once);
    }

    #[test]
    fn moving_reorders_and_clamps() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_added(&mut l, &d, &p1(), "m");
        panel_moved(&mut l, &d, &p1(), "m", 0);
        assert_eq!(ids(&l, "p1"), ["m", "a", "b"]);
        panel_moved(&mut l, &d, &p1(), "m", 1);
        assert_eq!(ids(&l, "p1"), ["a", "m", "b"]);
        panel_moved(&mut l, &d, &p1(), "a", 99);
        assert_eq!(ids(&l, "p1"), ["m", "b", "a"]);
    }

    #[test]
    fn half_is_kept_through_a_move() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_half_set(&mut l, &d, &p1(), "b", true);
        panel_moved(&mut l, &d, &p1(), "b", 0);
        let items = &l.page("p1").unwrap().items;
        assert_eq!(items[0].panel, "b");
        assert!(items[0].half);
        assert!(!items[1].half);
    }

    #[test]
    fn sending_a_movable_panel_moves_it() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_sent(&mut l, &d, &p1(), &p2(), "a");
        assert_eq!(ids(&l, "p1"), ["b"]);
        assert_eq!(removed(&l, "p1"), ["a"]);
        assert_eq!(ids(&l, "p2"), ["m", "a"]);
    }

    #[test]
    fn sending_a_fixed_panel_changes_nothing() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_sent(&mut l, &d, &p1(), &p2(), "b");
        assert_eq!(l, UiLayout::default());
    }

    #[test]
    fn a_panel_is_not_sent_to_its_own_tab_or_where_it_already_is() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let p1_other = at("overview", "p2");
        assert!(sendable(&l, &d, &p1(), &p2(), "a"));
        assert!(!sendable(&l, &d, &p1(), &p1_other, "a"));
        assert!(!sendable(&l, &d, &p1(), &p2(), "b"));
        assert!(!sendable(&l, &d, &p2(), &p1(), "a"), "a is on p1 already");
        assert!(!sendable(&l, &d, &p1(), &at("own-9", "own-9"), "a"));
        panel_sent(&mut l, &d, &p2(), &p1(), "a");
        assert_eq!(l, UiLayout::default());
    }

    #[test]
    fn a_panel_is_not_sent_to_a_hidden_tab() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        tab_hidden_set(&mut l, "plan", true);
        let before = l.clone();
        assert!(!sendable(&l, &d, &p1(), &p2(), "a"));
        panel_sent(&mut l, &d, &p1(), &p2(), "a");
        assert_eq!(l, before);
    }

    #[test]
    fn addable_lists_removed_home_panels_then_movable_guests() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let names = |l: &UiLayout, at: &PageRef| -> Vec<String> {
            addable(l, &d, at).into_iter().map(|(id, _)| id).collect()
        };
        assert_eq!(names(&l, &p1()), ["m"]);
        panel_removed(&mut l, &d, &p1(), "b");
        assert_eq!(names(&l, &p1()), ["b", "m"]);
        // b is not movable, so p2 never offers it.
        assert_eq!(names(&l, &p2()), ["a"]);
        let own = at("own-1", "own-1");
        assert_eq!(names(&l, &own), ["a", "m"]);
    }

    #[test]
    fn a_guest_panel_survives_resolve() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_added(&mut l, &d, &p2(), "a");
        let got: Vec<_> = resolve(&d, &l.plots, "p2", l.page("p2"))
            .into_iter()
            .map(|i| i.panel)
            .collect();
        assert_eq!(got, ["m", "a"]);
    }

    #[test]
    fn a_guest_that_left_the_registry_is_dropped() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_added(&mut l, &d, &p2(), "a");
        let without_a: Vec<&PanelDef> = r.iter().filter(|d| d.id != "a").collect();
        let got: Vec<_> = resolve(&without_a, &l.plots, "p2", l.page("p2"))
            .into_iter()
            .map(|i| i.panel)
            .collect();
        assert_eq!(got, ["m"]);
        // A saved guest that is not movable is dropped too.
        l.page_mut("t2", "T2", "p2").items.push(PanelItem {
            panel: "b".into(),
            folded: false,
            half: false,
        });
        assert!(
            !resolve(&d, &l.plots, "p2", l.page("p2"))
                .iter()
                .any(|i| i.panel == "b")
        );
    }

    #[test]
    fn new_tabs_get_the_smallest_free_id() {
        let mut l = UiLayout::default();
        assert_eq!(tab_added(&mut l, "One").unwrap(), "own-1");
        assert_eq!(tab_added(&mut l, "Two").unwrap(), "own-2");
        tab_deleted(&mut l, "own-1");
        assert_eq!(tab_added(&mut l, "Three").unwrap(), "own-1");
        let t = l.tabs.iter().find(|t| t.id == "own-2").unwrap();
        assert!(!t.built_in);
        assert_eq!(t.pages.len(), 1);
        assert_eq!(t.pages[0].id, "own-2");
    }

    #[test]
    fn tab_names_are_trimmed_and_checked() {
        let mut l = UiLayout::default();
        let id = tab_added(&mut l, "  Trips ").unwrap();
        let name = |l: &UiLayout| l.tabs.iter().find(|t| t.id == id).unwrap().name.clone();
        assert_eq!(name(&l), "Trips");
        assert!(tab_added(&mut l, "   ").is_err());
        assert!(tab_added(&mut l, &"x".repeat(41)).is_err());
        assert!(tab_added(&mut l, &"x".repeat(40)).is_ok());
        assert!(tab_added(&mut l, "Plan").is_ok());
        assert!(tab_renamed(&mut l, &id, "   ").is_err());
        assert!(tab_renamed(&mut l, &id, &"x".repeat(41)).is_err());
        tab_renamed(&mut l, &id, " Holidays ").unwrap();
        assert_eq!(name(&l), "Holidays");
    }

    #[test]
    fn the_fifty_first_tab_is_refused() {
        let mut l = UiLayout::default();
        // 7 built-in tabs plus 43 own ones make 50.
        for _ in 0..MAX_TABS - BUILT_IN.len() {
            tab_added(&mut l, "T").unwrap();
        }
        assert_eq!(tabs(&l).len(), MAX_TABS);
        let before = l.clone();
        assert!(tab_added(&mut l, "One more").is_err());
        assert_eq!(l, before);
        tab_moved(&mut l, "plan", 0);
        assert_eq!(l.tabs.len(), MAX_TABS);
        assert_eq!(l.validate(), Ok(()));
    }

    #[test]
    fn built_in_tabs_cannot_be_renamed_or_deleted() {
        let mut l = UiLayout::default();
        tab_moved(&mut l, "plan", 0);
        let before = l.clone();
        assert!(tab_renamed(&mut l, "plan", "Mine").is_err());
        tab_deleted(&mut l, "plan");
        tab_deleted(&mut l, "manage");
        assert_eq!(l, before);
    }

    #[test]
    fn manage_cannot_be_hidden() {
        let mut l = UiLayout::default();
        tab_hidden_set(&mut l, "manage", true);
        assert_eq!(l, UiLayout::default());
        tab_hidden_set(&mut l, "plan", true);
        let hidden: Vec<_> = tabs(&l)
            .into_iter()
            .filter(|t| t.hidden)
            .map(|t| t.id)
            .collect();
        assert_eq!(hidden, ["plan"]);
        let own = tab_added(&mut l, "Trips").unwrap();
        let before = l.clone();
        tab_hidden_set(&mut l, &own, true);
        assert_eq!(l, before);
    }

    #[test]
    fn tabs_default_order_then_saved_order() {
        let mut l = UiLayout::default();
        assert_eq!(tab_ids(&l), defaults());
        assert!(tabs(&l).iter().all(|t| t.built_in && !t.hidden));
        tab_added(&mut l, "Trips").unwrap();
        let mut want = defaults();
        want.push("own-1".into());
        assert_eq!(tab_ids(&l), want);
        tab_moved(&mut l, "own-1", 0);
        want.rotate_right(1);
        assert_eq!(tab_ids(&l), want);
    }

    #[test]
    fn a_saved_tab_list_missing_a_built_in_gets_it_back_in_place() {
        let tab = |id: &str| TabLayout {
            id: id.into(),
            name: id.into(),
            built_in: true,
            hidden: false,
            pages: vec![],
        };
        let l = UiLayout {
            tabs: [
                "plan",
                "overview",
                "transactions",
                "wishes",
                "events",
                "manage",
            ]
            .map(tab)
            .into(),
            tabs_ordered: true,
            ..Default::default()
        };
        assert_eq!(
            tab_ids(&l),
            [
                "plan",
                "overview",
                "transactions",
                "spending",
                "wishes",
                "events",
                "manage"
            ]
        );
        // With no earlier built-in saved, the missing one goes first.
        let l = UiLayout {
            tabs: vec![tab("plan")],
            tabs_ordered: true,
            ..Default::default()
        };
        assert_eq!(tab_ids(&l), defaults());
    }

    fn built_in(id: &str) -> TabLayout {
        TabLayout {
            id: id.into(),
            name: id.into(),
            built_in: true,
            hidden: false,
            pages: vec![],
        }
    }

    fn click_order() -> UiLayout {
        let mut spending = built_in("spending");
        spending.pages = vec![PageLayout {
            id: "spending.habits".into(),
            items: vec![PanelItem {
                panel: "s".into(),
                folded: true,
                half: false,
            }],
            removed: vec![],
        }];
        UiLayout {
            tabs: vec![spending, built_in("overview")],
            tabs_ordered: false,
            ..Default::default()
        }
    }

    #[test]
    fn a_0_3_0_layout_saved_in_click_order_shows_the_default_bar() {
        assert_eq!(tab_ids(&click_order()), defaults());
    }

    #[test]
    fn the_first_write_stores_the_default_order_and_the_flag() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = click_order();
        let spending = click_order().tabs[0].pages.clone();
        panel_removed(&mut l, &d, &p1(), "a");
        let saved: Vec<_> = l.tabs.iter().map(|t| t.id.clone()).collect();
        assert_eq!(saved, defaults());
        assert!(l.tabs_ordered);
        assert_eq!(removed(&l, "p1"), ["a"]);
        let after = l.tabs.iter().find(|t| t.id == "spending").unwrap();
        assert_eq!(after.pages, spending);
    }

    #[test]
    fn an_ordered_layout_keeps_its_saved_order() {
        let order = [
            "plan",
            "overview",
            "transactions",
            "spending",
            "wishes",
            "events",
            "manage",
        ];
        let l = UiLayout {
            tabs: order.map(built_in).into(),
            tabs_ordered: true,
            ..Default::default()
        };
        assert_eq!(tab_ids(&l), order);
    }

    #[test]
    fn moving_a_tab_writes_every_tab_in_order() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_removed(&mut l, &d, &at("plan", "p1"), "a");
        tab_moved(&mut l, "manage", 1);
        let saved: Vec<_> = l.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            saved,
            [
                "overview",
                "manage",
                "transactions",
                "spending",
                "plan",
                "wishes",
                "events"
            ]
        );
        assert_eq!(removed(&l, "p1"), ["a"]);
        tab_moved(&mut l, "overview", 99);
        assert_eq!(l.tabs.last().unwrap().id, "overview");
        assert_eq!(l.tabs.len(), 7);
    }

    #[test]
    fn page_reset_restores_first_run() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        panel_removed(&mut l, &d, &p1(), "a");
        panel_added(&mut l, &d, &p1(), "m");
        panel_moved(&mut l, &d, &p1(), "m", 0);
        panel_added(&mut l, &d, &p2(), "a");
        page_reset(&mut l, &p1());
        assert_eq!(
            resolve(&d, &l.plots, "p1", l.page("p1")),
            resolve(&d, &[], "p1", None)
        );
        assert_eq!(ids(&l, "p2"), ["m", "a"]);
    }

    #[test]
    fn all_reset_drops_own_tabs() {
        let mut l = UiLayout::default();
        tab_added(&mut l, "Trips").unwrap();
        tab_hidden_set(&mut l, "plan", true);
        all_reset(&mut l);
        assert_eq!(l, UiLayout::default());
        assert_eq!(tab_ids(&l), defaults());
    }

    #[test]
    fn saved_plots_survive_reset_and_tab_delete() {
        let plots = vec![finance_types::ui::SavedPlot {
            id: "p".into(),
            title: "P".into(),
            spec: Default::default(),
        }];
        let mut l = UiLayout {
            plots: plots.clone(),
            ..Default::default()
        };
        let own = tab_added(&mut l, "Trips").unwrap();
        tab_deleted(&mut l, &own);
        assert_eq!(l.plots, plots);
        all_reset(&mut l);
        assert_eq!(l.plots, plots);
        assert_eq!(tab_ids(&l), defaults());
    }

    fn spec(top: u8) -> PlotSpec {
        PlotSpec {
            top,
            ..Default::default()
        }
    }

    fn plot_ids(l: &UiLayout) -> Vec<&str> {
        l.plots.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn saving_a_new_plot_gives_the_smallest_free_id() {
        let mut l = UiLayout::default();
        assert_eq!(plot_saved(&mut l, None, "A", &spec(1)).unwrap(), "p1");
        assert_eq!(plot_saved(&mut l, None, "B", &spec(2)).unwrap(), "p2");
        plot_deleted(&mut l, "p1");
        assert_eq!(plot_saved(&mut l, None, "C", &spec(3)).unwrap(), "p1");
        assert_eq!(plot_ids(&l), ["p2", "p1"]);
        assert_eq!(l.plots[1].spec, serde_json::to_value(spec(3)).unwrap());
    }

    #[test]
    fn saving_over_an_id_replaces_title_and_spec() {
        let mut l = UiLayout::default();
        plot_saved(&mut l, None, "A", &spec(1)).unwrap();
        plot_saved(&mut l, None, "B", &spec(2)).unwrap();
        assert_eq!(
            plot_saved(&mut l, Some("p1"), "A2", &spec(9)).unwrap(),
            "p1"
        );
        assert_eq!(plot_ids(&l), ["p1", "p2"]);
        assert_eq!(l.plots[0].title, "A2");
        assert_eq!(l.plots[0].spec, serde_json::to_value(spec(9)).unwrap());
        let before = l.clone();
        assert!(plot_saved(&mut l, Some("p7"), "Gone", &spec(1)).is_err());
        assert_eq!(l, before);
    }

    #[test]
    fn a_plot_title_is_trimmed_and_checked() {
        let mut l = UiLayout::default();
        plot_saved(&mut l, None, "  Food ", &spec(1)).unwrap();
        assert_eq!(l.plots[0].title, "Food");
        let before = l.clone();
        assert!(plot_saved(&mut l, None, "   ", &spec(1)).is_err());
        assert!(plot_saved(&mut l, None, &"x".repeat(41), &spec(1)).is_err());
        assert!(plot_saved(&mut l, Some("p1"), " ", &spec(1)).is_err());
        assert_eq!(l, before);
        assert!(plot_saved(&mut l, None, &"x".repeat(40), &spec(1)).is_ok());
    }

    #[test]
    fn the_two_hundred_and_first_plot_is_refused() {
        let mut l = UiLayout::default();
        for _ in 0..MAX_PLOTS {
            plot_saved(&mut l, None, "P", &spec(1)).unwrap();
        }
        let before = l.clone();
        assert!(plot_saved(&mut l, None, "One more", &spec(1)).is_err());
        assert_eq!(l, before);
        // Replacing one still works at the limit.
        assert!(plot_saved(&mut l, Some("p200"), "Last", &spec(1)).is_ok());
        assert_eq!(l.validate(), Ok(()));
    }

    #[test]
    fn deleting_a_plot_removes_it_from_every_page() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let p1_id = plot_saved(&mut l, None, "A", &spec(1)).unwrap();
        let p2_id = plot_saved(&mut l, None, "B", &spec(1)).unwrap();
        let (a, b) = (plot_panel_id(&p1_id), plot_panel_id(&p2_id));
        panel_added(&mut l, &d, &p1(), &a);
        panel_added(&mut l, &d, &p2(), &a);
        panel_added(&mut l, &d, &p2(), &b);
        l.page_mut("plan", "Plan", "p2").removed.push(a.clone());
        plot_deleted(&mut l, &p1_id);
        assert_eq!(plot_ids(&l), ["p2"]);
        assert_eq!(ids(&l, "p1"), ["a", "b"]);
        assert_eq!(ids(&l, "p2"), ["m".to_string(), b]);
        assert!(removed(&l, "p2").is_empty());
        assert_eq!(l.validate(), Ok(()));
    }

    #[test]
    fn a_plot_panel_survives_resolve_and_a_deleted_one_does_not() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let id = plot_saved(&mut l, None, "A", &spec(1)).unwrap();
        let panel = plot_panel_id(&id);
        assert_eq!(plot_id_of(&panel), Some(id.as_str()));
        assert_eq!(plot_id_of("a"), None);
        // On any page, several at once, and sent between tabs.
        panel_added(&mut l, &d, &p1(), &panel);
        panel_added(&mut l, &d, &p2(), &panel);
        let shown = |l: &UiLayout, page: &str| -> Vec<String> {
            resolve(&d, &l.plots, page, l.page(page))
                .into_iter()
                .map(|i| i.panel)
                .collect()
        };
        assert_eq!(shown(&l, "p1"), ["a", "b", panel.as_str()]);
        assert_eq!(shown(&l, "p2"), ["m", panel.as_str()]);
        let own = at("own-1", "own-1");
        tab_added(&mut l, "Trips").unwrap();
        assert!(sendable(&l, &d, &p1(), &own, &panel));
        panel_sent(&mut l, &d, &p1(), &own, &panel);
        assert_eq!(shown(&l, "p1"), ["a", "b"]);
        assert!(removed(&l, "p1").is_empty());
        assert_eq!(shown(&l, "own-1"), [panel.as_str()]);
        // Removed from a page: just dropped.
        panel_removed(&mut l, &d, &own, &panel);
        assert!(removed(&l, "own-1").is_empty());
        // A plot the layout no longer has is dropped by resolve.
        l.plots.clear();
        assert_eq!(shown(&l, "p2"), ["m"]);
        let before = l.clone();
        panel_added(&mut l, &d, &p1(), &panel);
        assert_eq!(l, before);
    }

    #[test]
    fn addable_offers_saved_plots_not_on_the_page() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        plot_saved(&mut l, None, "Food", &spec(1)).unwrap();
        plot_saved(&mut l, None, "Food", &spec(2)).unwrap();
        let offered = |l: &UiLayout| addable(l, &d, &p1());
        let pair = |id: &str, t: &str| (id.to_string(), t.to_string());
        assert_eq!(
            offered(&l),
            [
                pair("m", "m"),
                pair("plot:p1", "Food"),
                pair("plot:p2", "Food")
            ]
        );
        panel_added(&mut l, &d, &p1(), "plot:p1");
        assert_eq!(offered(&l), [pair("m", "m"), pair("plot:p2", "Food")]);
    }

    #[test]
    fn an_edit_on_a_deleted_own_tab_changes_nothing() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let own = tab_added(&mut l, "Trips").unwrap();
        tab_deleted(&mut l, &own);
        let before = l.clone();
        let gone = at(&own, &own);
        panel_added(&mut l, &d, &gone, "m");
        panel_removed(&mut l, &d, &gone, "m");
        panel_moved(&mut l, &d, &gone, "m", 0);
        panel_half_set(&mut l, &d, &gone, "m", true);
        panel_sent(&mut l, &d, &p2(), &gone, "m");
        panel_sent(&mut l, &d, &gone, &p1(), "m");
        assert_eq!(l, before);
    }

    #[test]
    fn every_edit_passes_validate() {
        let r = reg();
        let d: Vec<&PanelDef> = r.iter().collect();
        let mut l = UiLayout::default();
        let own = tab_added(&mut l, "Trips").unwrap();
        let own_page = at(&own, &own);
        let edits: Vec<Box<dyn Fn(&mut UiLayout)>> = vec![
            Box::new(|l| panel_removed(l, &d, &p1(), "a")),
            Box::new(|l| panel_removed(l, &d, &p1(), "a")),
            Box::new(|l| panel_added(l, &d, &p1(), "a")),
            Box::new(|l| panel_added(l, &d, &p1(), "m")),
            Box::new(|l| panel_added(l, &d, &own_page, "m")),
            Box::new(|l| panel_moved(l, &d, &p1(), "m", 0)),
            Box::new(|l| panel_half_set(l, &d, &p1(), "b", true)),
            Box::new(|l| panel_sent(l, &d, &p1(), &p2(), "a")),
            Box::new(|l| panel_sent(l, &d, &p2(), &own_page, "a")),
            Box::new(|l| {
                let _ = tab_added(l, "Plan");
            }),
            Box::new(|l| {
                let _ = tab_renamed(l, "own-1", "Holidays");
            }),
            Box::new(|l| tab_hidden_set(l, "plan", true)),
            Box::new(|l| tab_hidden_set(l, "manage", true)),
            Box::new(|l| tab_moved(l, "own-1", 0)),
            Box::new(|l| tab_deleted(l, "own-2")),
            Box::new(|l| {
                let _ = plot_saved(l, None, " Food ", &PlotSpec::default());
            }),
            Box::new(|l| {
                let _ = plot_saved(l, None, "Bills", &PlotSpec::default());
            }),
            Box::new(|l| {
                let _ = plot_saved(l, Some("p1"), "Groceries", &PlotSpec::default());
            }),
            Box::new(|l| {
                let _ = plot_saved(l, None, "", &PlotSpec::default());
            }),
            Box::new(|l| panel_added(l, &d, &p1(), "plot:p1")),
            Box::new(|l| panel_added(l, &d, &own_page, "plot:p1")),
            Box::new(|l| panel_added(l, &d, &p2(), "plot:p2")),
            Box::new(|l| panel_sent(l, &d, &p2(), &p1(), "plot:p2")),
            Box::new(|l| panel_removed(l, &d, &p1(), "plot:p2")),
            Box::new(|l| plot_deleted(l, "p1")),
            Box::new(|l| page_reset(l, &p1())),
            Box::new(|l| all_reset(l)),
        ];
        for (n, e) in edits.iter().enumerate() {
            e(&mut l);
            assert_eq!(l.validate(), Ok(()), "edit {n}");
        }
    }
}
