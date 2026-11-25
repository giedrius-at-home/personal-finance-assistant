//! AI suggestion commands and the scheduler. Model and web calls are
//! blocking HTTP, so they run on their own thread, never on the async runtime.

use std::time::Duration;

use finance_ai::Assistant;
use finance_core::service::Service;
use finance_types::ai::{
    AiSettings, AiStatus, GroupSuggestion, MerchantResearchResult, SuggestionPass,
};
use tauri::{AppHandle, Emitter, Manager};

/// Event the UI listens to so the inbox reloads after a pass changed data.
pub const SUGGESTED: &str = "ai-suggestions";
/// Python's suggestion and daily-review loops slept 60 s between passes.
const EVERY: Duration = Duration::from_secs(60);
/// Let the window load before the first pass.
const FIRST: Duration = Duration::from_secs(60);

/// Run `f` on a plain thread and wait for it without blocking the runtime.
async fn off_runtime<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Service, &Assistant) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let worker = std::thread::spawn(move || f(&app.state::<Service>(), &app.state::<Assistant>()));
    tauri::async_runtime::spawn_blocking(move || worker.join())
        .await
        .map_err(|_| "AI suggestions stopped".to_owned())?
        .map_err(|_| "AI suggestions stopped".to_owned())?
}

#[tauri::command]
pub async fn ai_status(app: AppHandle) -> Result<AiStatus, String> {
    crate::traced("ai_status", off_runtime(app, |s, a| a.status(s)).await)
}

#[tauri::command]
pub async fn save_ai_settings(app: AppHandle, settings: AiSettings) -> Result<AiStatus, String> {
    crate::traced(
        "save_ai_settings",
        off_runtime(app, move |s, a| a.save_settings(s, settings)).await,
    )
}

/// Store the TypeSafe API key. It goes straight to the secret store and is
/// never returned to the UI.
#[tauri::command]
pub async fn set_typesafe_key(app: AppHandle, key: String) -> Result<(), String> {
    off_runtime(app, move |_, a| a.set_typesafe_key(&key)).await
}

#[tauri::command]
pub async fn forget_typesafe_key(app: AppHandle) -> Result<(), String> {
    off_runtime(app, |_, a| a.forget_typesafe_key()).await
}

/// Store the Mistral API key. It goes straight to the secret store and is
/// never returned to the UI.
#[tauri::command]
pub async fn set_mistral_key(app: AppHandle, key: String) -> Result<(), String> {
    off_runtime(app, move |_, a| a.set_mistral_key(&key)).await
}

#[tauri::command]
pub async fn forget_mistral_key(app: AppHandle) -> Result<(), String> {
    off_runtime(app, |_, a| a.forget_mistral_key()).await
}

/// The owner's "Suggest categories" (`POST /review/suggest`).
#[tauri::command]
pub async fn suggest_categories(app: AppHandle) -> Result<SuggestionPass, String> {
    let handle = app.clone();
    let result = off_runtime(app, |s, a| a.suggest_now(s)).await;
    if result.is_ok() {
        let _ = handle.emit(SUGGESTED, ());
    }
    crate::traced("suggest_categories", result)
}

#[tauri::command]
pub async fn suggest_spending_group(
    app: AppHandle,
    category: String,
) -> Result<GroupSuggestion, String> {
    crate::traced(
        "suggest_spending_group",
        off_runtime(app, move |s, a| a.suggest_group(s, &category)).await,
    )
}

/// Public research on one name the owner typed (`POST /review/{id}/research`).
#[tauri::command(rename_all = "snake_case")]
pub async fn research_merchant(
    app: AppHandle,
    proposal: String,
    public_name: String,
) -> Result<MerchantResearchResult, String> {
    crate::traced(
        "research_merchant",
        off_runtime(app, move |s, a| {
            a.research_merchant(s, &proposal, &public_name)
        })
        .await,
    )
}

/// A pass every minute, or sooner when a bank sync wrote transactions.
pub fn schedule(app: AppHandle) {
    std::thread::spawn(move || {
        app.state::<Assistant>().wait(FIRST);
        loop {
            let assistant = app.state::<Assistant>();
            match assistant.tick(&app.state::<Service>()) {
                Ok(Some(run)) => {
                    if crate::tracing() {
                        eprintln!(
                            "finance: ai pass suggested={} applied={} {}ms {}",
                            run.suggested,
                            run.applied,
                            run.millis,
                            if run.error.is_some() { "failed" } else { "ok" }
                        );
                    }
                    if run.suggested > 0 || run.applied > 0 {
                        let _ = app.emit(SUGGESTED, ());
                    }
                }
                Ok(None) => {}
                Err(_) if crate::tracing() => eprintln!("finance: ai pass failed"),
                Err(_) => {}
            }
            assistant.wait(EVERY);
        }
    });
}
