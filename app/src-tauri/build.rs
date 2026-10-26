/// Commands the UI may call; each needs an `allow-*` entry in a capability.
const COMMANDS: &[&str] = &[
    "overview",
    "accounts",
    "facets",
    "transactions",
    "category_totals",
    "status",
    "update_status",
    "check_for_updates",
    "install_update",
    "set_release_token",
    "forget_release_token",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
