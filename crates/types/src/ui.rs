//! The saved UI layout: which tabs exist, and which panels each page shows, folded or half-width.

use serde::{Deserialize, Serialize};

pub const MAX_TABS: usize = 50;
pub const MAX_NAME: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiLayout {
    #[serde(default)]
    pub tabs: Vec<TabLayout>,
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
        let one = |t| UiLayout { tabs: vec![t] };
        assert!(one(tab("plan", "Plan", true)).validate().is_ok());
        assert!(one(tab("manage", "Manage", true)).validate().is_err());
        assert!(one(tab("plan", "", false)).validate().is_err());
        assert!(one(tab("plan", &"x".repeat(41), false)).validate().is_err());
        let twice = UiLayout {
            tabs: vec![tab("plan", "Plan", false), tab("plan", "Plan 2", false)],
        };
        assert!(twice.validate().is_err());
        let many = UiLayout {
            tabs: (0..51).map(|i| tab(&format!("t{i}"), "T", false)).collect(),
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
}
