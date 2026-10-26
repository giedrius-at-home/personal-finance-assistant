/// Commands the UI may call; each needs an `allow-*` entry in a capability.
const COMMANDS: &[&str] = &[
    "dashboard",
    "review",
    "search_transactions",
    "refresh",
    "save_budget_plan",
    "save_spending_group",
    "create_spending_group",
    "preview_category_consolidation",
    "apply_category_consolidation",
    "status",
    "update_status",
    "check_for_updates",
    "install_update",
    "set_release_token",
    "forget_release_token",
    "create_bill",
    "edit_bill",
    "archive_bill",
    "save_subscription_categories",
    "link_bill_payment",
    "decline_bill_match",
    "create_planned_payment",
    "archive_planned_payment",
    "closure",
    "closure_action",
    "closure_pots",
    "save_pot",
    "save_account_role",
    "unclassified_accounts",
    "manage_workspace",
    "banks",
    "preview_csv_import",
    "commit_csv_import",
    "preview_settings",
    "apply_settings",
    "rollback_settings",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
