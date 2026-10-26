//! Finance.app: native window with macOS glass, hosting the Leptos UI.
//!
//! The UI talks to the local store only through the typed commands below.
//! Nothing listens on a network port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod update;

use finance_core::service::Service;
use finance_secrets::{Item, Secret};
use finance_types::gateway::BudgetPlan;
use finance_types::report::review::{TransactionSearchQuery, TransactionSearchResult};
use finance_types::report::{Dashboard, ReviewReport};
use finance_types::{DataStatus, UpdateStatus};
use tauri::{AppHandle, Manager, State};
use update::Updates;

/// Diagnostics: command outcome only, never any data.
fn traced<T>(name: &str, result: Result<T, String>) -> Result<T, String> {
    if std::env::var_os("FINANCE_TRACE").is_some() {
        eprintln!(
            "finance: {name} {}",
            if result.is_ok() { "ok" } else { "failed" }
        );
    }
    result
}

#[tauri::command(async)]
fn dashboard(service: State<'_, Service>) -> Result<Dashboard, String> {
    traced("dashboard", service.dashboard())
}

#[tauri::command(async)]
fn review(service: State<'_, Service>) -> Result<ReviewReport, String> {
    traced("review", service.review())
}

#[tauri::command(async)]
fn search_transactions(
    service: State<'_, Service>,
    query: TransactionSearchQuery,
) -> Result<TransactionSearchResult, String> {
    traced("search", service.search(&query))
}

/// Re-read the data from disk on the next request.
#[tauri::command(async)]
fn refresh(service: State<'_, Service>) {
    service.refresh();
}

#[tauri::command(async)]
fn save_budget_plan(service: State<'_, Service>, plan: BudgetPlan) -> Result<BudgetPlan, String> {
    traced("save_budget_plan", service.save_budget_plan(plan))
}

#[tauri::command(async)]
fn status(service: State<'_, Service>) -> Result<DataStatus, String> {
    traced("status", service.status())
}

// Transactions tab.

#[tauri::command(async)]
fn personal_transactions(
    service: State<'_, Service>,
    query: String,
) -> Result<rv::PersonalTransactions, String> {
    traced(
        "personal_transactions",
        service.personal_transactions(&query),
    )
}

#[tauri::command(async)]
fn save_personal_choice(
    service: State<'_, Service>,
    choice: rv::PersonalChoice,
) -> Result<rv::PersonalSaved, String> {
    traced("save_personal_choice", service.save_personal_choice(choice))
}

#[tauri::command(async)]
fn set_transaction_category(
    service: State<'_, Service>,
    change: rv::CategoryChange,
) -> Result<rv::CategorySaved, String> {
    traced(
        "set_transaction_category",
        service.set_transaction_category(change),
    )
}

#[tauri::command(async)]
fn review_decisions(
    service: State<'_, Service>,
    decisions: Vec<rv::ReviewDecision>,
) -> Result<rv::DecisionsSaved, String> {
    traced("review_decisions", service.review_decisions(decisions))
}

#[tauri::command(async)]
fn review_batch(
    service: State<'_, Service>,
    items: Vec<rv::BatchDecision>,
) -> Result<rv::BatchSaved, String> {
    traced("review_batch", service.review_batch(items))
}

#[tauri::command(async)]
fn review_category_options(
    service: State<'_, Service>,
    proposal: String,
) -> Result<rv::CategoryOptions, String> {
    traced(
        "review_category_options",
        service.review_category_options(&proposal),
    )
}

#[tauri::command(async)]
fn save_review_details(
    service: State<'_, Service>,
    details: rv::DetailsSave,
) -> Result<finance_types::gateway::SavedDetails, String> {
    traced("save_review_details", service.save_review_details(details))
}

#[tauri::command(async)]
fn audit_daily_review(
    service: State<'_, Service>,
    transaction: String,
) -> Result<rv::Audited, String> {
    traced(
        "audit_daily_review",
        service.audit_daily_review(&transaction),
    )
}

#[tauri::command(async)]
fn preview_consolidation(
    service: State<'_, Service>,
    request: rv::Consolidation,
) -> Result<rv::ConsolidationPreview, String> {
    traced(
        "preview_consolidation",
        service.preview_consolidation(&request),
    )
}

#[tauri::command(async)]
fn apply_consolidation(
    service: State<'_, Service>,
    request: rv::Consolidation,
) -> Result<rv::ConsolidationApplied, String> {
    traced("apply_consolidation", service.apply_consolidation(request))
}

use finance_types::report::review as rv;

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

/// macOS 26 Liquid Glass behind the transparent web view; on older systems
/// the translucent window-background material (the same look as Finder's
/// sidebars). The UI tints it with the site's page colour.
#[cfg(target_os = "macos")]
fn apply_glass(window: &tauri::WebviewWindow) {
    use window_vibrancy::{
        LiquidGlassOptions, NSGlassEffectViewStyle, NSVisualEffectMaterial, NSVisualEffectState,
        apply_liquid_glass, apply_vibrancy,
    };
    let glass = apply_liquid_glass(
        window,
        LiquidGlassOptions::new(NSGlassEffectViewStyle::Regular),
    );
    if std::env::var_os("FINANCE_TRACE").is_some() {
        eprintln!(
            "finance: liquid glass {}",
            if glass.is_ok() { "on" } else { "unavailable" }
        );
    }
    if glass.is_err() {
        apply_vibrancy(
            window,
            NSVisualEffectMaterial::UnderWindowBackground,
            Some(NSVisualEffectState::Active),
            None,
        )
        .expect("vibrancy is supported on macOS");
    }
}

fn main() {
    let path = finance_storage::default_db_path().expect("application data folder");
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Reopen with the size and position the owner left the window at.
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .manage(Service::new(path))
        .manage(Updates::default())
        .invoke_handler(tauri::generate_handler![
            dashboard,
            review,
            search_transactions,
            refresh,
            save_budget_plan,
            status,
            update_status,
            check_for_updates,
            install_update,
            set_release_token,
            forget_release_token,
            personal_transactions,
            save_personal_choice,
            set_transaction_category,
            review_decisions,
            review_batch,
            review_category_options,
            save_review_details,
            audit_daily_review,
            preview_consolidation,
            apply_consolidation,
        ])
        .setup(|app| {
            update::schedule(app.handle().clone());
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                apply_glass(&window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Finance");
}
