//! The app's data service: one place that opens the store, caches the
//! snapshot between refreshes and runs every read and write the screens ask
//! for. The Tauri commands and the development server are thin wrappers.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use finance_storage::Store;
use finance_types::DataStatus;
use finance_types::gateway::{Wish, WishDraft, WishPurchase, WishSettings};
use finance_types::report::closure::{
    AccountRoleDraft, ClosureAction, ClosureView, PotDraft, PotView, PotsView, UnclassifiedAccounts,
};
use finance_types::report::review::{self as rv, TransactionSearchQuery, TransactionSearchResult};
use finance_types::report::wishes::WishesView;
use finance_types::report::{Dashboard, ReviewReport};
use jiff::Zoned;
use rust_decimal::Decimal;

use crate::alerts::{Notification, Notifier, Outcome};
use crate::daily::SeriesForecaster;
use crate::forecast_model::{ForecastModel, ModelRunner, RunStatus};
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
    if target.exists() {
        // Another store of this session backed up this same second.
        return Ok(());
    }
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

/// No delivery channel is registered (development server, tests).
struct Unavailable;

impl Notifier for Unavailable {
    fn send(&self, _: &Notification) -> Outcome {
        Outcome::Unavailable
    }
}

/// Errors cross to the UI as short, data-free strings.
pub const NO_DATA: &str = "no-data";

#[derive(Debug)]
struct Open {
    store: Store,
    snapshot: Option<Snapshot>,
}

/// Where alerts go: the app's desktop notifications, once it registered them.
#[derive(Default)]
struct AlertSink(Mutex<Option<Arc<dyn Notifier + Send + Sync>>>);

impl std::fmt::Debug for AlertSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AlertSink")
    }
}

#[derive(Debug)]
pub struct Service {
    path: PathBuf,
    state: Mutex<Option<Open>>,
    /// The read-write store, opened on the first write.
    writer: Mutex<Option<Store>>,
    alerts: AlertSink,
    /// The daily forecast's model (TimesFM), when the app has one.
    model: Mutex<Option<Arc<ModelRunner>>>,
}

impl Service {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
            writer: Mutex::new(None),
            alerts: AlertSink::default(),
            model: Mutex::new(None),
        }
    }

    /// Deliver alerts through `notifier` from now on (the app registers its
    /// desktop notifications at start).
    pub fn set_alert_notifier(&self, notifier: Arc<dyn Notifier + Send + Sync>) {
        if let Ok(mut sink) = self.alerts.0.lock() {
            *sink = Some(notifier);
        }
    }

    /// Evaluate the alerts now through the registered notifier; without one,
    /// a due alert is recorded as `unavailable` (or `disabled` when the owner
    /// turned notifications off), as Python recorded it.
    pub fn evaluate_alerts_now(&self) -> Result<Vec<crate::alerts::Decision>, String> {
        let notifier = self.alerts.0.lock().ok().and_then(|s| s.clone());
        match notifier {
            Some(n) => self.evaluate_alerts(jiff::Timestamp::now(), n.as_ref()),
            None => self.evaluate_alerts(jiff::Timestamp::now(), &Unavailable),
        }
    }

    /// Use `model` for daily forecasts from now on (`None`: the weekday
    /// baseline only). Forecasts it computed before are dropped.
    pub fn set_forecast_model(&self, model: Option<ForecastModel>) {
        if let Ok(mut slot) = self.model.lock() {
            *slot = model.map(|m| Arc::new(ModelRunner::new(m)));
        }
    }

    /// The forecast model's background runs, when there is a model.
    pub fn forecast_model_status(&self) -> Option<RunStatus> {
        self.runner().map(|r| r.status())
    }

    fn runner(&self) -> Option<Arc<ModelRunner>> {
        self.model.lock().ok().and_then(|m| m.clone())
    }

    /// Run `f` with the daily forecast's model answering from forecasts it
    /// already computed; a miss starts computing them in the background
    /// (the model's `on_done` says when they are ready).
    fn read_forecast<T>(
        &self,
        f: impl FnOnce(&Store, &Snapshot, &Zoned, Option<&dyn SeriesForecaster>) -> crate::Result<T>,
    ) -> Result<T, String> {
        let runner = self.runner();
        let now = Zoned::now();
        self.read(|store, snap| {
            let Some(runner) = &runner else {
                return f(store, snap, &now, None);
            };
            let cached = runner.cached();
            let value = f(store, snap, &now, Some(&cached))?;
            if cached.missed() {
                runner.start(&cached, self.path.clone(), snap.clone(), now.clone());
            }
            Ok(value)
        })
    }

    /// [`Service::read_forecast`] as of `now`, in the data's time zone.
    pub(crate) fn read_forecast_at<T>(
        &self,
        now: jiff::Timestamp,
        f: impl FnOnce(&Store, &Snapshot, &Zoned, Option<&dyn SeriesForecaster>) -> crate::Result<T>,
    ) -> Result<T, String> {
        let runner = self.runner();
        self.read(|store, snap| {
            let zoned = now.to_zoned(snap.zone.clone());
            let Some(runner) = &runner else {
                return f(store, snap, &zoned, None);
            };
            let cached = runner.cached();
            let value = f(store, snap, &zoned, Some(&cached))?;
            if cached.missed() {
                runner.start(&cached, self.path.clone(), snap.clone(), zoned.clone());
            }
            Ok(value)
        })
    }

    /// Run `f` on the store and a current snapshot (loaded once, then reused
    /// until [`Service::refresh`]).
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Store, &Snapshot) -> crate::Result<T>,
    ) -> Result<T, String> {
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

    /// Run `f` on the store alone, without building a snapshot.
    pub(crate) fn read_store<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> crate::Result<T>,
    ) -> Result<T, String> {
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
        let open = guard.as_ref().ok_or_else(|| NO_DATA.to_owned())?;
        f(open.store.connection()).map_err(|e| e.to_string())
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
        self.write_then(f, true)
    }

    /// [`Service::write`] for background work: the cached snapshot is
    /// dropped only when the transaction changed rows.
    pub(crate) fn write_if_changed<T>(
        &self,
        f: impl FnOnce(&Writer<'_>, &Snapshot) -> crate::Result<T>,
    ) -> Result<T, String> {
        self.write_then(f, false)
    }

    /// A write that needs no snapshot and leaves the cached one in place;
    /// the caller refreshes when its work is done.
    pub(crate) fn write_raw<T>(
        &self,
        f: impl FnOnce(&Writer<'_>) -> crate::Result<T>,
    ) -> Result<T, String> {
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| "store-poisoned".to_owned())?;
        if guard.is_none() {
            if !self.path.exists() {
                return Err(NO_DATA.to_owned());
            }
            backup(&self.path).map_err(|e| format!("backup failed: {e}"))?;
            *guard = Some(Store::open(&self.path).map_err(|e| e.to_string())?);
        }
        let store = guard.as_mut().ok_or_else(|| NO_DATA.to_owned())?;
        (|| {
            let writer = Writer {
                tx: store.connection_mut().transaction()?,
                now: Zoned::now(),
            };
            let value = f(&writer)?;
            writer.tx.commit()?;
            Ok::<T, crate::Error>(value)
        })()
        .map_err(|e| e.to_string())
    }

    fn write_then<T>(
        &self,
        f: impl FnOnce(&Writer<'_>, &Snapshot) -> crate::Result<T>,
        always_refresh: bool,
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
        let before = store.connection().total_changes();
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
        let changed = store.connection().total_changes() != before;
        drop(guard);
        if always_refresh || changed {
            self.refresh();
        }
        result
    }

    /// Run `f` on the cached snapshot.
    pub(crate) fn read_snapshot<T>(&self, f: impl FnOnce(&Snapshot) -> T) -> Result<T, String> {
        self.read(|_, snap| Ok(f(snap)))
    }

    /// Run `f` on the read-write connection, which manages its own
    /// transactions (the alert lifecycle commits per candidate). Opens and
    /// backs up like [`Service::write`]; the cached snapshot stays, as `f`
    /// writes only tables the snapshot does not hold.
    pub(crate) fn with_connection<T>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> crate::Result<T>,
    ) -> Result<T, String> {
        if !self.path.exists() {
            return Err(NO_DATA.to_owned());
        }
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| "store-poisoned".to_owned())?;
        if guard.is_none() {
            backup(&self.path).map_err(|e| format!("backup failed: {e}"))?;
            *guard = Some(Store::open(&self.path).map_err(|e| e.to_string())?);
        }
        let store = guard.as_mut().ok_or_else(|| NO_DATA.to_owned())?;
        f(store.connection_mut()).map_err(|e| e.to_string())
    }

    /// The dashboard as of `now` (the spending email builds it for its slot).
    pub fn dashboard_at(&self, now: &Zoned) -> Result<Dashboard, String> {
        self.read_forecast_at(now.timestamp(), |store, snap, now, model| {
            dashboard::dashboard(store, snap, now, model)
        })
    }

    pub fn dashboard(&self) -> Result<Dashboard, String> {
        self.read_forecast(|store, snap, now, model| dashboard::dashboard(store, snap, now, model))
    }

    pub fn wishes(&self) -> Result<WishesView, String> {
        self.read_forecast(|store, snap, now, model| {
            let dash = dashboard::dashboard(store, snap, now, model)?;
            let wishes = crate::wishes::store::load_wishes(store.connection())?;
            let settings = crate::wishes::store::load_wish_settings(store.connection())?;
            Ok(crate::wishes::wishes_view(&dash, &wishes, &settings))
        })
    }

    pub fn save_wish(&self, draft: WishDraft) -> Result<Wish, String> {
        self.write(|w, snap| crate::writes::wishes::save_wish(w, snap, draft))
    }

    pub fn record_wish_saved(&self, wish_id: &str, saved: Decimal) -> Result<Wish, String> {
        self.write(|w, _| crate::writes::wishes::record_saved(w, wish_id, saved))
    }

    pub fn drop_wish(&self, wish_id: &str) -> Result<(), String> {
        self.write(|w, _| crate::writes::wishes::drop_wish(w, wish_id))
    }

    /// Buys a wish, recording the cuts month 0 makes for it. The view is read
    /// first and the write follows, so they do not share one lock.
    pub fn mark_wish_bought(&self, wish_id: &str, purchase: WishPurchase) -> Result<Wish, String> {
        let cuts = crate::wishes::month_zero_cuts(&self.wishes()?, wish_id);
        self.write(|w, snap| crate::writes::wishes::mark_bought(w, snap, wish_id, purchase, &cuts))
    }

    pub fn save_wish_settings(&self, settings: WishSettings) -> Result<WishSettings, String> {
        self.write(|w, _| crate::writes::wishes::save_wish_settings(w, settings))
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

    pub fn create_planned_payment(
        &self,
        request: &finance_types::gateway::PlannedPaymentRequest,
    ) -> Result<finance_types::gateway::PlannedPayment, String> {
        self.write(|w, snap| crate::writes::plans::create_planned_payment(w, snap, request))
    }

    pub fn archive_planned_payment(&self, plan_id: &str) -> Result<(), String> {
        self.write(|w, _| crate::writes::plans::archive_planned_payment(w, plan_id))
    }

    /// The closure screen of `month` (`YYYY-MM-01`).
    pub fn closure(&self, month: &str) -> Result<ClosureView, String> {
        self.read_forecast(|store, snap, now, model| {
            crate::closure::closure_view(
                store,
                snap,
                crate::closure::parse_month(month)?,
                now,
                model,
            )
        })
    }

    /// Run one closure step, then return the month's screen as it now stands.
    /// A sync or finalize first re-reads the data in its own write (the
    /// screen syncs the banks before it asks), so late arrivals are flagged
    /// even when the step itself is refused.
    pub fn closure_action(&self, action: ClosureAction) -> Result<ClosureView, String> {
        if matches!(
            action,
            ClosureAction::Sync { .. } | ClosureAction::Finalize { .. }
        ) {
            self.write(|w, snap| crate::writes::closure::prepare(w, snap, &action))?;
        }
        let month = self.write(|w, snap| crate::writes::closure::act(w, snap, action))?;
        self.closure(&month.to_string())
    }

    /// Every pot (archived too) and the 50 newest pot entries.
    pub fn pots(&self) -> Result<PotsView, String> {
        self.read(|store, _| crate::closure::pots_view(store.connection()))
    }

    pub fn save_pot(&self, pot: PotDraft) -> Result<PotView, String> {
        self.write(|w, snap| crate::writes::closure::save_pot(w, snap, pot))
    }

    /// Save an account's closure role; open transfers follow a new route.
    pub fn save_account_role(&self, role: AccountRoleDraft) -> Result<(), String> {
        self.write(|w, snap| crate::writes::closure::save_account_role(w, snap, role))
    }

    /// Accounts without a role, and outflows that look like savings transfers.
    pub fn unclassified_accounts(&self) -> Result<UnclassifiedAccounts, String> {
        self.read(|_, snap| Ok(crate::closure::unclassified_accounts(snap)))
    }

    pub fn status(&self) -> Result<DataStatus, String> {
        self.read(|store, _| Ok(store.status()?))
    }

    /// Spending → Organise: set (or clear, with `None`) a category's group.
    pub fn save_spending_group(
        &self,
        category: &str,
        group: Option<&str>,
        revision: u32,
    ) -> Result<(), String> {
        self.write(|w, snap| {
            crate::writes::groups::save_group_mapping(
                w,
                &crate::writes::groups::known_categories(snap),
                &snap.custom_groups,
                category,
                group,
                revision,
            )
        })
    }

    /// The Import, Status and Settings screens' data.
    pub fn manage_workspace(&self) -> Result<finance_types::manage::ManageWorkspace, String> {
        let last_write = last_write(&self.path);
        self.read(|store, snap| crate::manage::workspace(store, snap, last_write))
    }

    pub fn banks(&self) -> Result<finance_types::manage::BanksReport, String> {
        self.read(|store, _| crate::manage::banks(store))
    }

    pub fn preview_csv_import(
        &self,
        upload: finance_types::manage::CsvUpload,
    ) -> Result<finance_types::manage::CsvPreview, String> {
        self.read(|store, snap| {
            let conn = store.connection();
            let targets = crate::manage::import_targets(conn, snap)?;
            crate::writes::csv_import::preview(
                conn,
                &targets,
                &upload.content,
                &upload.filename,
                &upload.account_id,
            )
        })
    }

    /// Spending → Organise: add an owner-named group; returns its stored name.
    pub fn create_spending_group(&self, group: &str) -> Result<String, String> {
        self.write(|w, _| crate::writes::groups::create_spending_group(w, group))
    }

    pub fn commit_csv_import(
        &self,
        commit: finance_types::manage::CsvCommit,
    ) -> Result<finance_types::manage::CsvCommitResult, String> {
        self.write(|w, snap| {
            let targets = crate::manage::import_targets(&w.tx, snap)?;
            crate::writes::csv_import::commit(w, &targets, &commit)
        })
    }

    pub fn preview_settings(
        &self,
        form: finance_types::manage::SettingsForm,
    ) -> Result<finance_types::manage::SettingsPreview, String> {
        self.read(|store, snap| {
            crate::writes::settings::preview(
                store.connection(),
                &form,
                &crate::manage::salary_accounts(snap),
            )
        })
    }

    pub fn apply_settings(
        &self,
        apply: finance_types::manage::SettingsApply,
    ) -> Result<i64, String> {
        self.write(|w, snap| {
            crate::writes::settings::apply(w, &apply, &crate::manage::salary_accounts(snap))
        })
    }

    pub fn rollback_settings(&self, revision: i64) -> Result<(), String> {
        self.write(|w, _| crate::writes::settings::rollback(w, revision))
    }

    // Transactions tab ------------------------------------------------------

    pub fn personal_transactions(&self, query: &str) -> Result<rv::PersonalTransactions, String> {
        self.read(|_, snap| crate::writes::transactions::personal_transactions(snap, query))
    }

    pub fn save_personal_choice(
        &self,
        choice: rv::PersonalChoice,
    ) -> Result<rv::PersonalSaved, String> {
        self.write(|w, snap| crate::writes::transactions::save_personal_choice(w, snap, choice))
    }

    pub fn set_transaction_category(
        &self,
        change: rv::CategoryChange,
    ) -> Result<rv::CategorySaved, String> {
        self.write(|w, snap| crate::writes::transactions::set_transaction_category(w, snap, change))
    }

    pub fn review_decisions(
        &self,
        decisions: Vec<rv::ReviewDecision>,
    ) -> Result<rv::DecisionsSaved, String> {
        self.write(|w, snap| crate::writes::review::review_decisions(w, snap, decisions))
    }

    pub fn review_batch(&self, items: Vec<rv::BatchDecision>) -> Result<rv::BatchSaved, String> {
        self.write(|w, snap| crate::writes::review::review_batch(w, snap, items))
    }

    pub fn review_category_options(&self, proposal: &str) -> Result<rv::CategoryOptions, String> {
        self.read(|store, snap| {
            crate::writes::review::category_options(store.connection(), snap, proposal)
        })
    }

    pub fn save_review_details(
        &self,
        details: rv::DetailsSave,
    ) -> Result<finance_types::gateway::SavedDetails, String> {
        self.write(|w, _| crate::writes::transactions::save_review_details(w, details))
    }

    pub fn audit_daily_review(&self, transaction: &str) -> Result<rv::Audited, String> {
        self.write(|w, _| crate::writes::transactions::audit_daily_review(w, transaction))
    }

    pub fn preview_consolidation(
        &self,
        request: &rv::Consolidation,
    ) -> Result<rv::ConsolidationPreview, String> {
        self.read(|_, snap| crate::writes::transactions::preview_consolidation(snap, request))
    }

    pub fn apply_consolidation(
        &self,
        request: rv::Consolidation,
    ) -> Result<rv::ConsolidationApplied, String> {
        self.write(|w, snap| crate::writes::transactions::apply_consolidation(w, snap, request))
    }
}

/// When the database (or its write-ahead log) last changed, RFC 3339 UTC.
fn last_write(path: &std::path::Path) -> Option<String> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    [path.to_path_buf(), PathBuf::from(wal)]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
        .max()
        .and_then(|t| jiff::Timestamp::try_from(t).ok())
        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}
