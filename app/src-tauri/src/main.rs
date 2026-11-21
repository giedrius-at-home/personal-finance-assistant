//! Finance.app: native window with macOS glass, hosting the Leptos UI.
//!
//! The UI talks to the local store only through the typed commands below.
//! Nothing listens on a network port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod bank;
mod model;
mod notify;
mod update;

use finance_core::service::Service;
use finance_secrets::{Item, Secret};
use finance_types::gateway::{BudgetPlan, EventPick, Wish, WishDraft, WishPurchase, WishSettings};
use finance_types::report::closure::{
    AccountRoleDraft, ClosureAction, ClosureView, UnclassifiedAccounts,
};
use finance_types::report::events::EventsView;
use finance_types::report::review::{TransactionSearchQuery, TransactionSearchResult};
use finance_types::report::wishes::WishesView;
use finance_types::report::{Dashboard, ReviewReport};
use finance_types::{DataStatus, UpdateStatus};
use rust_decimal::Decimal;
use tauri::{AppHandle, Manager, State};
use update::Updates;

/// `FINANCE_TRACE` is set: print outcomes (never data) to stderr.
fn tracing() -> bool {
    std::env::var_os("FINANCE_TRACE").is_some()
}

/// Diagnostics: command outcome only, never any data.
fn traced<T>(name: &str, result: Result<T, String>) -> Result<T, String> {
    if tracing() {
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

/// Re-read the data from disk on the next request, then evaluate the
/// alerts against it (in the background).
#[tauri::command(async)]
fn refresh(app: AppHandle) {
    app.state::<Service>().refresh();
    notify::evaluate_alerts(app);
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

#[tauri::command(async, rename_all = "snake_case")]
fn tie_planned_payment(
    service: State<'_, Service>,
    plan_id: String,
    transaction_id: Option<String>,
) -> Result<finance_types::gateway::PlannedPayment, String> {
    traced(
        "tie_planned_payment",
        service.tie_planned_payment(&plan_id, transaction_id.as_deref()),
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
fn wishes(service: State<'_, Service>) -> Result<WishesView, String> {
    traced("wishes", service.wishes())
}

#[tauri::command(async)]
fn save_wish(service: State<'_, Service>, draft: WishDraft) -> Result<Wish, String> {
    traced("save_wish", service.save_wish(draft))
}

#[tauri::command(async, rename_all = "snake_case")]
fn record_wish_saved(
    service: State<'_, Service>,
    wish_id: String,
    saved: Decimal,
) -> Result<Wish, String> {
    traced(
        "record_wish_saved",
        service.record_wish_saved(&wish_id, saved),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn events(service: State<'_, Service>) -> Result<EventsView, String> {
    traced("events", service.events())
}

#[tauri::command(async, rename_all = "snake_case")]
fn plan_wish_as_event(
    service: State<'_, Service>,
    wish_id: String,
    starts_on: String,
    ends_on: String,
) -> Result<Wish, String> {
    traced(
        "plan_wish_as_event",
        service.plan_wish_as_event(&wish_id, &starts_on, &ends_on),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn wish_back_from_event(service: State<'_, Service>, wish_id: String) -> Result<Wish, String> {
    traced(
        "wish_back_from_event",
        service.wish_back_from_event(&wish_id),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn move_wish(
    service: State<'_, Service>,
    wish_id: String,
    event_id: Option<String>,
) -> Result<Wish, String> {
    traced(
        "move_wish",
        service.move_wish(&wish_id, event_id.as_deref()),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn set_event_transaction(
    service: State<'_, Service>,
    wish_id: String,
    transaction_id: String,
    pick: EventPick,
) -> Result<Wish, String> {
    traced(
        "set_event_transaction",
        service.set_event_transaction(&wish_id, &transaction_id, pick),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn set_wish_must(service: State<'_, Service>, wish_id: String, must: bool) -> Result<Wish, String> {
    traced("set_wish_must", service.set_wish_must(&wish_id, must))
}

#[tauri::command(async, rename_all = "snake_case")]
fn drop_wish(service: State<'_, Service>, wish_id: String) -> Result<(), String> {
    traced("drop_wish", service.drop_wish(&wish_id))
}

#[tauri::command(async, rename_all = "snake_case")]
fn mark_wish_bought(
    service: State<'_, Service>,
    wish_id: String,
    purchase: WishPurchase,
) -> Result<Wish, String> {
    traced(
        "mark_wish_bought",
        service.mark_wish_bought(&wish_id, purchase),
    )
}

#[tauri::command(async, rename_all = "snake_case")]
fn start_wish(
    service: State<'_, Service>,
    wish_id: String,
    match_category: Option<String>,
) -> Result<Wish, String> {
    traced("start_wish", service.start_wish(&wish_id, match_category))
}

#[tauri::command(async)]
fn save_wish_settings(
    service: State<'_, Service>,
    settings: WishSettings,
) -> Result<WishSettings, String> {
    traced("save_wish_settings", service.save_wish_settings(settings))
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

/// Store the read-only release token. It goes straight to the secret store and is
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

#[tauri::command(async)]
fn manage_workspace(
    service: State<'_, Service>,
) -> Result<finance_types::manage::ManageWorkspace, String> {
    traced("manage_workspace", service.manage_workspace())
}

#[tauri::command(async)]
fn banks(service: State<'_, Service>) -> Result<finance_types::manage::BanksReport, String> {
    traced("banks", service.banks())
}

#[tauri::command(async)]
fn preview_csv_import(
    service: State<'_, Service>,
    upload: finance_types::manage::CsvUpload,
) -> Result<finance_types::manage::CsvPreview, String> {
    traced("preview_csv_import", service.preview_csv_import(upload))
}

#[tauri::command(async)]
fn commit_csv_import(
    service: State<'_, Service>,
    commit: finance_types::manage::CsvCommit,
) -> Result<finance_types::manage::CsvCommitResult, String> {
    traced("commit_csv_import", service.commit_csv_import(commit))
}

#[tauri::command(async)]
fn preview_settings(
    service: State<'_, Service>,
    form: finance_types::manage::SettingsForm,
) -> Result<finance_types::manage::SettingsPreview, String> {
    traced("preview_settings", service.preview_settings(form))
}

#[tauri::command(async)]
fn apply_settings(
    service: State<'_, Service>,
    apply: finance_types::manage::SettingsApply,
) -> Result<i64, String> {
    traced("apply_settings", service.apply_settings(apply))
}

#[tauri::command(async)]
fn rollback_settings(service: State<'_, Service>, revision: i64) -> Result<(), String> {
    traced("rollback_settings", service.rollback_settings(revision))
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
        .plugin(tauri_plugin_notification::init())
        .manage(Service::new(path))
        .manage(Updates::default())
        .manage(finance_bank::Syncer::default())
        .manage(model::Model::default())
        .manage(finance_ai::Assistant::default())
        .invoke_handler(tauri::generate_handler![
            dashboard,
            review,
            search_transactions,
            refresh,
            save_budget_plan,
            save_spending_group,
            create_spending_group,
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
            tie_planned_payment,
            closure,
            closure_action,
            wishes,
            save_wish,
            record_wish_saved,
            set_wish_must,
            events,
            plan_wish_as_event,
            wish_back_from_event,
            move_wish,
            set_event_transaction,
            drop_wish,
            mark_wish_bought,
            start_wish,
            save_wish_settings,
            save_account_role,
            unclassified_accounts,
            manage_workspace,
            banks,
            preview_csv_import,
            commit_csv_import,
            preview_settings,
            apply_settings,
            rollback_settings,
            personal_transactions,
            save_personal_choice,
            set_transaction_category,
            review_decisions,
            review_batch,
            save_review_details,
            audit_daily_review,
            preview_consolidation,
            apply_consolidation,
            bank::bank_sync_status,
            bank::sync_bank,
            bank::renew_bank_consent,
            bank::adopt_bank_consents,
            bank::set_bank_credentials,
            bank::forget_bank_credentials,
            bank::bank_link_start,
            bank::bank_link_finish,
            bank::bank_link_confirm,
            bank::reconcile_bank_account,
            notify::email_status,
            notify::save_email_settings,
            notify::set_smtp_password,
            notify::forget_smtp_password,
            notify::send_test_email,
            model::forecast_model_status,
            model::download_forecast_model,
            ai::ai_status,
            ai::save_ai_settings,
            ai::set_typesafe_key,
            ai::forget_typesafe_key,
            ai::suggest_categories,
            ai::suggest_spending_group,
            ai::research_merchant,
        ])
        .setup(|app| {
            update::schedule(app.handle().clone());
            bank::schedule(app.handle().clone());
            notify::register(app.handle());
            notify::schedule(app.handle().clone());
            model::install(app.handle());
            ai::schedule(app.handle().clone());
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                apply_glass(&window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Finance");
}
