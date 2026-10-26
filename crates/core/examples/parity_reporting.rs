//! Development check: print the reporting sections as JSON, keyed like the
//! Python `DashboardView`, for `cmp_report.py`.

use std::collections::HashSet;

use finance_core::reporting::{
    ReportInputs, assignments, merge_aggregates, net_worth_report, taxonomy_aggregates,
    taxonomy_recommendations, weekly_report,
};
use finance_core::{Ctx, Snapshot, cash, catalog, groups};
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
    let snap = Snapshot::load(&store)?;
    let ctx = Ctx::new(&snap, &now);
    let inputs = ReportInputs::load(store.connection(), snap.sync_health.0)?;
    let txs = &snap.transactions;
    let assigned = assignments(txs, &inputs.approved_categories);

    // Python nets linked reimbursements (planned payments) here too; this
    // database has none, so the settlement filter is the whole difference.
    let spending = groups::without_settlements(txs, &snap.details);
    let daily_ids: HashSet<String> = snap
        .daily_review_entries
        .iter()
        .map(|e| e.transaction_id.clone())
        .collect();

    let report = serde_json::json!({
        "weekly": weekly_report(
            txs,
            &assigned,
            &snap.goals,
            &inputs.safe_to_spend,
            &inputs.alert_severities,
            &ctx.now,
        ),
        "net_worth": net_worth_report(&snap.accounts, &ctx.now, inputs.refreshed_at),
        "cash_overview": cash::cash_overview(
            &snap.accounts,
            txs,
            &ctx.now,
            snap.settings.payday_day,
            inputs.refreshed_at,
        ),
        "monthly_spending": groups::monthly_spending(&spending, &snap.group_mappings, &ctx.now),
        "category_catalog": catalog::category_catalog(txs),
        "bulk_payments": catalog::bulk_payment_summaries(txs, &snap.details),
        "historical_uncategorized_count": catalog::historical_uncategorized_count(txs, &daily_ids),
        "taxonomy": taxonomy_recommendations(&merge_aggregates(&taxonomy_aggregates(
            txs, &assigned, &ctx.now,
        ))),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
