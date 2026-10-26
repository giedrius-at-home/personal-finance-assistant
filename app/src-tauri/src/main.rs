//! Finance.app: native window with macOS glass, hosting the Leptos UI.
//!
//! The UI talks to the local store only through the typed commands below.
//! Nothing listens on a network port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod update;

use finance_core::service::Service;
use finance_secrets::{Item, Secret};
use finance_types::gateway::BudgetPlan;
use finance_types::report::closure::{
    AccountRoleDraft, ClosureAction, ClosureView, PotDraft, PotView, PotsView, UnclassifiedAccounts,
};
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
fn save_spending_group(
    service: State<'_, Service>,
    category: String,
    group: Option<String>,
    revision: u32,
) -> Result<(), String> {
    traced(
        "save_spending_group",
        service.save_spending_group(&category, group.as_deref(), revision),
    )
}

#[tauri::command(async)]
fn create_spending_group(service: State<'_, Service>, group: String) -> Result<String, String> {
    traced(
        "create_spending_group",
        service.create_spending_group(&group),
    )
}

#[tauri::command(async)]
fn preview_category_consolidation(
    service: State<'_, Service>,
    source: String,
    target: String,
) -> Result<finance_types::report::catalog::ConsolidationPreview, String> {
    traced(
        "preview_category_consolidation",
        service.preview_category_consolidation(&source, &target),
    )
}

#[tauri::command(async)]
fn apply_category_consolidation(
    service: State<'_, Service>,
    source: String,
    target: String,
    token: Option<String>,
) -> Result<finance_types::report::catalog::ConsolidationResult, String> {
    traced(
        "apply_category_consolidation",
        service.apply_category_consolidation(&source, &target, token.as_deref()),
    )
}

#[tauri::command(async)]
fn create_planned_payment(
    service: State<'_, Service>,
    request: finance_types::gateway::PlannedPaymentRequest,
) -> Result<finance_types::gateway::PlannedPayment, String> {
    traced(
        "create_planned_payment",
        service.create_planned_payment(&request),
    )
}

/// Snake-case argument keys (`plan_id`), as the UI and devserver send them.
#[tauri::command(async, rename_all = "snake_case")]
fn archive_planned_payment(service: State<'_, Service>, plan_id: String) -> Result<(), String> {
    traced(
        "archive_planned_payment",
        service.archive_planned_payment(&plan_id),
    )
}

#[tauri::command(async)]
fn closure(service: State<'_, Service>, month: String) -> Result<ClosureView, String> {
    traced("closure", service.closure(&month))
}

#[tauri::command(async)]
fn closure_action(
    service: State<'_, Service>,
    action: ClosureAction,
) -> Result<ClosureView, String> {
    traced("closure_action", service.closure_action(action))
}

#[tauri::command(async)]
fn closure_pots(service: State<'_, Service>) -> Result<PotsView, String> {
    traced("closure_pots", service.pots())
}

#[tauri::command(async)]
fn save_pot(service: State<'_, Service>, pot: PotDraft) -> Result<PotView, String> {
    traced("save_pot", service.save_pot(pot))
}

#[tauri::command(async)]
fn save_account_role(service: State<'_, Service>, role: AccountRoleDraft) -> Result<(), String> {
    traced("save_account_role", service.save_account_role(role))
}

#[tauri::command(async)]
fn unclassified_accounts(service: State<'_, Service>) -> Result<UnclassifiedAccounts, String> {
    traced("unclassified_accounts", service.unclassified_accounts())
}

#[tauri::command(async)]
fn status(service: State<'_, Service>) -> Result<DataStatus, String> {
    traced("status", service.status())
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

#[tauri::command(async)]
fn create_bill(
    service: State<'_, Service>,
    request: finance_types::gateway::BillCreateRequest,
) -> Result<finance_types::gateway::BillSaved, String> {
    traced("create_bill", service.create_bill(request))
}

#[tauri::command(async)]
fn edit_bill(
    service: State<'_, Service>,
    request: finance_types::gateway::BillEditRequest,
) -> Result<(), String> {
    traced("edit_bill", service.edit_bill(request))
}

#[tauri::command(async)]
fn archive_bill(
    service: State<'_, Service>,
    request: finance_types::gateway::BillRef,
) -> Result<(), String> {
    traced("archive_bill", service.archive_bill(request))
}

#[tauri::command(async)]
fn save_subscription_categories(
    service: State<'_, Service>,
    request: finance_types::gateway::SubscriptionCategoriesRequest,
) -> Result<finance_types::gateway::SubscriptionCategoryRule, String> {
    traced(
        "save_subscription_categories",
        service.save_subscription_categories(request),
    )
}

#[tauri::command(async)]
fn link_bill_payment(
    service: State<'_, Service>,
    request: finance_types::gateway::BillMatchRequest,
) -> Result<(), String> {
    traced("link_bill_payment", service.link_bill_payment(request))
}

#[tauri::command(async)]
fn decline_bill_match(
    service: State<'_, Service>,
    request: finance_types::gateway::BillMatchRequest,
) -> Result<(), String> {
    traced("decline_bill_match", service.decline_bill_match(request))
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
            save_spending_group,
            create_spending_group,
            preview_category_consolidation,
            apply_category_consolidation,
            status,
            update_status,
            check_for_updates,
            install_update,
            set_release_token,
            forget_release_token,
            create_bill,
            edit_bill,
            archive_bill,
            save_subscription_categories,
            link_bill_payment,
            decline_bill_match,
            create_planned_payment,
            archive_planned_payment,
            closure,
            closure_action,
            closure_pots,
            save_pot,
            save_account_role,
            unclassified_accounts,
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
