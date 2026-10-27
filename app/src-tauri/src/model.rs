//! The daily forecast's TimesFM model: find its weights on this Mac, hand
//! the service a loader for them, and download them from Hugging Face when
//! the owner asks.
//!
//! The service loads the weights only for a background run (on its own
//! thread, never the async runtime), forecasts everything the screens need
//! for the current data and drops them again, so the 1.3 GB stay out of
//! memory between runs. Without the weights daily forecasts use the weekday
//! baseline.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use finance_core::daily::SeriesForecaster;
use finance_core::forecast_model::ForecastModel;
use finance_core::service::Service;
use finance_timesfm::{CheckpointFile, FILES, NAME, TimesFm};
use finance_types::ForecastModelStatus;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};

/// Event: the model's status changed (download progress, a run ended).
pub const CHANGED: &str = "forecast-model";
/// Event: new forecasts are ready, so screens reload.
pub const READY: &str = "daily-forecast";
/// Report download progress at most this often.
const PROGRESS_EVERY: u64 = 16 << 20;

#[derive(Debug, Default)]
pub struct Model {
    download: Mutex<Download>,
}

#[derive(Debug, Default)]
struct Download {
    /// Bytes received, while a download runs.
    received: Option<u64>,
    error: Option<String>,
}

impl Model {
    fn update(&self, app: &AppHandle, f: impl FnOnce(&mut Download)) {
        if let Ok(mut download) = self.download.lock() {
            f(&mut download);
        }
        let _ = app.emit(CHANGED, ());
    }
}

/// `~/Library/Application Support/Finance/models/timesfm-3.0`.
fn app_dir() -> Option<PathBuf> {
    finance_storage::data_dir()
        .ok()
        .map(|d| d.join("models").join("timesfm-3.0"))
}

/// The weights and where they were found: the app's folder first, else the
/// Hugging Face cache.
fn locate() -> Option<(PathBuf, &'static str)> {
    if let Some(dir) = app_dir().filter(|d| finance_timesfm::is_complete(d)) {
        return Some((dir, "app"));
    }
    finance_timesfm::hub_cache()
        .and_then(|hub| finance_timesfm::hub_snapshot(&hub))
        .map(|dir| (dir, "huggingface"))
}

/// Give the service the model when its weights are on this Mac.
pub fn install(app: &AppHandle) {
    let model = locate().map(|(dir, _)| {
        let handle = app.clone();
        ForecastModel {
            name: NAME.into(),
            supports_covariates: true,
            load: Box::new(move || {
                TimesFm::load(&dir)
                    .map(|m| Box::new(m) as Box<dyn SeriesForecaster + Send>)
                    .map_err(|e| e.to_string())
            }),
            on_done: Box::new(move |ready| {
                if crate::tracing() {
                    let status = handle.state::<Service>().forecast_model_status();
                    eprintln!(
                        "finance: forecast model run {} in {:.1}s",
                        if ready { "ok" } else { "failed" },
                        status.and_then(|s| s.last_seconds).unwrap_or_default()
                    );
                }
                if ready {
                    let _ = handle.emit(READY, ());
                }
                let _ = handle.emit(CHANGED, ());
            }),
        }
    });
    app.state::<Service>().set_forecast_model(model);
}

#[tauri::command(async)]
pub fn forecast_model_status(
    service: State<'_, Service>,
    model: State<'_, Model>,
) -> ForecastModelStatus {
    let located = locate();
    let run = service.forecast_model_status().unwrap_or_default();
    let (downloading, download_error) = model
        .download
        .lock()
        .map(|d| (d.received, d.error.clone()))
        .unwrap_or_default();
    ForecastModelStatus {
        available: located.is_some(),
        source: located.map(|(_, source)| source.to_owned()),
        download_bytes: FILES.iter().map(|f| f.bytes).sum(),
        downloading,
        running: run.running,
        last_run_seconds: run.last_seconds,
        error: download_error.or(run.error),
    }
}

/// Start downloading the weights into the app's folder (on the owner's
/// click). Progress and the outcome arrive as [`CHANGED`] events.
#[tauri::command(async)]
pub fn download_forecast_model(app: AppHandle, model: State<'_, Model>) -> Result<(), String> {
    {
        let mut download = model
            .download
            .lock()
            .map_err(|_| "download state poisoned".to_owned())?;
        if download.received.is_some() {
            return Ok(());
        }
        download.received = Some(0);
        download.error = None;
    }
    let _ = app.emit(CHANGED, ());
    std::thread::spawn(move || {
        let result = app_dir()
            .ok_or_else(|| "No application data folder".to_owned())
            .and_then(|dir| download(&app, &dir));
        let model = app.state::<Model>();
        if result.is_ok() {
            install(&app);
        }
        model.update(&app, |d| {
            d.received = None;
            d.error = result.as_ref().err().cloned();
        });
        if crate::tracing() {
            let outcome = if result.is_ok() { "ok" } else { "failed" };
            eprintln!("finance: download_forecast_model {outcome}");
        }
        if result.is_ok() {
            let _ = app.emit(READY, ());
        }
    });
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file already there, complete and intact.
fn verified(path: &Path, file: &CheckpointFile) -> bool {
    let Ok(mut input) = File::open(path) else {
        return false;
    };
    if input.metadata().map(|m| m.len()).ok() != Some(file.bytes) {
        return false;
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut input, &mut hash).is_ok() && hex(&hash.finalize()) == file.sha256
}

/// Fetch each checkpoint file at the pinned revision into `dir`, checking
/// its size and SHA-256 before it replaces anything.
fn download(app: &AppHandle, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("Finance.app")
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(2 * 60 * 60))
        .build()
        .map_err(|e| e.to_string())?;
    let model = app.state::<Model>();
    let started = Instant::now();
    let mut received = 0u64;
    let mut reported = 0u64;
    for file in &FILES {
        let target = dir.join(file.name);
        if verified(&target, file) {
            received += file.bytes;
            continue;
        }
        let partial = dir.join(format!("{}.partial", file.name));
        let mut response = client
            .get(file.url())
            .send()
            .map_err(|_| "Hugging Face could not be reached".to_owned())?;
        if !response.status().is_success() {
            return Err(format!(
                "Hugging Face answered {}",
                response.status().as_u16()
            ));
        }
        let mut out = File::create(&partial).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        let mut size = 0u64;
        loop {
            let n = response
                .read(&mut buffer)
                .map_err(|_| "The download was interrupted".to_owned())?;
            if n == 0 {
                break;
            }
            out.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            hash.update(&buffer[..n]);
            size += n as u64;
            received += n as u64;
            if received - reported >= PROGRESS_EVERY {
                reported = received;
                model.update(app, |d| d.received = Some(received));
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        if size != file.bytes || hex(&hash.finalize()) != file.sha256 {
            let _ = std::fs::remove_file(&partial);
            return Err("The download did not match the published model".to_owned());
        }
        std::fs::rename(&partial, &target).map_err(|e| e.to_string())?;
    }
    if crate::tracing() {
        eprintln!(
            "finance: forecast model downloaded in {:.0}s",
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
