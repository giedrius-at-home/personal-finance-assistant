//! The saved UI layout: one JSON row, kept apart from the ledger.

use finance_types::ui::UiLayout;
use rusqlite::{Connection, OptionalExtension};

use crate::Result;
use crate::writes::{Writer, invalid};

const MAX_PAYLOAD: usize = 256 * 1024;

/// The stored layout; the default when the table or row is missing or the
/// payload does not parse or breaks a layout rule, so a bad row never hides
/// the app or blocks later saves.
pub fn load(conn: &Connection) -> Result<UiLayout> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'ui_layout')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(UiLayout::default());
    }
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM ui_layout WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(payload
        .and_then(|p| serde_json::from_str::<UiLayout>(&p).ok())
        .filter(|l| l.validate().is_ok())
        .unwrap_or_default())
}

pub fn save(w: &Writer<'_>, layout: &UiLayout) -> Result<()> {
    layout.validate().map_err(invalid)?;
    if serde_json::to_string(layout)
        .map_err(|e| invalid(e.to_string()))?
        .len()
        > MAX_PAYLOAD
    {
        return Err(invalid("the layout is too large"));
    }
    w.put_json(
        "INSERT INTO ui_layout (singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        layout,
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Service;
    use finance_types::ui::{PageLayout, PanelItem, TabLayout};
    use std::path::PathBuf;

    fn tab(id: &str, hidden: bool) -> TabLayout {
        TabLayout {
            id: id.into(),
            name: id.into(),
            built_in: true,
            hidden,
            pages: vec![PageLayout {
                id: "p".into(),
                items: vec![PanelItem {
                    panel: "a".into(),
                    folded: true,
                    half: false,
                }],
                removed: vec![],
            }],
        }
    }

    fn layout() -> UiLayout {
        UiLayout {
            tabs: vec![tab("overview", false), tab("manage", false)],
        }
    }

    /// A database file with the write tests' tiny ledger, for the service.
    fn service(name: &str) -> (Service, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("finance-layout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("finance.sqlite3");
        let store = finance_storage::Store::open(&path).unwrap();
        store
            .connection()
            .execute_batch(include_str!("writes/test_fixture.sql"))
            .unwrap();
        drop(store);
        (Service::new(path), dir)
    }

    fn backups(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir.join("backups")).map_or(0, |d| d.count())
    }

    #[test]
    fn load_without_the_table_is_default() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), UiLayout::default());
    }

    #[test]
    fn load_of_garbage_payload_is_default() {
        let db = crate::writes::fixture::db();
        for garbage in ["not json", "[1,2]"] {
            db.store
                .connection()
                .execute(
                    "INSERT OR REPLACE INTO ui_layout (singleton, payload_json) VALUES (1, ?1)",
                    [garbage],
                )
                .unwrap();
            assert_eq!(load(db.store.connection()).unwrap(), UiLayout::default());
        }
    }

    #[test]
    fn a_stored_layout_that_breaks_a_rule_loads_as_default_and_saves_again() {
        let mut db = crate::writes::fixture::db();
        let bad = UiLayout {
            tabs: vec![tab("manage", true)],
        };
        db.store
            .connection()
            .execute(
                "INSERT OR REPLACE INTO ui_layout (singleton, payload_json) VALUES (1, ?1)",
                [serde_json::to_string(&bad).unwrap()],
            )
            .unwrap();
        assert_eq!(load(db.store.connection()).unwrap(), UiLayout::default());
        db.write(|w, _| save(w, &layout())).unwrap();
        assert_eq!(load(db.store.connection()).unwrap(), layout());
    }

    #[test]
    fn save_then_load_round_trips() {
        let mut db = crate::writes::fixture::db();
        db.write(|w, _| save(w, &layout())).unwrap();
        assert_eq!(load(db.store.connection()).unwrap(), layout());
    }

    #[test]
    fn save_refuses_hidden_manage_and_keeps_the_old_row() {
        let mut db = crate::writes::fixture::db();
        db.write(|w, _| save(w, &layout())).unwrap();
        let bad = UiLayout {
            tabs: vec![tab("manage", true)],
        };
        assert!(db.write(|w, _| save(w, &bad)).is_err());
        assert_eq!(load(db.store.connection()).unwrap(), layout());
    }

    #[test]
    fn save_refuses_a_payload_over_256_kib() {
        let mut db = crate::writes::fixture::db();
        let mut big = layout();
        big.tabs[0].pages[0].removed = (0..30_000).map(|i| format!("panel-number-{i}")).collect();
        assert!(db.write(|w, _| save(w, &big)).is_err());
        assert_eq!(load(db.store.connection()).unwrap(), UiLayout::default());
    }

    #[test]
    fn saving_the_layout_keeps_the_cached_snapshot() {
        let (service, dir) = service("snapshot");
        service.dashboard().unwrap();
        assert!(service.snapshot_cached());
        service.save_ui_layout(layout()).unwrap();
        assert!(service.snapshot_cached());
        assert_eq!(service.ui_layout().unwrap(), layout());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_the_layout_makes_no_backup() {
        let (service, dir) = service("nobackup");
        service.save_ui_layout(layout()).unwrap();
        assert_eq!(backups(&dir), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_first_ledger_write_after_a_layout_save_still_backs_up() {
        let (service, dir) = service("backup");
        service.save_ui_layout(layout()).unwrap();
        service.write(|_, _| Ok(())).unwrap();
        assert_eq!(backups(&dir), 1);
        service.write(|_, _| Ok(())).unwrap();
        assert_eq!(backups(&dir), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_database_gives_no_data() {
        let service =
            Service::new(std::env::temp_dir().join("finance-layout-missing/none.sqlite3"));
        assert_eq!(service.ui_layout().unwrap_err(), "no-data");
        assert_eq!(service.save_ui_layout(layout()).unwrap_err(), "no-data");
    }
}
