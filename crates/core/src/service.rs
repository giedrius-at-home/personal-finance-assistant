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

use crate::{Ctx, Snapshot, dashboard, review};

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
}

impl Service {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
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

    pub fn status(&self) -> Result<DataStatus, String> {
        self.read(|store, _| Ok(store.status()?))
    }
}
