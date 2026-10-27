//! The app's path end to end: the service answers with the baseline, runs
//! TimesFM in the background, then answers with its forecasts.
//!
//! ```sh
//! TIMESFM_DB=<copy of finance.sqlite3> \
//! cargo test --release -p finance-timesfm --test service -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use finance_core::daily::SeriesForecaster;
use finance_core::forecast_model::ForecastModel;
use finance_core::service::Service;
use finance_timesfm::{NAME, TimesFm, hub_cache, hub_snapshot};

#[test]
#[ignore = "needs the TimesFM weights and a database"]
fn forecasts_arrive_after_a_background_run() -> Result<()> {
    let db: PathBuf = std::env::var_os("TIMESFM_DB")
        .context("set TIMESFM_DB")?
        .into();
    let checkpoint = hub_cache()
        .and_then(|hub| hub_snapshot(&hub))
        .context("no checkpoint")?;
    let service = Service::new(db);
    let (done, finished) = mpsc::channel();
    service.set_forecast_model(Some(ForecastModel {
        name: NAME.into(),
        supports_covariates: true,
        load: Box::new(move || {
            TimesFm::load(&checkpoint)
                .map(|m| Box::new(m) as Box<dyn SeriesForecaster + Send>)
                .map_err(|e| e.to_string())
        }),
        on_done: Box::new(move |ready| {
            let _ = done.send(ready);
        }),
    }));

    let started = Instant::now();
    let first = service.dashboard().map_err(anyhow::Error::msg)?;
    println!("first dashboard: {:.2}s", started.elapsed().as_secs_f64());
    assert!(
        first
            .daily_forecasts
            .iter()
            .all(|f| f.method == "weekday_baseline")
    );
    assert!(service.forecast_model_status().is_some_and(|s| s.running));

    assert!(finished.recv_timeout(Duration::from_secs(120))?);
    let status = service.forecast_model_status().context("status")?;
    println!(
        "background run: {:.2}s",
        status.last_seconds.unwrap_or_default()
    );
    assert_eq!(status.error, None);

    let started = Instant::now();
    let second = service.dashboard().map_err(anyhow::Error::msg)?;
    println!("second dashboard: {:.2}s", started.elapsed().as_secs_f64());
    for f in &second.daily_forecasts {
        println!("{}: {} ({})", f.currency.code, f.method, f.model);
        assert!(f.model_error.is_some());
    }
    // No further run: everything was answered.
    assert!(!service.forecast_model_status().is_some_and(|s| s.running));
    Ok(())
}
