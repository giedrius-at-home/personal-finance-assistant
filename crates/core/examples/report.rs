//! Development check: build the whole dashboard and review report, print
//! them as one JSON object for `cmp_report.py`, and time each step.

use std::time::Instant;

use finance_core::{Snapshot, dashboard};
use finance_storage::Store;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let now = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "2026-10-26T21:00:00+01:00[Europe/Amsterdam]".into())
        .parse()?;
    let store = Store::open_read_only(&path)?;
    let started = Instant::now();
    let snap = Snapshot::load(&store)?;
    let loaded = started.elapsed();
    let report = dashboard::dashboard(&store, &snap, &now, None)?;
    let built = started.elapsed();
    let review = dashboard::review_report(&snap, &now);
    let reviewed = started.elapsed();
    eprintln!(
        "snapshot {loaded:?}, dashboard {:?}, review {:?}",
        built - loaded,
        reviewed - built
    );
    let mut value = serde_json::to_value(&report)?;
    let review = serde_json::to_value(&review)?;
    if let (Some(map), Some(extra)) = (value.as_object_mut(), review.as_object()) {
        map.extend(extra.clone());
    }
    println!("{value}");
    Ok(())
}
