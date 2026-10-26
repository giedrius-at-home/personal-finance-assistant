/// Commands the UI may call; each needs an `allow-*` entry in a capability.
const COMMANDS: &[&str] = &[
    "dashboard",
    "review",
    "search_transactions",
    "refresh",
    "save_budget_plan",
    "status",
    "update_status",
    "check_for_updates",
    "install_update",
    "set_release_token",
    "forget_release_token",
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
