//! The app's data service: one place that opens the store, caches the
//! snapshot between refreshes and runs every read and write the screens ask
//! for. The Tauri commands and the development server are thin wrappers.

use std::path::PathBuf;
use std::sync::Mutex;

use finance_storage::Store;
use finance_types::DataStatus;
use finance_types::report::review::{TransactionSearchQuery, TransactionSearchResult};
use finance_types::report::{Dashboard, ReviewReport};
use jiff::Zoned;

use crate::writes::Writer;
use crate::{Ctx, Snapshot, dashboard, review};

/// Keep this many automatic backups.
const BACKUPS: usize = 10;

/// Copy the database to `backups/finance-<UTC time>.sqlite3` next to it and
/// prune old copies. `VACUUM INTO` gives a consistent copy of a live WAL file.
fn backup(path: &std::path::Path) -> Result<(), String> {
    let Some(dir) = path.parent().map(|p| p.join("backups")) else {
        return Ok(());
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!(
        "finance-{}.sqlite3",
        jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ")
    );
    let target = dir.join(name);
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
        .map_err(|e| e.to_string())?;
    let mut copies: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("finance-2") && n.ends_with(".sqlite3"))
        })
        .collect();
    copies.sort();
    while copies.len() > BACKUPS {
        let oldest = copies.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Ok(())
}

/// Errors cross to the UI as short, data-free strings.
pub const NO_DATA: &str = "no-data";

#[derive(Debug)]
struct Open {
    store: Store,
    snapshot: Option<Snapshot>,
}

#[derive(Debug)]
pub struct Service {
    path: PathBuf,
    state: Mutex<Option<Open>>,
    /// The read-write store, opened on the first write.
    writer: Mutex<Option<Store>>,
}

impl Service {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
            writer: Mutex::new(None),
        }
    }

    /// Run `f` on the store and a current snapshot (loaded once, then reused
    /// until [`Service::refresh`]).
    fn read<T>(&self, f: impl FnOnce(&Store, &Snapshot) -> crate::Result<T>) -> Result<T, String> {
        let mut guard = self.state.lock().map_err(|_| "store-poisoned".to_owned())?;
        if guard.is_none() {
            if !self.path.exists() {
                return Err(NO_DATA.to_owned());
            }
            let store = Store::open_read_only(&self.path).map_err(|e| e.to_string())?;
            *guard = Some(Open {
                store,
                snapshot: None,
            });
        }
        let open = guard.as_mut().ok_or_else(|| NO_DATA.to_owned())?;
        if open.snapshot.is_none() {
            open.snapshot = Some(Snapshot::load(&open.store).map_err(|e| e.to_string())?);
        }
        let snapshot = open.snapshot.as_ref().ok_or_else(|| NO_DATA.to_owned())?;
        f(&open.store, snapshot).map_err(|e| e.to_string())
    }

    /// Forget the cached snapshot so the next read sees the latest data.
    pub fn refresh(&self) {
        if let Ok(mut guard) = self.state.lock()
            && let Some(open) = guard.as_mut()
        {
            open.snapshot = None;
        }
    }

    /// Run `f` in one write transaction, then drop the cached snapshot so
    /// every screen sees the change. The first write of a session backs the
    /// database up first.
    pub fn write<T>(
        &self,
        f: impl FnOnce(&Writer<'_>, &Snapshot) -> crate::Result<T>,
    ) -> Result<T, String> {
        let snapshot = {
            let mut snap = None;
            self.read(|_, s| {
                snap = Some(s.clone());
                Ok(())
            })?;
            snap.ok_or_else(|| NO_DATA.to_owned())?
        };
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| "store-poisoned".to_owned())?;
        if guard.is_none() {
            backup(&self.path).map_err(|e| format!("backup failed: {e}"))?;
            *guard = Some(Store::open(&self.path).map_err(|e| e.to_string())?);
        }
        let store = guard.as_mut().ok_or_else(|| NO_DATA.to_owned())?;
        let result = (|| {
            let writer = Writer {
                tx: store.connection_mut().transaction()?,
                now: Zoned::now().with_time_zone(snapshot.zone.clone()),
            };
            let value = f(&writer, &snapshot)?;
            writer.tx.commit()?;
            Ok::<T, crate::Error>(value)
        })()
        .map_err(|e| e.to_string());
        drop(guard);
        self.refresh();
        result
    }

    pub fn dashboard(&self) -> Result<Dashboard, String> {
        self.read(|store, snap| dashboard::dashboard(store, snap, &Zoned::now()))
    }

    pub fn review(&self) -> Result<ReviewReport, String> {
        self.read(|_, snap| Ok(dashboard::review_report(snap, &Zoned::now())))
    }

    pub fn search(
        &self,
        query: &TransactionSearchQuery,
    ) -> Result<TransactionSearchResult, String> {
        self.read(|_, snap| review::search_transactions(&Ctx::new(snap, &Zoned::now()), query))
    }

    pub fn save_budget_plan(
        &self,
        plan: finance_types::gateway::BudgetPlan,
    ) -> Result<finance_types::gateway::BudgetPlan, String> {
        self.write(|w, snap| crate::writes::budget::save_budget_plan(w, snap, plan))
    }

    pub fn create_bill(
        &self,
        request: finance_types::gateway::BillCreateRequest,
    ) -> Result<finance_types::gateway::BillSaved, String> {
        self.write(|w, snap| {
            let checks = crate::writes::bills::Checks::new(snap, &w.now)?;
            crate::writes::bills::create_bill(w, &checks, request)
        })
    }

    pub fn edit_bill(
        &self,
        request: finance_types::gateway::BillEditRequest,
    ) -> Result<(), String> {
        self.write(|w, snap| {
            let checks = crate::writes::bills::Checks::new(snap, &w.now)?;
            crate::writes::bills::edit_bill(w, &checks, request)
        })
    }

    pub fn archive_bill(&self, request: finance_types::gateway::BillRef) -> Result<(), String> {
        self.write(|w, _| crate::writes::bills::archive_bill(w, request))
    }

    pub fn save_subscription_categories(
        &self,
        request: finance_types::gateway::SubscriptionCategoriesRequest,
    ) -> Result<finance_types::gateway::SubscriptionCategoryRule, String> {
        self.write(|w, snap| {
            let checks = crate::writes::bills::Checks::new(snap, &w.now)?;
            crate::writes::bills::save_subscription_categories(w, &checks, request)
        })
    }

    pub fn link_bill_payment(
        &self,
        request: finance_types::gateway::BillMatchRequest,
    ) -> Result<(), String> {
        self.write(|w, snap| {
            let checks = crate::writes::bills::Checks::new(snap, &w.now)?;
            crate::writes::bills::link_bill_payment(w, &checks, request)
        })
    }

    pub fn decline_bill_match(
        &self,
        request: finance_types::gateway::BillMatchRequest,
    ) -> Result<(), String> {
        self.write(|w, snap| {
            let checks = crate::writes::bills::Checks::new(snap, &w.now)?;
            crate::writes::bills::decline_bill_match(w, &checks, request)
        })
    }

    pub fn status(&self) -> Result<DataStatus, String> {
        self.read(|store, _| Ok(store.status()?))
    }
}
