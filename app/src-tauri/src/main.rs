//! Finance.app: native window with macOS glass, hosting the Leptos UI.
//!
//! The UI talks to the local store only through the typed commands below.
//! Nothing listens on a network port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod update;

use std::path::PathBuf;
use std::sync::Mutex;

use finance_secrets::{Item, Secret};
use finance_storage::Store;
use finance_types::{
    AccountSummary, CategoryTotal, DataStatus, Facets, Overview, TxPage, TxQuery, UpdateStatus,
};
use tauri::{AppHandle, Manager, State};
use update::Updates;

/// The store is opened lazily so the app can start (and explain itself)
/// before any data has been migrated.
struct Db {
    path: PathBuf,
    store: Mutex<Option<Store>>,
}

/// Errors cross the boundary as short, data-free strings the UI can show.
const NO_DATA: &str = "no-data";

impl Db {
    fn with<T>(&self, f: impl FnOnce(&Store) -> finance_storage::Result<T>) -> Result<T, String> {
        let result = self.run(f);
        // Diagnostics: command outcome only, never any data.
        if std::env::var_os("FINANCE_TRACE").is_some() {
            eprintln!(
                "finance: command {}",
                if result.is_ok() { "ok" } else { "failed" }
            );
        }
        result
    }

    fn run<T>(&self, f: impl FnOnce(&Store) -> finance_storage::Result<T>) -> Result<T, String> {
        let mut guard = self.store.lock().map_err(|_| "store-poisoned".to_owned())?;
        if guard.is_none() {
            if !self.path.exists() {
                return Err(NO_DATA.to_owned());
            }
            *guard = Some(Store::open_read_only(&self.path).map_err(|e| e.to_string())?);
        }
        let store = guard.as_ref().ok_or_else(|| NO_DATA.to_owned())?;
        f(store).map_err(|e| e.to_string())
    }
}

#[tauri::command(async)]
fn overview(db: State<'_, Db>) -> Result<Overview, String> {
    db.with(Store::overview)
}

#[tauri::command(async)]
fn accounts(db: State<'_, Db>) -> Result<Vec<AccountSummary>, String> {
    db.with(Store::accounts)
}

#[tauri::command(async)]
fn facets(db: State<'_, Db>) -> Result<Facets, String> {
    db.with(Store::facets)
}

#[tauri::command(async)]
fn transactions(db: State<'_, Db>, query: TxQuery) -> Result<TxPage, String> {
    db.with(|s| s.transactions(&query))
}

#[tauri::command(async)]
fn category_totals(db: State<'_, Db>, month: String) -> Result<Vec<CategoryTotal>, String> {
    db.with(|s| s.category_totals(&month))
}

#[tauri::command(async)]
fn status(db: State<'_, Db>) -> Result<DataStatus, String> {
    db.with(Store::status)
}

#[tauri::command]
fn update_status(app: AppHandle, updates: State<'_, Updates>) -> UpdateStatus {
    updates.status(&app)
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) {
    update::check(app).await;
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    update::install(app).await
}

/// Store the read-only release token. It goes straight to the Keychain and is
/// never returned to the UI.
#[tauri::command]
async fn set_release_token(app: AppHandle, token: String) -> Result<(), String> {
    let token = Secret::new(token.trim().to_owned());
    if token.expose().is_empty() {
        return Err("The token is empty".to_owned());
    }
    finance_secrets::set(Item::ReleaseToken, &token).map_err(|e| e.to_string())?;
    update::check(app).await;
    Ok(())
}

#[tauri::command]
fn forget_release_token() -> Result<(), String> {
    finance_secrets::delete(Item::ReleaseToken).map_err(|e| e.to_string())
}

fn main() {
    let path = finance_storage::default_db_path().expect("application data folder");
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Db {
            path,
            store: Mutex::new(None),
        })
        .manage(Updates::default())
        .invoke_handler(tauri::generate_handler![
            overview,
            accounts,
            facets,
            transactions,
            category_totals,
            status,
            update_status,
            check_for_updates,
            install_update,
            set_release_token,
            forget_release_token,
        ])
        .setup(|app| {
            update::schedule(app.handle().clone());
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use window_vibrancy::{
                    NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy,
                };
                // Sidebar material is the system's translucent, wallpaper-tinted glass.
                apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::Sidebar,
                    Some(NSVisualEffectState::Active),
                    None,
                )
                .expect("vibrancy is supported on macOS");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Finance");
}
