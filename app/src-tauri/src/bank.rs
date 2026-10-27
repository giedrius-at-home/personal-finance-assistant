//! Bank sync commands and the daily scheduler. Syncs make blocking HTTP
//! calls, so they run on their own thread, never on the async runtime.

use std::time::Duration;

use finance_bank::{Credentials, Syncer};
use finance_core::service::Service;
use finance_types::bank::{
    BankConsent, BankCredentials, BankLinkConfirm, BankLinkProposal, BankLinkStart, BankSyncReport,
    BankSyncResult,
};
use tauri::{AppHandle, Emitter, Manager, State};

/// Event the UI listens to so every screen reloads after a scheduled sync.
pub const SYNCED: &str = "bank-sync";
/// How often the scheduler looks for due banks; each bank itself syncs once
/// a day (hourly retries after a failure), as the old importer's agent did.
const EVERY: Duration = Duration::from_secs(15 * 60);
/// Let the window load before the first check.
const FIRST: Duration = Duration::from_secs(60);

/// Run `f` on a plain thread and wait for it without blocking the runtime.
async fn off_runtime<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Service, &Syncer) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let worker = std::thread::spawn(move || f(&app.state::<Service>(), &app.state::<Syncer>()));
    tauri::async_runtime::spawn_blocking(move || worker.join())
        .await
        .map_err(|_| "Bank sync stopped".to_owned())?
        .map_err(|_| "Bank sync stopped".to_owned())?
}

#[tauri::command(async)]
pub fn bank_sync_status(
    service: State<'_, Service>,
    syncer: State<'_, Syncer>,
) -> Result<BankSyncReport, String> {
    crate::traced("bank_sync_status", syncer.report(&service))
}

#[tauri::command]
pub async fn sync_bank(app: AppHandle, bank: String) -> Result<BankSyncResult, String> {
    let handle = app.clone();
    let result = off_runtime(app, move |service, syncer| syncer.sync(service, &bank)).await;
    if let Ok(r) = &result {
        wake_ai(&handle, std::slice::from_ref(r));
    }
    crate::traced("sync_bank", result)
}

/// New transactions get category suggestions right away.
fn wake_ai(app: &AppHandle, results: &[BankSyncResult]) {
    if results.iter().any(|r| r.created.is_some_and(|n| n > 0)) {
        app.state::<finance_ai::Assistant>().wake();
    }
}

#[tauri::command]
pub async fn renew_bank_consent(
    app: AppHandle,
    consent: BankConsent,
) -> Result<BankSyncResult, String> {
    crate::traced(
        "renew_bank_consent",
        off_runtime(app, move |service, syncer| syncer.renew(service, &consent)).await,
    )
}

#[tauri::command(async)]
pub fn adopt_bank_consents(
    service: State<'_, Service>,
    syncer: State<'_, Syncer>,
) -> Result<Vec<String>, String> {
    crate::traced("adopt_bank_consents", syncer.adopt_legacy(&service))
}

/// Ask Enable Banking for the approval page and open it in the browser.
#[tauri::command]
pub async fn bank_link_start(app: AppHandle, start: BankLinkStart) -> Result<String, String> {
    let url = off_runtime(app, move |_, syncer| syncer.link_start(&start)).await;
    let url = crate::traced("bank_link_start", url)?;
    // `open` hands the https address to the default browser.
    std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|_| "The browser could not be opened.".to_owned())?;
    Ok(url)
}

#[tauri::command]
pub async fn bank_link_finish(
    app: AppHandle,
    bank: String,
    address: String,
) -> Result<BankLinkProposal, String> {
    crate::traced(
        "bank_link_finish",
        off_runtime(app, move |service, syncer| {
            syncer.link_finish(service, &bank, &address)
        })
        .await,
    )
}

#[tauri::command]
pub async fn bank_link_confirm(
    app: AppHandle,
    confirm: BankLinkConfirm,
) -> Result<BankSyncResult, String> {
    crate::traced(
        "bank_link_confirm",
        off_runtime(app, move |service, syncer| {
            syncer.link_confirm(service, &confirm)
        })
        .await,
    )
}

/// Store the Enable Banking application id and key. They go straight to the
/// Keychain and are never returned to the UI.
#[tauri::command(async)]
pub fn set_bank_credentials(credentials: BankCredentials) -> Result<(), String> {
    Credentials::store(&credentials).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn forget_bank_credentials() -> Result<(), String> {
    Credentials::forget().map_err(|e| e.to_string())
}

/// Sync every due bank now and then, for as long as the app runs.
pub fn schedule(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST);
        loop {
            let results = app.state::<Syncer>().sync_due(&app.state::<Service>());
            if crate::tracing() {
                for r in &results {
                    eprintln!("finance: scheduled sync {} {:?}", r.bank, r.status);
                }
            }
            if !results.is_empty() {
                let _ = app.emit(SYNCED, ());
            }
            wake_ai(&app, &results);
            std::thread::sleep(EVERY);
        }
    });
}
