//! Development check: the spending email against the Python golden of the
//! same database and moment (`py_email.py`). Prints, per part, whether the
//! Rust output is identical: first rendered from Python's own risk (the
//! template alone), then from the risk Rust computes (the whole pipeline).
//!
//! usage: parity_email <db copy> <rfc3339 moment> <python golden.json>

use finance_core::Snapshot;
use finance_core::spending_email::{self, DIGEST_INTRO, Risk, html};
use finance_storage::Store;
use jiff::Timestamp;
use serde_json::Value;

fn report(name: &str, rust: &str, python: Option<&str>) {
    match python {
        None => println!("{name}: no Python value"),
        Some(p) if p == rust => println!("{name}: IDENTICAL ({} bytes)", rust.len()),
        Some(p) => {
            let at = rust
                .bytes()
                .zip(p.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(rust.len().min(p.len()));
            let from = at.saturating_sub(80);
            println!("{name}: DIFFERS at byte {at}");
            println!(
                "  rust:   {}",
                rust.get(from..(at + 120).min(rust.len())).unwrap_or("")
            );
            println!(
                "  python: {}",
                p.get(from..(at + 120).min(p.len())).unwrap_or("")
            );
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("db path");
    let now: Timestamp = args.next().expect("moment").parse()?;
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(args.next().expect("golden"))?)?;
    let text = |k: &str| golden.get(k).and_then(Value::as_str);

    // The template alone, from Python's risk.
    let digest: Risk = serde_json::from_value(golden["digest_risk"].clone())?;
    report(
        "template digest_html",
        &html::render(&digest, DIGEST_INTRO, false, true),
        text("digest_html"),
    );
    if !golden["risk"].is_null() {
        let risk: Risk = serde_json::from_value(golden["risk"].clone())?;
        report(
            "template alert_html",
            &html::render(&risk, "Hello <you> & 'friends'", false, false),
            text("alert_html"),
        );
        report(
            "template sample_html",
            &html::render(&risk, "Hello", true, false),
            text("sample_html"),
        );
        report(
            "template factual_summary",
            &spending_email::factual_summary(&risk),
            text("factual_summary"),
        );
    }

    // The whole pipeline: Rust's own dashboard and risk.
    let store = Store::open_read_only(path.as_ref())?;
    let snap = Snapshot::load(&store)?;
    let zoned = now.to_zoned(snap.zone.clone());
    let dash = finance_core::dashboard::dashboard(&store, &snap, &zoned, None)?;
    let risk = spending_email::spending_risk(&dash, &snap, &zoned);
    let risk_json = serde_json::to_value(&risk)?;
    if risk_json == golden["risk"] {
        println!("pipeline risk: IDENTICAL");
    } else {
        println!("pipeline risk: DIFFERS");
        if let (Some(r), Some(p)) = (risk_json.as_object(), golden["risk"].as_object()) {
            for (k, v) in r {
                if p.get(k) != Some(v) {
                    println!(
                        "  {k}:\n    rust   {v}\n    python {}",
                        p.get(k).unwrap_or(&Value::Null)
                    );
                }
            }
        }
    }
    report(
        "pipeline forecast_summary",
        &spending_email::forecast_summary(&dash.daily_forecasts, true),
        text("forecast_summary"),
    );
    let digest = Risk {
        forecasts: dash.daily_forecasts.clone(),
        ..Risk::default()
    };
    report(
        "pipeline digest_html",
        &html::render(&digest, DIGEST_INTRO, false, true),
        text("digest_html"),
    );
    if let Some(risk) = &risk {
        report(
            "pipeline alert_html",
            &html::render(risk, "Hello <you> & 'friends'", false, false),
            text("alert_html"),
        );
        report(
            "pipeline factual_summary",
            &spending_email::factual_summary(risk),
            text("factual_summary"),
        );
    }
    Ok(())
}
