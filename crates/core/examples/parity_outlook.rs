//! Development check: print the savings outlook and daily forecast as JSON,
//! keyed like the Python `DashboardView`, for `cmp_report.py`.
//!
//! usage: parity_outlook DB [NOW, default 2026-10-26T21:00:00+01:00[Europe/Amsterdam]]

use finance_core::daily::{DailyInputs, daily_spending_forecasts, forecast_plans};
use finance_core::outlook::{OutlookInputs, data_fresh, savings_outlooks};
use finance_core::personal::apply_personal_spending;
use finance_core::plans::{net_spending_transactions, planned_payment_views};
use finance_core::subscriptions::{BillWindow, subscription_report};
use finance_core::{Ctx, Snapshot, groups};
use finance_storage::Store;
use finance_types::report::Currency;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let now = args
        .next()
        .unwrap_or_else(|| "2026-10-26T21:00:00+01:00[Europe/Amsterdam]".into())
        .parse()?;
    let store = Store::open_read_only(&path)?;
    let snap = Snapshot::load(&store)?;
    let ctx = Ctx::new(&snap, &now);

    // The dashboard's spending inputs: no settlements, linked reimbursements
    // netted, owner-marked shared spending applied.
    let payments = planned_payment_views(&ctx);
    let spending = apply_personal_spending(
        net_spending_transactions(
            &groups::without_settlements(&snap.transactions, &snap.details),
            &payments,
        ),
        &snap.details,
    );
    let fixed_categories =
        groups::fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    let fixed_ids = groups::fixed_transaction_ids(&spending, &fixed_categories);
    // Forecast horizons run from today, so they keep calendar months.
    let subscriptions = (0..4)
        .map(|offset| subscription_report(&ctx, BillWindow::CalendarMonth(offset)))
        .collect::<Result<Vec<_>, _>>()?;
    let current_month = ctx.current_month();
    let mut currencies: Vec<Currency> = snap
        .accounts
        .iter()
        .map(|a| a.currency().clone())
        .chain(spending.iter().map(|t| t.currency.clone()))
        .collect();
    currencies.sort_by(|a, b| a.code.cmp(&b.code));
    currencies.dedup();

    let outlooks = savings_outlooks(&OutlookInputs {
        transactions: &spending,
        accounts: &snap.accounts,
        plans: &snap.budget_plans,
        mappings: &snap.group_mappings,
        payments: &payments,
        subscriptions: &subscriptions,
        fixed_ids: &fixed_ids,
        payday_day: snap.settings.payday_day,
        now: &ctx.now,
        data_fresh: data_fresh(&snap.sync_health, &ctx.now),
        current_month,
        resolver: &ctx.resolver,
    });
    let plans = forecast_plans(&snap.budget_plans, current_month, &currencies);
    let daily = daily_spending_forecasts(&DailyInputs {
        transactions: &spending,
        payments: &payments,
        subscriptions: &subscriptions,
        fixed_ids: &fixed_ids,
        now: &ctx.now,
        current_month,
        month_start: ctx.resolver.period_start_date(current_month),
        budget_plans: &plans,
        mappings: &snap.group_mappings,
        excluded_categories: &snap.settings.forecast_excluded_categories,
        forecaster: None,
    });
    let report = serde_json::json!({
        "savings_outlooks": outlooks,
        "daily_forecasts": daily,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
