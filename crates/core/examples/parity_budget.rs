//! Development check: print the monthly budgets and allocation as JSON, keyed
//! like the Python `DashboardView`, for `cmp_report.py`.

use finance_core::allocation::{AllocationInputs, monthly_allocation, savings_roles};
use finance_core::budget::{monthly_budgets, spending_transactions};
use finance_core::plans::planned_payment_views;
use finance_core::reporting::{ReportInputs, assignments, weekly_report};
use finance_core::subscriptions::{BillWindow, budget_calendar_offset, subscription_report};
use finance_core::{Ctx, Snapshot, cash};
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

    let payments = planned_payment_views(&ctx);
    let spending = spending_transactions(&ctx, &payments);
    let offset = budget_calendar_offset(&ctx);
    let current = subscription_report(&ctx, BillWindow::CalendarMonth(offset))?;
    let next = subscription_report(&ctx, BillWindow::CalendarMonth(offset + 1))?;
    let budgets = monthly_budgets(&ctx, &spending, &payments, &current, &next);

    let overview = cash::cash_overview(
        &snap.accounts,
        &snap.transactions,
        &ctx.now,
        snap.settings.payday_day,
        inputs.refreshed_at,
    );
    let goal_evaluations = if snap.settings.savings_goal_contributions_enabled {
        let assigned = assignments(&snap.transactions, &inputs.approved_categories);
        weekly_report(
            &snap.transactions,
            &assigned,
            &snap.goals,
            &inputs.safe_to_spend,
            &inputs.alert_severities,
            &ctx.now,
        )
        .goals
    } else {
        Vec::new()
    };
    let (savings_accounts, savings_goals) = savings_roles(&ctx);
    let current_month = ctx.current_month();
    // Spending nudges only matter without budgets; the dashboard always has them.
    let allocation = monthly_allocation(&AllocationInputs {
        through_date: overview.through_date.parse()?,
        accounts: &snap.accounts,
        goals: &snap.goals,
        goal_evaluations: &goal_evaluations,
        payday_status: overview.payday_status.as_ref(),
        current_subscriptions: &current,
        next_subscriptions: &next,
        planned_payments: &payments,
        spending_limits: &snap.spending_limits,
        spending_nudges: &[],
        budgets: Some(&budgets),
        savings_account_ids: &savings_accounts,
        current_month: Some(current_month),
        savings_goal_ids: &savings_goals,
        period_start: Some(ctx.resolver.period_start_date(current_month)),
    });

    let report = serde_json::json!({
        "monthly_budgets": budgets,
        "monthly_allocation": allocation,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
