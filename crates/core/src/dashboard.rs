//! Assemble the dashboard report the way the Python `DashboardService.view`
//! did: one snapshot, one moment, every section from the same inputs.

use std::collections::HashSet;

use finance_storage::Store;
use finance_types::report::{Dashboard, ReviewReport};
use jiff::Zoned;

use crate::reporting::{self, ReportInputs};
use crate::subscriptions::{BillWindow, budget_calendar_offset, subscription_report};
use crate::{
    Ctx, Result, Snapshot, cash, catalog, forecast, groups, limits, plans, review, trends,
};
use crate::{allocation, budget, daily, outlook};

/// `forecaster` (TimesFM) is the model the daily, group and outlook
/// forecasts backtest against their fallbacks (none: the fallbacks alone).
pub fn dashboard(
    store: &Store,
    snap: &Snapshot,
    now: &Zoned,
    forecaster: Option<&dyn daily::SeriesForecaster>,
) -> Result<Dashboard> {
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
    let spending = budget::spending_transactions(&ctx, &payments);

    let fixed_categories =
        groups::fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    let fixed_ids = groups::non_variable_ids(txs, &fixed_categories);
    let time_trends =
        trends::time_trends(&spending, &snap.group_mappings, &ctx.now, &HashSet::new());
    let current_month = ctx.current_month();
    let mut forecast_excluded = fixed_ids.clone();
    forecast_excluded.extend(plans::paid_outflow_ids(&payments));
    let spending_forecasts = forecast::spending_forecasts(
        &spending,
        &snap.group_mappings,
        &ctx.now,
        &forecast_excluded,
        |t| ctx.resolver.month_of(t),
        forecast::ForecastPeriod {
            current_month,
            period_start: ctx.resolver.period_start_date(current_month),
        },
        forecaster,
    );

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

    let budgets = budget::monthly_budgets(
        &ctx,
        &spending,
        &payments,
        &current_subscriptions,
        &next_subscriptions,
    );
    let spending_nudges = limits::spending_nudges(&snap.spending_limits, &time_trends);
    let usage: Vec<allocation::GroupUsage> = spending_nudges
        .iter()
        .map(|n| allocation::GroupUsage {
            currency: n.currency.clone(),
            group: n.group.clone(),
            spent: n.spent,
            projected_month_end: n.projected_month_end,
        })
        .collect();
    let goal_evaluations = if snap.settings.savings_goal_contributions_enabled {
        weekly.goals.clone()
    } else {
        Vec::new()
    };
    let (savings_accounts, savings_goals) = allocation::savings_roles(&ctx);
    let monthly_allocation = allocation::monthly_allocation(&allocation::AllocationInputs {
        through_date: cash
            .through_date
            .parse()
            .map_err(|_| crate::Error::Invalid("through date".into()))?,
        accounts: &snap.accounts,
        goals: &snap.goals,
        goal_evaluations: &goal_evaluations,
        payday_status: cash.payday_status.as_ref(),
        current_subscriptions: &current_subscriptions,
        next_subscriptions: &next_subscriptions,
        planned_payments: &payments,
        spending_limits: &snap.spending_limits,
        spending_nudges: &usage,
        budgets: Some(&budgets),
        savings_account_ids: &savings_accounts,
        current_month: Some(current_month),
        savings_goal_ids: &savings_goals,
        period_start: Some(ctx.resolver.period_start_date(current_month)),
    });

    // Forecast horizons run from today, so they keep calendar months.
    let future_subscriptions = (0..4)
        .map(|offset| subscription_report(&ctx, BillWindow::CalendarMonth(offset)))
        .collect::<Result<Vec<_>>>()?;
    let budget_fixed_ids = groups::fixed_transaction_ids(&spending, &fixed_categories);
    let savings_outlooks = outlook::savings_outlooks(&outlook::OutlookInputs {
        transactions: &spending,
        accounts: &snap.accounts,
        plans: &snap.budget_plans,
        mappings: &snap.group_mappings,
        payments: &payments,
        subscriptions: &future_subscriptions,
        fixed_ids: &budget_fixed_ids,
        payday_day: snap.settings.payday_day,
        now: &ctx.now,
        data_fresh: outlook::data_fresh(&snap.sync_health, &ctx.now),
        current_month,
        resolver: &ctx.resolver,
        forecaster,
    });
    let mut currencies: Vec<finance_types::report::Currency> = snap
        .accounts
        .iter()
        .map(|a| a.currency().clone())
        .chain(spending.iter().map(|t| t.currency.clone()))
        .collect();
    currencies.sort_by(|a, b| a.code.cmp(&b.code));
    currencies.dedup();
    let forecast_plans = daily::forecast_plans(&snap.budget_plans, current_month, &currencies);
    let daily_forecasts = daily::daily_spending_forecasts(&daily::DailyInputs {
        transactions: &spending,
        payments: &payments,
        subscriptions: &future_subscriptions,
        fixed_ids: &budget_fixed_ids,
        now: &ctx.now,
        current_month,
        month_start: ctx.resolver.period_start_date(current_month),
        budget_plans: &forecast_plans,
        mappings: &snap.group_mappings,
        excluded_categories: &snap.settings.forecast_excluded_categories,
        forecaster,
    });

    // The unplanned estimate and the varying bills, for the money plan.
    let expected_unplanned = savings_outlooks
        .iter()
        .find(|o| o.currency.code == "EUR")
        .map_or(rust_decimal::Decimal::ZERO, |o| o.expected_unplanned_month);
    let predicted_bills = crate::bill_forecast::predicted_bills(
        txs,
        &snap.settings.predicted_bill_categories,
        "EUR",
        current_month,
        |t| ctx.resolver.month_of(t),
        forecaster,
    );
    let money_plan = monthly_allocation
        .currencies
        .iter()
        .find(|c| c.currency.code == "EUR")
        .map(|allocation| {
            let roles: Vec<finance_types::gateway::AccountRole> =
                snap.account_roles.values().cloned().collect();
            let accounts = crate::writes::closure::checked_accounts(snap, &roles);
            let budget = snap
                .budget_plans
                .iter()
                .find(|p| p.month == allocation.current_month && p.currency.code == "EUR");
            let wishes = crate::wishes::store::load_wishes(store.connection())?;
            Ok::<_, crate::Error>(crate::closure::plan::money_plan(
                &crate::closure::plan::PlanInputs {
                    as_of: ctx.now.date(),
                    allocation,
                    budget,
                    payments: &payments,
                    accounts: &accounts,
                    roles: &roles,
                    wishes: &wishes,
                    salary_account_id: snap.settings.salary_account_id.as_deref(),
                    expected_unplanned,
                    predicted_bills: &predicted_bills,
                },
            ))
        })
        .transpose()?;
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
        variable_time_trends: trends::time_trends(
            &spending,
            &snap.group_mappings,
            &ctx.now,
            &fixed_ids,
        ),
        spending_limits: snap.spending_limits.clone(),
        spending_nudges,
        monthly_budgets: budgets,
        savings_outlooks,
        daily_forecasts,
        monthly_allocation: Some(monthly_allocation),
        money_plan,
        spending_forecasts,
        time_trends,
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
