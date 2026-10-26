//! Development check: print the trend, limit and forecast sections as JSON,
//! keyed like the Python `DashboardView`, for `cmp_report.py`.
//!
//! usage: parity_trends DB [NOW, default 2026-10-26T21:00:00+01:00[Europe/Amsterdam]]

use finance_core::forecast::{ForecastPeriod, spending_forecasts};
use finance_core::limits::spending_nudges;
use finance_core::personal::apply_personal_spending;
use finance_core::plans::{net_spending_transactions, paid_outflow_ids, planned_payment_views};
use finance_core::trends::time_trends;
use finance_core::{Ctx, Snapshot, groups};
use finance_storage::Store;
use jiff::Zoned;

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

    // The dashboard's spending: settlements out, linked reimbursements
    // netted, then owner-marked personal spending.
    let payments = planned_payment_views(&ctx);
    let spending = groups::without_settlements(&snap.transactions, &snap.details);
    let spending = net_spending_transactions(&spending, &payments);
    let spending = apply_personal_spending(spending, &snap.details);
    let fixed_categories =
        groups::fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    let fixed_ids = groups::non_variable_ids(&snap.transactions, &fixed_categories);

    let full = time_trends(
        &spending,
        &snap.group_mappings,
        &ctx.now,
        &Default::default(),
    );
    let current_month = ctx.current_month();
    let mut forecast_excluded = fixed_ids.clone();
    forecast_excluded.extend(paid_outflow_ids(&payments));
    let report = serde_json::json!({
        "spending_limits": snap.spending_limits,
        "spending_nudges": spending_nudges(&snap.spending_limits, &full),
        "variable_time_trends": time_trends(&spending, &snap.group_mappings, &ctx.now, &fixed_ids),
        "spending_forecasts": spending_forecasts(
            &spending,
            &snap.group_mappings,
            &ctx.now,
            &forecast_excluded,
            |t| ctx.resolver.month_of(t),
            ForecastPeriod {
                current_month,
                period_start: ctx.resolver.period_start_date(current_month),
            },
        ),
        "time_trends": full,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
