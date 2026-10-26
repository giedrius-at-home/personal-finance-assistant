//! Assemble the dashboard report the way the Python `DashboardService.view`
//! did: one snapshot, one moment, every section from the same inputs.

use std::collections::HashSet;

use finance_storage::Store;
use finance_types::report::{Dashboard, ReviewReport};
use jiff::Zoned;

use crate::reporting::{self, ReportInputs};
use crate::subscriptions::{BillWindow, budget_calendar_offset, subscription_report};
use crate::{Ctx, Result, Snapshot, cash, catalog, groups, plans, review};

/// The transactions spending reports count: bulk settlements left out and
/// reimbursed amounts netted against what they repay.
pub fn spending_transactions(
    ctx: &Ctx<'_>,
    payments: &[finance_types::report::plans::PlannedPaymentView],
) -> Vec<crate::model::Tx> {
    let without = groups::without_settlements(&ctx.snap.transactions, &ctx.snap.details);
    plans::net_spending_transactions(&without, payments)
}

pub fn dashboard(store: &Store, snap: &Snapshot, now: &Zoned) -> Result<Dashboard> {
    let ctx = Ctx::new(snap, now);
    let inputs = ReportInputs::load(store.connection(), snap.sync_health.0)?;
    let txs = &snap.transactions;
    let assigned = reporting::assignments(txs, &inputs.approved_categories);
    let weekly = reporting::weekly_report(
        txs,
        &assigned,
        &snap.goals,
        &inputs.safe_to_spend,
        &inputs.alert_severities,
        &ctx.now,
    );
    let cash = cash::cash_overview(
        &snap.accounts,
        txs,
        &ctx.now,
        snap.settings.payday_day,
        inputs.refreshed_at,
    );
    let payments = plans::planned_payment_views(&ctx);
    let spending = spending_transactions(&ctx, &payments);

    let offset = budget_calendar_offset(&ctx);
    let current_subscriptions = subscription_report(&ctx, BillWindow::CalendarMonth(offset))?;
    let next_subscriptions = subscription_report(&ctx, BillWindow::CalendarMonth(offset + 1))?;
    let period_start = ctx
        .resolver
        .period_start_date(ctx.current_month())
        .to_string();
    let salary_received = cash
        .payday_status
        .as_ref()
        .and_then(|p| p.received_on.as_deref())
        .is_some_and(|received| received >= period_start.as_str());

    let daily_ids: HashSet<String> = snap
        .daily_review_entries
        .iter()
        .map(|e| e.transaction_id.clone())
        .collect();
    Ok(Dashboard {
        generated_at: ctx.now.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        settings: snap.settings.clone(),
        net_worth: reporting::net_worth_report(&snap.accounts, &ctx.now, inputs.refreshed_at),
        monthly_spending: groups::monthly_spending(&spending, &snap.group_mappings, &ctx.now),
        category_catalog: catalog::category_catalog(txs),
        taxonomy: reporting::taxonomy_recommendations(&reporting::merge_aggregates(
            &reporting::taxonomy_aggregates(txs, &assigned, &ctx.now),
        )),
        bulk_payments: catalog::bulk_payment_summaries(txs, &snap.details),
        historical_uncategorized_count: catalog::historical_uncategorized_count(txs, &daily_ids),
        custom_spending_groups: snap.custom_groups.clone(),
        last_firefly_refresh: snap.sync_health.0.map(|t| t.to_string()),
        firefly_refresh_failed: snap.sync_health.2,
        subscriptions: current_subscriptions,
        next_month_subscriptions: salary_received.then_some(next_subscriptions),
        planned_payments: payments,
        weekly,
        cash_overview: cash,
    })
}

/// The review inbox. Automatic categorisation runs in a later phase, so the
/// inbox reports it as inactive.
pub fn review_report(snap: &Snapshot, now: &Zoned) -> ReviewReport {
    let ctx = Ctx::new(snap, now);
    ReviewReport {
        review_items: review::review_items(&ctx),
        daily_review: review::daily_inbox(&ctx, false),
    }
}
