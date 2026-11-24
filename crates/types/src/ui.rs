//! The saved UI layout: which tabs exist, and which panels each page shows, folded or half-width.

use serde::{Deserialize, Serialize};

pub const MAX_TABS: usize = 50;
pub const MAX_NAME: usize = 40;
pub const MAX_PLOTS: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiLayout {
    #[serde(default)]
    pub tabs: Vec<TabLayout>,
    /// `tabs` is the whole bar in the owner's order. False: written by 0.3.0
    /// folds in click order, so the built-in tabs show in their default order.
    #[serde(default)]
    pub tabs_ordered: bool,
    #[serde(default)]
    pub plots: Vec<SavedPlot>,
}

/// A plot the owner saved. `spec` is kept as JSON (a `PlotSpec` when readable)
/// so one plot an older or newer build can't read cannot fail the layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPlot {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub spec: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabLayout {
    pub id: String,
    pub name: String,
    pub built_in: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pages: Vec<PageLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageLayout {
    pub id: String,
    #[serde(default)]
    pub items: Vec<PanelItem>,
    #[serde(default)]
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelItem {
    pub panel: String,
    #[serde(default)]
    pub folded: bool,
    #[serde(default)]
    pub half: bool,
}

impl UiLayout {
    pub fn page(&self, page_id: &str) -> Option<&PageLayout> {
        self.tabs
            .iter()
            .flat_map(|t| &t.pages)
            .find(|p| p.id == page_id)
    }

    /// The page's entry, created (with its tab) when missing. `tab_name` is used only on creation.
    pub fn page_mut(&mut self, tab_id: &str, tab_name: &str, page_id: &str) -> &mut PageLayout {
        let ti = match self.tabs.iter().position(|t| t.id == tab_id) {
            Some(i) => i,
            None => {
                self.tabs.push(TabLayout {
                    id: tab_id.into(),
                    name: tab_name.into(),
                    built_in: true,
                    hidden: false,
                    pages: vec![],
                });
                self.tabs.len() - 1
            }
        };
        let pages = &mut self.tabs[ti].pages;
        let pi = match pages.iter().position(|p| p.id == page_id) {
            Some(i) => i,
            None => {
                pages.push(PageLayout {
                    id: page_id.into(),
                    items: vec![],
                    removed: vec![],
                });
                pages.len() - 1
            }
        };
        &mut pages[pi]
    }

    /// Drops saved plots that break a rule (empty or repeated id, bad title,
    /// past `MAX_PLOTS`) and the panels showing them, so one bad plot can't
    /// reset the whole layout on load.
    pub fn drop_bad_plots(&mut self) {
        let before: Vec<String> = self.plots.iter().map(|p| p.id.clone()).collect();
        let mut kept: Vec<SavedPlot> = Vec::new();
        for p in std::mem::take(&mut self.plots) {
            let title = p.title.chars().count();
            if !p.id.is_empty()
                && (1..=MAX_NAME).contains(&title)
                && !kept.iter().any(|o| o.id == p.id)
            {
                kept.push(p);
            }
        }
        kept.truncate(MAX_PLOTS);
        let gone: Vec<String> = before
            .into_iter()
            .filter(|id| !kept.iter().any(|p| &p.id == id))
            .map(|id| format!("plot:{id}"))
            .collect();
        self.plots = kept;
        for page in self.tabs.iter_mut().flat_map(|t| &mut t.pages) {
            page.items.retain(|it| !gone.contains(&it.panel));
            page.removed.retain(|panel| !gone.contains(panel));
        }
    }

    /// `Err` with a short reason when the layout breaks a rule.
    pub fn validate(&self) -> Result<(), String> {
        if self.tabs.len() > MAX_TABS {
            return Err(format!("at most {MAX_TABS} tabs"));
        }
        for (i, t) in self.tabs.iter().enumerate() {
            if self.tabs[..i].iter().any(|o| o.id == t.id) {
                return Err(format!("tab id {} used twice", t.id));
            }
            if t.name.is_empty() || t.name.chars().count() > MAX_NAME {
                return Err(format!("tab name must be 1 to {MAX_NAME} characters"));
            }
            if t.id == "manage" && t.hidden {
                return Err("the Manage tab cannot be hidden".into());
            }
            for p in &t.pages {
                for (j, it) in p.items.iter().enumerate() {
                    if p.items[..j].iter().any(|o| o.panel == it.panel)
                        || p.removed.contains(&it.panel)
                    {
                        return Err(format!("panel {} listed twice on page {}", it.panel, p.id));
                    }
                }
            }
        }
        if self.plots.len() > MAX_PLOTS {
            return Err(format!("at most {MAX_PLOTS} plots"));
        }
        for (i, p) in self.plots.iter().enumerate() {
            if p.id.is_empty() {
                return Err("a plot needs an id".into());
            }
            if self.plots[..i].iter().any(|o| o.id == p.id) {
                return Err(format!("plot id {} used twice", p.id));
            }
            if p.title.is_empty() || p.title.chars().count() > MAX_NAME {
                return Err(format!("plot title must be 1 to {MAX_NAME} characters"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_is_the_default() {
        assert_eq!(
            serde_json::from_str::<UiLayout>("{}").unwrap(),
            UiLayout::default()
        );
    }

    #[test]
    fn tabs_ordered_defaults_to_false_for_old_json() {
        let old = r#"{"tabs":[{"id":"spending","name":"Spending","built_in":true}]}"#;
        let l: UiLayout = serde_json::from_str(old).unwrap();
        assert_eq!(l.tabs.len(), 1);
        assert!(!l.tabs_ordered);
    }

    #[test]
    fn page_mut_creates_tab_and_page_once() {
        let mut l = UiLayout::default();
        l.page_mut("spending", "Spending", "spending.habits")
            .items
            .push(PanelItem {
                panel: "spending.habits.outlook".into(),
                folded: true,
                half: false,
            });
        l.page_mut("spending", "Spending", "spending.history");
        assert_eq!(l.tabs.len(), 1);
        assert_eq!(l.tabs[0].pages.len(), 2);
        assert!(l.tabs[0].built_in);
        assert!(l.page("spending.habits").unwrap().items[0].folded);
        assert!(l.page("nope").is_none());
    }

    #[test]
    fn validate_rules() {
        let tab = |id: &str, name: &str, hidden| TabLayout {
            id: id.into(),
            name: name.into(),
            built_in: true,
            hidden,
            pages: vec![],
        };
        let one = |t| UiLayout {
            tabs: vec![t],
            ..Default::default()
        };
        assert!(one(tab("plan", "Plan", true)).validate().is_ok());
        assert!(one(tab("manage", "Manage", true)).validate().is_err());
        assert!(one(tab("plan", "", false)).validate().is_err());
        assert!(one(tab("plan", &"x".repeat(41), false)).validate().is_err());
        let twice = UiLayout {
            tabs: vec![tab("plan", "Plan", false), tab("plan", "Plan 2", false)],
            ..Default::default()
        };
        assert!(twice.validate().is_err());
        let many = UiLayout {
            tabs: (0..51).map(|i| tab(&format!("t{i}"), "T", false)).collect(),
            ..Default::default()
        };
        assert!(many.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_panel_listed_twice_on_a_page() {
        let item = |p: &str| PanelItem {
            panel: p.into(),
            folded: false,
            half: false,
        };
        let layout = |items: Vec<PanelItem>, removed: Vec<String>| {
            let mut l = UiLayout::default();
            let page = l.page_mut("plan", "Plan", "plan");
            page.items = items;
            page.removed = removed;
            l
        };
        assert!(
            layout(vec![item("a"), item("b")], vec!["c".into()])
                .validate()
                .is_ok()
        );
        assert!(
            layout(vec![item("a"), item("a")], vec![])
                .validate()
                .is_err()
        );
        assert!(
            layout(vec![item("a")], vec!["a".into()])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn plots_default_to_empty_for_old_layouts() {
        let l: UiLayout = serde_json::from_str(r#"{"tabs":[]}"#).unwrap();
        assert!(l.plots.is_empty());
    }

    #[test]
    fn a_layout_with_an_unreadable_plot_still_loads() {
        let json = r#"{"tabs":[],"plots":[{"id":"p1","title":"Moon","spec":{"by":"moon_phase"}}]}"#;
        let l: UiLayout = serde_json::from_str(json).unwrap();
        assert_eq!(l.plots[0].title, "Moon");
        assert!(l.validate().is_ok());
        let spec = serde_json::from_value::<crate::plot::PlotSpec>(l.plots[0].spec.clone());
        assert!(spec.is_err());
    }

    #[test]
    fn validate_rules_for_plots() {
        let plot = |id: &str, title: &str| SavedPlot {
            id: id.into(),
            title: title.into(),
            spec: serde_json::json!({}),
        };
        let with = |plots| UiLayout {
            plots,
            ..Default::default()
        };
        assert!(
            with(vec![plot("a", "A"), plot("b", "A")])
                .validate()
                .is_ok()
        );
        assert!(
            with(vec![plot("a", "A"), plot("a", "B")])
                .validate()
                .is_err()
        );
        assert!(with(vec![plot("a", "")]).validate().is_err());
        assert!(with(vec![plot("", "A")]).validate().is_err());
        assert!(with(vec![plot("a", &"x".repeat(41))]).validate().is_err());
        let many = (0..=MAX_PLOTS)
            .map(|i| plot(&format!("p{i}"), "P"))
            .collect();
        assert!(with(many).validate().is_err());
    }

    #[test]
    fn drop_bad_plots_keeps_the_good_ones_and_their_panels() {
        let plot = |id: &str, title: &str| SavedPlot {
            id: id.into(),
            title: title.into(),
            spec: serde_json::json!({}),
        };
        let item = |p: &str| PanelItem {
            panel: p.into(),
            folded: false,
            half: false,
        };
        let mut l = UiLayout {
            plots: vec![
                plot("a", "A"),
                plot("", "No id"),
                plot("b", ""),
                plot("a", "Again"),
                plot("c", &"x".repeat(41)),
                plot("d", "D"),
            ],
            ..Default::default()
        };
        let page = l.page_mut("t", "T", "t");
        page.items = vec![
            item("plot:a"),
            item("plot:b"),
            item("other"),
            item("plot:d"),
        ];
        page.removed = vec!["plot:c".into()];
        l.drop_bad_plots();
        let ids: Vec<_> = l
            .plots
            .iter()
            .map(|p| (p.id.as_str(), p.title.as_str()))
            .collect();
        assert_eq!(ids, [("a", "A"), ("d", "D")]);
        let panels: Vec<_> = l
            .page("t")
            .unwrap()
            .items
            .iter()
            .map(|i| i.panel.as_str())
            .collect();
        assert_eq!(panels, ["plot:a", "other", "plot:d"]);
        assert!(l.page("t").unwrap().removed.is_empty());
        assert!(l.validate().is_ok());

        let mut many = UiLayout {
            plots: (0..MAX_PLOTS + 5)
                .map(|i| plot(&format!("p{i}"), "P"))
                .collect(),
            ..Default::default()
        };
        many.drop_bad_plots();
        assert_eq!(many.plots.len(), MAX_PLOTS);
    }
}
