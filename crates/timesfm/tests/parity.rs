//! Parity with the PyTorch reference on dumps from `parity/dump.py`.
//!
//! Needs the 1.3 GB weights, so it is ignored by default:
//!
//! ```sh
//! TIMESFM_CHECKPOINT=~/.cache/huggingface/hub/models--google--timesfm-3.0-pytorch/snapshots/<rev> \
//! TIMESFM_PARITY=<dump dir> \
//! cargo test --release -p finance-timesfm --test parity -- --ignored --nocapture
//! ```
//!
//! Errors are relative to `max(|python|, 1)`: below one euro a day they are
//! absolute, so zero-heavy quantiles do not divide by zero.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use finance_core::daily::SeriesForecaster;
use finance_timesfm::{TimesFm, hub_snapshot, service_rows};
use serde::Deserialize;

const TOLERANCE: f64 = 1e-3;

#[derive(Deserialize)]
struct Case {
    #[serde(default)]
    name: String,
    contexts: Vec<Vec<f64>>,
    horizon: usize,
    covariates: Option<Vec<Option<Vec<f64>>>>,
    quantiles: Vec<Vec<Vec<f64>>>,
    /// Golden calls only: the service's (q0.1, mean, q0.9) rows.
    rows: Option<Vec<Vec<Vec<f64>>>>,
}

#[derive(Default)]
struct Errors {
    max: f64,
    sum: f64,
    count: usize,
}

impl Errors {
    fn add(&mut self, rust: f64, python: f64) {
        let error = (rust - python).abs() / python.abs().max(1.0);
        self.max = self.max.max(error);
        self.sum += error;
        self.count += 1;
    }

    fn mean(&self) -> f64 {
        self.sum / self.count.max(1) as f64
    }
}

fn checkpoint() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("TIMESFM_CHECKPOINT") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").context("HOME")?;
    hub_snapshot(&PathBuf::from(home).join(".cache/huggingface/hub"))
        .context("no TimesFM checkpoint; set TIMESFM_CHECKPOINT")
}

fn dumps() -> Result<PathBuf> {
    std::env::var_os("TIMESFM_PARITY")
        .map(Into::into)
        .context("set TIMESFM_PARITY to parity/dump.py's output directory")
}

fn load_cases(file: &str, key: &str) -> Result<Vec<Case>> {
    let path = dumps()?.join(file);
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    Ok(serde_json::from_value(value[key].take())?)
}

fn narrow(values: &[f64]) -> Vec<f32> {
    values.iter().map(|v| *v as f32).collect()
}

#[test]
#[ignore = "needs the TimesFM weights and parity dumps"]
fn matches_pytorch_on_synthetic_and_golden_series() -> Result<()> {
    let started = Instant::now();
    let model = TimesFm::load(&checkpoint()?)?;
    println!("loaded in {:.1}s", started.elapsed().as_secs_f64());

    let mut worst = 0f64;
    for case in load_cases("synthetic.json", "cases")? {
        let contexts: Vec<Vec<f32>> = case.contexts.iter().map(|c| narrow(c)).collect();
        let covariates: Option<Vec<Option<Vec<f32>>>> = case
            .covariates
            .as_ref()
            .map(|c| c.iter().map(|v| v.as_deref().map(narrow)).collect());
        let started = Instant::now();
        let outputs = model.predict(&contexts, case.horizon, covariates.as_deref())?;
        let (mut all, mut used) = (Errors::default(), Errors::default());
        for (rust, python) in outputs.iter().zip(&case.quantiles) {
            assert_eq!(rust.len(), python.len(), "{}: horizon", case.name);
            for (r, p) in rust.iter().zip(python) {
                for (a, b) in r.iter().zip(p) {
                    all.add(f64::from(*a), *b);
                }
            }
            let python: Vec<[f32; 9]> = python
                .iter()
                .map(|p| std::array::from_fn(|q| p[q] as f32))
                .collect();
            for (r, p) in service_rows(rust).iter().zip(service_rows(&python)) {
                for (a, b) in r.iter().zip(p) {
                    used.add(*a, b);
                }
            }
        }
        println!(
            "{:28} all deciles max {:.2e} mean {:.2e} | q10/mean/q90 max {:.2e} mean {:.2e} ({:.2}s)",
            case.name,
            all.max,
            all.mean(),
            used.max,
            used.mean(),
            started.elapsed().as_secs_f64()
        );
        worst = worst.max(used.max);
    }

    let calls = load_cases("golden_calls.json", "calls")?;
    let (mut errors, mut seconds) = (Errors::default(), 0.0);
    for (i, call) in calls.iter().enumerate() {
        let covariates: Option<Vec<Vec<f64>>> = call
            .covariates
            .as_ref()
            .map(|c| c.iter().map(|v| v.clone().unwrap_or_default()).collect());
        let started = Instant::now();
        let rows = model
            .quantiles(&call.contexts, call.horizon, covariates.as_deref())
            .map_err(anyhow::Error::msg)?;
        seconds += started.elapsed().as_secs_f64();
        let mut call_errors = Errors::default();
        for (rust, python) in rows.iter().zip(call.rows.as_ref().context("rows")?) {
            for (r, p) in rust.iter().zip(python) {
                for (a, b) in r.iter().zip(p) {
                    errors.add(*a, *b);
                    call_errors.add(*a, *b);
                }
            }
        }
        println!(
            "golden call {i}: {} contexts, horizon {}, covariates {}: max {:.2e} mean {:.2e}",
            call.contexts.len(),
            call.horizon,
            call.covariates.is_some(),
            call_errors.max,
            call_errors.mean()
        );
    }
    println!(
        "golden: {} calls in {seconds:.2}s, q10/mean/q90 max {:.2e} mean {:.2e}",
        calls.len(),
        errors.max,
        errors.mean()
    );
    worst = worst.max(errors.max);
    assert!(worst < TOLERANCE, "worst relative error {worst:.2e}");
    Ok(())
}
