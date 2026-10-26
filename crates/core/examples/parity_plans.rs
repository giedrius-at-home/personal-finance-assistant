//! Development check: print the bill and planned-payment sections as JSON,
//! keyed like the Python dashboard, for `cmp_report.py`. On stderr, compare
//! the derived bill schedule with the last Firefly bill snapshot
//! (`subscription_snapshot`), which Firefly computed itself.
//!
//! usage: parity_plans DB [NOW, default 2026-10-26T21:00:00+01:00[Europe/Amsterdam]]

use finance_core::Ctx;
use finance_core::Snapshot;
use finance_core::plans::planned_payment_views;
use finance_core::subscriptions::{
    BillWindow, budget_calendar_offset, linked_payment_dates, schedule_dates, subscription_report,
};
use finance_storage::Store;
use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let store = Store::open_read_only(&path)?;
    let snap = Snapshot::load(&store)?;
    let now: Zoned = args
        .next()
        .as_deref()
        .unwrap_or("2026-10-26T21:00:00+01:00[Europe/Amsterdam]")
        .parse()?;
    let ctx = Ctx::new(&snap, &now);
    let offset = budget_calendar_offset(&ctx);
    let report = serde_json::json!({
        "subscriptions": subscription_report(&ctx, BillWindow::CalendarMonth(offset))?,
        "next_month_subscriptions": subscription_report(&ctx, BillWindow::CalendarMonth(offset + 1))?,
        "planned_payments": planned_payment_views(&ctx),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    oracle(&store, &snap)
}

/// Firefly listed a bill's pay dates in the snapshot window minus those
/// already paid in their cycle, and paid dates as the linked bookings.
fn oracle(store: &Store, snap: &Snapshot) -> anyhow::Result<()> {
    let payload: String = store.connection().query_row(
        "SELECT payload_json FROM subscription_snapshot WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    let stored: serde_json::Value = serde_json::from_str(&payload)?;
    let start: Date = stored["start"].as_str().unwrap_or_default().parse()?;
    let end: Date = stored["end"].as_str().unwrap_or_default().parse()?;
    let local = |v: &serde_json::Value| -> Option<Date> {
        let at: Timestamp = v.as_str()?.parse().ok()?;
        Some(at.to_zoned(snap.zone.clone()).date())
    };
    let mut mismatches = 0;
    for b in stored["bills"].as_array().into_iter().flatten() {
        let id = b["id"].as_str().unwrap_or_default();
        let Some(bill) = snap.bills.iter().find(|x| x.id == id) else {
            eprintln!("bill {id}: missing from the ledger");
            mismatches += 1;
            continue;
        };
        let a = &b["attributes"];
        let firefly_pay: Vec<Date> = a["pay_dates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(local)
            .collect();
        let firefly_paid: Vec<Date> = a["paid_dates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| local(&p["date"]))
            .collect();
        let mut paid = linked_payment_dates(&snap.raw_transactions, id, start, end);
        paid.sort();
        let horizon = schedule_dates(bill, end.saturating_add(jiff::Span::new().years(1)));
        let pay: Vec<Date> = horizon
            .iter()
            .enumerate()
            .filter(|(_, d)| start <= **d && **d <= end)
            .filter(|(i, d)| {
                let next = horizon.get(i + 1).copied().unwrap_or(Date::MAX);
                !paid.iter().any(|p| **d <= *p && *p < next)
            })
            .map(|(_, d)| *d)
            .collect();
        let mut firefly_paid_sorted = firefly_paid.clone();
        firefly_paid_sorted.sort();
        if pay != firefly_pay || paid != firefly_paid_sorted {
            mismatches += 1;
            eprintln!(
                "bill {id} {}: pay {pay:?} vs firefly {firefly_pay:?}; paid {paid:?} vs firefly {firefly_paid_sorted:?}",
                bill.name
            );
        }
    }
    eprintln!("bill schedule oracle: {mismatches} mismatching bills");
    Ok(())
}
