//! Panel registry types and the pure logic that turns a saved layout into the
//! panels a page shows.

// Nothing renders from here yet; Task 5 removes this allow.
#![allow(dead_code)]

use finance_types::report::Dashboard;
use finance_types::ui::{PageLayout, PanelItem};
use leptos::prelude::AnyView;

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
}

fn item_from_def(d: &PanelDef, folded: bool) -> PanelItem {
    PanelItem {
        panel: d.id.to_string(),
        folded,
        half: d.half,
    }
}

/// The panels a page shows, in order. `defs` are the registry entries whose
/// home is this page, in registry order.
pub fn resolve(defs: &[&PanelDef], saved: Option<&PageLayout>) -> Vec<PanelItem> {
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
        .filter(|i| defs.iter().any(|d| d.id == i.panel))
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

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::IntoAny;

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
        let got = resolve(&refs, None);
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
        assert_eq!(resolve(&refs, Some(&saved)), saved.items);
    }

    #[test]
    fn an_unknown_saved_panel_is_skipped() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false), it("gone.panel", false)], &[]);
        assert_eq!(
            ids(&resolve(&refs, Some(&saved))),
            [("a", false), ("b", true)]
        );
    }

    #[test]
    fn a_removed_panel_stays_removed() {
        let d = abc();
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &["b"]);
        assert_eq!(resolve(&refs, Some(&saved)), saved.items);
    }

    #[test]
    fn an_absent_panel_is_never_inserted() {
        let d = [
            def("a", Initial::Open, false),
            def("c", Initial::Absent, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("a", false)], &[]);
        assert_eq!(resolve(&refs, Some(&saved)), saved.items);
    }

    #[test]
    fn a_new_panel_lands_folded_after_its_registry_neighbour() {
        let d = [
            def("a", Initial::Open, false),
            def("n", Initial::Open, false),
            def("b", Initial::Open, false),
        ];
        let refs: Vec<&PanelDef> = d.iter().collect();
        let saved = page(vec![it("b", false), it("a", false)], &[]);
        assert_eq!(
            ids(&resolve(&refs, Some(&saved))),
            [("b", false), ("a", false), ("n", true)]
        );
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
            ids(&resolve(&refs, Some(&saved))),
            [("n", true), ("a", false)]
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
