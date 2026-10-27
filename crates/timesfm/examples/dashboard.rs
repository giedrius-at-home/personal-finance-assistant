//! Development check: build the dashboard with TimesFM on a database copy,
//! print its daily, group and outlook forecasts as JSON (for `cmp_report.py` against a golden
//! from `parity/dump.py`) and the timings to stderr.
//!
//! usage: dashboard DB CHECKPOINT [NOW, default 2026-10-26T21:00:00+01:00[Europe/Amsterdam]]
//! (CHECKPOINT `-` runs without the model.) Peak memory: run under
//! `/usr/bin/time -l`.

use std::cell::RefCell;
use std::time::Instant;

use anyhow::Context;
use finance_core::daily::SeriesForecaster;
use finance_core::{Snapshot, dashboard};
use finance_storage::Store;
use finance_timesfm::TimesFm;

/// Times every model call.
#[derive(Debug)]
struct Timed<'a> {
    model: &'a TimesFm,
    calls: RefCell<Vec<(usize, usize, bool, f64)>>,
}

impl SeriesForecaster for Timed<'_> {
    fn name(&self) -> &str {
        self.model.name()
    }

    fn supports_covariates(&self) -> bool {
        self.model.supports_covariates()
    }

    fn quantiles(
        &self,
        contexts: &[Vec<f64>],
        horizon: usize,
        covariates: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<[f64; 3]>>, String> {
        let started = Instant::now();
        let rows = self.model.quantiles(contexts, horizon, covariates);
        self.calls.borrow_mut().push((
            contexts.len(),
            horizon,
            covariates.is_some(),
            started.elapsed().as_secs_f64(),
        ));
        rows
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let db = args
        .next()
        .context("usage: dashboard DB CHECKPOINT [NOW]")?;
    let checkpoint = args
        .next()
        .context("usage: dashboard DB CHECKPOINT [NOW]")?;
    let now: jiff::Zoned = args
        .next()
        .unwrap_or_else(|| "2026-10-26T21:00:00+01:00[Europe/Amsterdam]".into())
        .parse()?;
    let store = Store::open_read_only(db.as_ref())?;
    let started = Instant::now();
    let snap = Snapshot::load(&store)?;
    eprintln!("snapshot: {:.2}s", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let baseline = dashboard::dashboard(&store, &snap, &now, None)?;
    eprintln!(
        "dashboard without model: {:.2}s",
        started.elapsed().as_secs_f64()
    );
    if checkpoint == "-" {
        let daily = serde_json::json!({
            "daily_forecasts": baseline.daily_forecasts,
            "spending_forecasts": baseline.spending_forecasts,
            "savings_outlooks": baseline.savings_outlooks,
        });
        println!("{}", serde_json::to_string_pretty(&daily)?);
        return Ok(());
    }

    let total = Instant::now();
    let model = TimesFm::load(checkpoint.as_ref())?;
    eprintln!("model load: {:.2}s", total.elapsed().as_secs_f64());
    let timed = Timed {
        model: &model,
        calls: RefCell::new(Vec::new()),
    };
    let started = Instant::now();
    let report = dashboard::dashboard(&store, &snap, &now, Some(&timed))?;
    eprintln!(
        "dashboard with model: {:.2}s (load + dashboard {:.2}s)",
        started.elapsed().as_secs_f64(),
        total.elapsed().as_secs_f64()
    );
    // Again with the weights loaded and warm.
    let warm = Instant::now();
    dashboard::dashboard(&store, &snap, &now, Some(&model))?;
    eprintln!(
        "dashboard with model, again: {:.2}s",
        warm.elapsed().as_secs_f64()
    );
    for (contexts, horizon, covariates, seconds) in timed.calls.borrow().iter() {
        eprintln!(
            "  quantiles: {contexts} contexts, horizon {horizon}, covariates {covariates}: {seconds:.2}s"
        );
    }
    let daily = serde_json::json!({
        "daily_forecasts": report.daily_forecasts,
        "spending_forecasts": report.spending_forecasts,
        "savings_outlooks": report.savings_outlooks,
    });
    println!("{}", serde_json::to_string_pretty(&daily)?);
    Ok(())
}
