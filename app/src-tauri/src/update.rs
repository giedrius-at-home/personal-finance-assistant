//! Signed in-app updates from this repository's GitHub releases.
//!
//! The release workflow publishes `latest.json` next to the signed app
//! archive. The repository is private, so both are fetched through the GitHub
//! API with a read-only token kept in the Keychain. The updater plugin checks
//! the archive's signature against the public key compiled into the app and
//! refuses anything that does not verify or is not newer.

use std::sync::Mutex;
use std::time::Duration;

use finance_secrets::{Item, Secret};
use finance_types::{AvailableUpdate, UpdateStatus};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

const REPO: &str = "giedrius-at-home/personal-finance-assistant";
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Event the UI listens to so it can refresh the update state.
pub const CHANGED: &str = "update-status";

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Update>>,
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

impl Updates {
    pub fn status(&self, app: &AppHandle) -> UpdateStatus {
        let mut status = self.status.lock().map(|s| s.clone()).unwrap_or_default();
        status.current_version = app.package_info().version.to_string();
        status.token_set = matches!(finance_secrets::get(Item::ReleaseToken), Ok(Some(_)));
        status
    }

    fn update(&self, app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
        let _ = app.emit(CHANGED, ());
    }
}

fn token() -> Result<Secret, String> {
    finance_secrets::get(Item::ReleaseToken)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No release token yet".to_owned())
}

/// URL of `latest.json` on the newest release, as a GitHub API asset URL.
async fn manifest_url(token: &Secret) -> Result<Url, String> {
    let client = reqwest::Client::builder()
        .user_agent("Finance.app")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .bearer_auth(token.expose())
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "GitHub could not be reached".to_owned())?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err("The release token was refused".to_owned()),
        404 => return Err("No release published yet".to_owned()),
        code => return Err(format!("GitHub answered {code}")),
    }
    let release: Release = response
        .json()
        .await
        .map_err(|_| "Unexpected release format".to_owned())?;
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == "latest.json")
        .ok_or_else(|| "The latest release has no latest.json".to_owned())?;
    Url::parse(&asset.url).map_err(|e| e.to_string())
}

async fn find_update(app: &AppHandle) -> Result<Option<Update>, String> {
    let token = token()?;
    let url = manifest_url(&token).await?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .header("Authorization", format!("Bearer {}", token.expose()))
        .map_err(|e| e.to_string())?
        .header("Accept", "application/octet-stream")
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| e.to_string())
}

pub async fn check(app: AppHandle) {
    let updates = app.state::<Updates>();
    if updates
        .status
        .lock()
        .is_ok_and(|s| s.checking || s.installing)
    {
        return;
    }
    updates.update(&app, |s| s.checking = true);
    let result = find_update(&app).await;
    let now = now_local();
    updates.update(&app, |s| {
        s.checking = false;
        s.last_checked = Some(now);
        match &result {
            Ok(found) => {
                s.error = None;
                s.available = found.as_ref().map(|u| AvailableUpdate {
                    version: u.version.clone(),
                    notes: u.body.clone(),
                    date: u.date.map(|d| d.date().to_string()),
                });
            }
            Err(e) => s.error = Some(e.clone()),
        }
    });
    if let (Ok(found), Ok(mut pending)) = (result, updates.pending.lock()) {
        *pending = found;
    }
}

pub async fn install(app: AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let update = updates
        .pending
        .lock()
        .map_err(|_| "update state poisoned".to_owned())?
        .take()
        .ok_or_else(|| "No update to install".to_owned())?;
    updates.update(&app, |s| s.installing = true);
    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        let message = e.to_string();
        updates.update(&app, |s| {
            s.installing = false;
            s.error = Some(message.clone());
        });
        return Err(message);
    }
    app.restart();
}

/// Check at launch and then every few hours.
pub fn schedule(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if matches!(finance_secrets::get(Item::ReleaseToken), Ok(Some(_))) {
                check(app.clone()).await;
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

fn now_local() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string()
}
