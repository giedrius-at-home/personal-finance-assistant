//! Daily habit-spending forecast with planned payments as known future flows
//! (Python `daily_forecast.py`).
//!
//! Habit spending is every outflow net of refunds that is not a bill,
//! recurring direct debit, fixed category or planned payment. A
//! [`SeriesForecaster`] (TimesFM, when the app has its weights) forecasts that
//! series day by day; planned payments and unpaid bills are known amounts,
//! added on top and offered to the model as a past-and-future covariate. A
//! rolling weekly backtest keeps whichever of TimesFM (`timesfm`), TimesFM with
//! planned-payment covariates (`timesfm_planned`) or a same-weekday baseline
//! (`weekday_baseline`) has been most accurate on this owner's own history,
//! and calibrates it by its average miss. Without a forecaster (or when it
//! fails) the baseline is the only candidate. The model learns from everyday
//! spending only (see [`everyday_inputs`]); spent-so-far totals keep every
//! habit outflow.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{BudgetPlan, Direction, GroupMapping};
use finance_types::report::Currency;
use finance_types::report::outlook::{
    DailyPoint, DailySpendingForecast, ExcludedCategory, FlowSource, ForecastExclusions,
    GroupMonthForecast, MonthHabitForecast, PlannedFlow, WeekdayHabit,
};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::subscriptions::SubscriptionReport;
use jiff::Zoned;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::budget::{budget_owner, default_budget};
use crate::groups::{group_index, is_recurring_debit};
use crate::model::{Kind, Tx};
use crate::outlook::{add_days, days_between, np, plans_by_month};
use crate::period::shift_month;
use crate::plans::paid_outflow_ids;
use crate::text::casefold;

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MIN_HISTORY_DAYS: i64 = 28;
const BACKTEST_WEEKS: usize = 8;
const MAX_CONTEXT_DAYS: i64 = 730;
/// Everyday forecast inputs: larger single outflows are one-offs, not habits.
pub const EVERYDAY_SINGLE_PURCHASE_LIMIT: i64 = 150;
const EXCLUSION_SUMMARY_DAYS: i64 = 183;
const BASELINE: &str = "weekday_baseline";
const TIMESFM: &str = "timesfm";
const TIMESFM_PLANNED: &str = "timesfm_planned";
/// Backtest windows need this much history before their origin.
const MIN_BACKTEST_CONTEXT: usize = 56;

/// A model that forecasts daily series (Python `SeriesForecaster`): one
/// `(q10, mean, q90)` row per future day for each context. The mean, not the
/// median, is the daily estimate: most days have no habit spending, so daily
/// medians are zero and would understate every total.
pub trait SeriesForecaster: std::fmt::Debug {
    /// The name daily forecasts report as their model.
    fn name(&self) -> &str;

    /// Whether `quantiles` accepts past-and-future covariates.
    fn supports_covariates(&self) -> bool;

    /// Forecast `horizon` days after each context. A covariate, when given,
    /// covers its context and the horizon.
    fn quantiles(
        &self,
        contexts: &[Vec<f64>],
        horizon: usize,
        covariates: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<[f64; 3]>>, String>;
}

/// Everything the forecast reads. `now` is in the owner's time zone.
#[derive(Clone, Copy, Debug)]
pub struct DailyInputs<'a> {
    pub transactions: &'a [Tx],
    pub payments: &'a [PlannedPaymentView],
    /// Calendar-month bill reports for this month and the three after it.
    pub subscriptions: &'a [SubscriptionReport],
    pub fixed_ids: &'a HashSet<String>,
    pub now: &'a Zoned,
    /// The running budget month (after an early cutoff, the next one).
    pub current_month: Date,
    /// The first local day counted as spent in `current_month`.
    pub month_start: Date,
    /// This month's and next month's plans; see [`forecast_plans`].
    pub budget_plans: &'a [BudgetPlan],
    pub mappings: &'a [GroupMapping],
    pub excluded_categories: &'a [String],
    /// The model to backtest against the baseline; `None` keeps the baseline.
    pub forecaster: Option<&'a dyn SeriesForecaster>,
}

/// One habit transaction: a positive outflow or a negative refund.
#[derive(Clone, Debug, PartialEq)]
pub struct HabitOutflow {
    pub day: Date,
    pub amount: f64,
    pub category: Option<String>,
    /// (budget month, group id) pairs this transaction counts towards.
    pub owners: Vec<(Date, String)>,
}

/// Each budget month's plan for the forecast, else the month before's (as
/// month closure does), else the default plan.
pub fn forecast_plans(
    saved: &[BudgetPlan],
    current_month: Date,
    currencies: &[Currency],
) -> Vec<BudgetPlan> {
    let by_month = plans_by_month(saved);
    let mut currencies = currencies.to_vec();
    currencies.sort_by(|a, b| a.code.cmp(&b.code));
    [current_month, shift_month(current_month, 1)]
        .into_iter()
        .flat_map(|month| currencies.iter().map(move |c| (month, c)))
        .map(|(month, currency)| {
            if let Some(plan) = by_month.get(&(month, currency.clone())) {
                return (*plan).clone();
            }
            let mut plan = by_month
                .get(&(shift_month(month, -1), currency.clone()))
                .map_or_else(|| default_budget(month, currency), |p| (*p).clone());
            plan.month = month.to_string();
            plan
        })
        .collect()
}

/// A date-keyed float accumulator that keeps first-insertion order, like
/// the Python `defaultdict` the compensated sums iterate over.
#[derive(Clone, Debug, Default)]
struct DayTotals {
    order: Vec<Date>,
    values: HashMap<Date, f64>,
}

impl DayTotals {
    fn add(&mut self, day: Date, amount: f64) {
        let value = self.values.entry(day).or_insert_with(|| {
            self.order.push(day);
            0.0
        });
        *value += amount;
    }

    fn get(&self, day: Date) -> f64 {
        self.values.get(&day).copied().unwrap_or(0.0)
    }

    fn sum_where(&self, keep: impl Fn(Date) -> bool) -> f64 {
        np::py_sum(
            self.order
                .iter()
                .filter(|d| keep(**d))
                .map(|d| self.values[d]),
        )
    }
}

/// `Decimal(str(round(value, 2))).quantize(Decimal("0.01"))`.
fn money(value: f64) -> Decimal {
    let mut cents = np::round_decimal(value, 2);
    cents.rescale(2);
    cents
}

/// The habit rows the forecast learns from: excluded categories and single
/// amounts over the limit are dropped. The summary covers outflows dropped
/// from `since` on.
pub fn everyday_inputs(
    rows: &[HabitOutflow],
    since: Date,
    excluded_categories: &[String],
) -> (Vec<HabitOutflow>, ForecastExclusions) {
    let excluded: HashSet<String> = excluded_categories
        .iter()
        .map(|c| casefold(c.trim()))
        .collect();
    let limit = EVERYDAY_SINGLE_PURCHASE_LIMIT as f64;
    let mut dropped: Vec<(String, Vec<f64>)> = Vec::new();
    let mut large = Vec::new();
    let mut kept = Vec::new();
    for row in rows {
        let category = row.category.as_deref().filter(|c| !c.is_empty());
        if let Some(category) = category.filter(|c| excluded.contains(&casefold(c.trim()))) {
            if row.amount > 0.0 && row.day >= since {
                match dropped.iter_mut().find(|(name, _)| name == category) {
                    Some((_, amounts)) => amounts.push(row.amount),
                    None => dropped.push((category.to_owned(), vec![row.amount])),
                }
            }
        } else if row.amount.abs() > limit {
            if row.amount > 0.0 && row.day >= since {
                large.push(row.amount);
            }
        } else {
            kept.push(row.clone());
        }
    }
    let mut categories: Vec<(String, Vec<f64>, f64)> = dropped
        .into_iter()
        .map(|(name, amounts)| {
            let total = np::py_sum(amounts.iter().copied());
            (name, amounts, total)
        })
        .collect();
    categories.sort_by(|a, b| b.2.total_cmp(&a.2));
    let summary = ForecastExclusions {
        since: since.to_string(),
        limit: Decimal::from(EVERYDAY_SINGLE_PURCHASE_LIMIT),
        large_count: large.len(),
        large_amount: money(np::py_sum(large.iter().copied())),
        categories: categories
            .into_iter()
            .map(|(category, amounts, total)| ExcludedCategory {
                category,
                count: amounts.len(),
                amount: money(total),
            })
            .collect(),
    };
    (kept, summary)
}

/// Same-weekday mean of the last `weeks` weeks with an empirical 10–90% band:
/// `(q10, mean, q90)` per future day.
pub fn weekday_baseline(context: &[f64], horizon: usize, weeks: usize) -> Vec<[f64; 3]> {
    let len = context.len() as i64;
    (0..horizon)
        .map(|step| {
            let lag = 7 - (step % 7) as i64;
            // Python's `values[len - lag::-7]`, negative start counting from the end.
            let mut first = len - lag;
            if first < 0 {
                first += len;
            }
            let mut samples: Vec<f64> = if first < 0 {
                Vec::new()
            } else {
                (0..=first)
                    .rev()
                    .step_by(7)
                    .take(weeks)
                    .map(|i| context[i as usize])
                    .collect()
            };
            if samples.is_empty() {
                samples = if context.is_empty() {
                    vec![0.0]
                } else {
                    context[context.len().saturating_sub(7)..].to_vec()
                };
            }
            [
                np::quantile(&samples, 0.1),
                np::mean(&samples),
                np::quantile(&samples, 0.9),
            ]
        })
        .collect()
}

/// Remove the method's average weekly miss from its daily estimates. Lumpy
/// spending sits mostly above a model's upper quantiles, so raw daily
/// estimates can run low; the backtest measures that bias on this history.
fn calibrated(rows: &[[f64; 3]], residuals: &[f64]) -> (Vec<[f64; 3]>, f64) {
    let rows: Vec<[f64; 3]> = rows.iter().map(|r| r.map(|v| v.max(0.0))).collect();
    if residuals.len() < 4 {
        return (rows, 0.0);
    }
    let bias = np::mean(residuals);
    let week = np::sum_iter(rows.iter().take(7).map(|r| r[1]));
    let rows = rows
        .iter()
        .map(|r| {
            let mean = if week > 0.01 {
                r[1] * py_max(0.0, week + bias) / week
            } else {
                (r[1] + bias / 7.0).max(0.0)
            };
            [r[0].min(mean), mean, r[2].max(mean)]
        })
        .collect();
    (rows, bias)
}

/// Python's `max(a, b)`: `a` unless `b` is larger.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Range for a multi-day total from past weekly misses, when there are some.
fn band(
    expected: f64,
    daily: &[[f64; 3]],
    residuals: &[f64],
    bias: f64,
    days: usize,
) -> (f64, f64) {
    if residuals.len() >= 4 && days > 0 {
        let scale = (days as f64 / 7.0).powf(0.5);
        let low = (np::quantile(residuals, 0.1) - bias) * scale;
        let high = (np::quantile(residuals, 0.9) - bias) * scale;
        return (
            py_max(0.0, py_min(expected, expected + low)),
            py_max(expected, expected + high),
        );
    }
    // Otherwise sum daily ranges, which assumes co-moving days: deliberately wide.
    let head = &daily[..days.min(daily.len())];
    (
        np::sum_iter(head.iter().map(|r| r[0])),
        np::sum_iter(head.iter().map(|r| r[2])),
    )
}

/// Unpaid planned payments and bills from today through `end`. An overdue
/// outflow is still owed, so it lands on today; an overdue deposit is dropped.
fn upcoming(
    currency: &Currency,
    today: Date,
    end: Date,
    payments: &[PlannedPaymentView],
    subscriptions: &[SubscriptionReport],
) -> Vec<(Date, PlannedFlow)> {
    let mut flows = Vec::new();
    for p in payments {
        if p.amount.currency != *currency || p.status == PlanStatus::Paid {
            continue;
        }
        let Ok(due_on) = p.due_on.parse::<Date>() else {
            continue;
        };
        if p.direction == Direction::Inflow && due_on < today {
            continue;
        }
        let day = today.max(due_on);
        if day <= end {
            flows.push((
                day,
                PlannedFlow {
                    day: day.to_string(),
                    name: p.name.clone(),
                    amount: p.amount.amount,
                    direction: p.direction,
                    source: FlowSource::PlannedPayment,
                },
            ));
        }
    }
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for bill in subscriptions.iter().flat_map(|r| &r.scheduled_payments) {
        if bill.amount.currency != *currency
            || bill.paid_on.is_some()
            || !seen.insert((&bill.bill_id, &bill.due_on))
        {
            continue;
        }
        let Ok(due_on) = bill.due_on.parse::<Date>() else {
            continue;
        };
        let day = today.max(due_on);
        if day <= end {
            flows.push((
                day,
                PlannedFlow {
                    day: day.to_string(),
                    name: bill.name.clone(),
                    amount: bill.amount.amount,
                    direction: Direction::Outflow,
                    source: FlowSource::Bill,
                },
            ));
        }
    }
    flows.sort_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    flows
}

type GroupKey = (Currency, Date, String);

struct Prepared {
    currency: Currency,
    first: Date,
    series: Vec<f64>,
    /// Planned payments and bills paid on each context day.
    past_planned: Vec<f64>,
    future_planned: Vec<f64>,
    future_income: Vec<f64>,
    upcoming: Vec<(Date, PlannedFlow)>,
    exclusions: ForecastExclusions,
}

/// Habit spending forecast from today through the budget month's end (at
/// least a week), per currency with at least four weeks of history. The
/// baseline runs on through next month's end so `months` can total both
/// budget months, split over the groups of each month's plan.
pub fn daily_spending_forecasts(input: &DailyInputs) -> Vec<DailySpendingForecast> {
    let now = input.now;
    let today = now.date();
    let month = input.current_month;
    let month_start = input.month_start;
    let month_end = today.max(month.last_of_month());
    let next_month = shift_month(month, 1);
    let next_end = next_month.last_of_month();
    // Shown days and flows stop at month end; the model runs on through next month.
    let shown = usize::try_from(days_between(today, month_end) + 1)
        .unwrap_or(0)
        .max(7);
    let horizon = usize::try_from(days_between(today, next_end) + 1)
        .unwrap_or(0)
        .max(shown);
    let end = add_days(today, horizon as i64 - 1);
    let shown_end = add_days(today, shown as i64 - 1);
    let spans = [
        (month, month_start, month_end),
        (next_month, next_month, next_end),
    ];
    let group_of = group_index(input.mappings);
    let plans: HashMap<(Date, Currency), &BudgetPlan> = plans_by_month(input.budget_plans)
        .into_iter()
        .filter(|(_, p)| !p.groups.is_empty())
        .collect();
    let planned_ids = paid_outflow_ids(input.payments);

    let mut currency_order: Vec<Currency> = Vec::new();
    let mut habit: HashMap<Currency, DayTotals> = HashMap::new();
    let mut planned: HashMap<Currency, DayTotals> = HashMap::new();
    let mut counts: HashMap<Currency, HashMap<Date, usize>> = HashMap::new();
    let mut grouped: HashMap<GroupKey, DayTotals> = HashMap::new();
    let mut habit_rows: HashMap<Currency, Vec<HabitOutflow>> = HashMap::new();
    for t in input.transactions {
        if !matches!(t.kind, Kind::Outflow | Kind::Refund)
            || t.occurred_at.timestamp() > now.timestamp()
        {
            continue;
        }
        let on = t.occurred_at.date();
        let currency = &t.currency;
        let magnitude = np::to_f64(t.amount);
        let amount = if t.kind == Kind::Refund {
            -magnitude
        } else {
            magnitude
        };
        // Only bills and explicit plans are knowable in advance; other fixed
        // debits (and the bank's recurring collections) simply stay out of habits.
        if t.subscription_id.is_some() || planned_ids.contains(&t.transaction_id) {
            planned.entry(currency.clone()).or_default().add(on, amount);
        } else if !input.fixed_ids.contains(&t.transaction_id)
            && !is_recurring_debit(t.description.as_deref())
        {
            if !habit.contains_key(currency) {
                currency_order.push(currency.clone());
            }
            habit.entry(currency.clone()).or_default().add(on, amount);
            if t.kind == Kind::Outflow {
                *counts
                    .entry(currency.clone())
                    .or_default()
                    .entry(on)
                    .or_default() += 1;
            }
            let mut owners = Vec::new();
            for (budget_month, _, _) in spans {
                let Some(plan) = plans.get(&(budget_month, currency.clone())) else {
                    continue;
                };
                let source = group_of
                    .get(t.category.as_deref().unwrap_or(""))
                    .copied()
                    .flatten();
                if let Some(owner) = budget_owner(
                    &plan.groups,
                    t.category.as_deref(),
                    source,
                    Some(&t.transaction_id),
                ) {
                    grouped
                        .entry((currency.clone(), budget_month, owner.to_owned()))
                        .or_default()
                        .add(on, amount);
                    owners.push((budget_month, owner.to_owned()));
                }
            }
            habit_rows
                .entry(currency.clone())
                .or_default()
                .push(HabitOutflow {
                    day: on,
                    amount,
                    category: t.category.clone(),
                    owners,
                });
        }
    }

    // The everyday series each group's model learns from.
    let mut model_grouped: HashMap<GroupKey, DayTotals> = HashMap::new();
    let mut prepared = Vec::new();
    currency_order.sort_by(|a, b| a.code.cmp(&b.code));
    let empty = DayTotals::default();
    for currency in currency_order {
        let totals = &habit[&currency];
        let Some(earliest) = totals.order.iter().min().copied() else {
            continue;
        };
        let first = earliest.max(add_days(today, -MAX_CONTEXT_DAYS));
        let length = days_between(first, today);
        if length < MIN_HISTORY_DAYS {
            continue;
        }
        let (kept, exclusions) = everyday_inputs(
            &habit_rows[&currency],
            add_days(today, -EXCLUSION_SUMMARY_DAYS),
            input.excluded_categories,
        );
        let mut daily_model = DayTotals::default();
        for row in &kept {
            daily_model.add(row.day, row.amount);
            for (budget_month, owner) in &row.owners {
                model_grouped
                    .entry((currency.clone(), *budget_month, owner.clone()))
                    .or_default()
                    .add(row.day, row.amount);
            }
        }
        let flows = upcoming(&currency, today, end, input.payments, input.subscriptions);
        let series: Vec<f64> = (0..length)
            .map(|i| py_max(0.0, daily_model.get(add_days(first, i))))
            .collect();
        let paid = planned.get(&currency).unwrap_or(&empty);
        let past_planned: Vec<f64> = (0..length).map(|i| paid.get(add_days(first, i))).collect();
        let mut future_planned = vec![0.0; horizon];
        let mut future_income = vec![0.0; horizon];
        for (day, flow) in &flows {
            let target = if flow.direction == Direction::Inflow {
                &mut future_income
            } else {
                &mut future_planned
            };
            let index = usize::try_from(days_between(today, *day)).unwrap_or(0);
            target[index] += np::to_f64(flow.amount);
        }
        prepared.push(Prepared {
            currency,
            first,
            series,
            past_planned,
            future_planned,
            future_income,
            upcoming: flows,
            exclusions,
        });
    }

    // One batched call per candidate covers every backtest window of every currency.
    let windows = backtest_windows(&prepared);
    let contexts: Vec<Vec<f64>> = windows
        .iter()
        .map(|w| prepared[w.index].series[..w.origin].to_vec())
        .collect();
    let mut forecaster = input.forecaster;
    let mut residuals = Residuals::default();
    if !windows.is_empty() {
        let baseline: Vec<_> = contexts.iter().map(|c| weekday_baseline(c, 7, 4)).collect();
        residuals.score(BASELINE, &windows, &prepared, &baseline);
        if let Some(model) = forecaster {
            let scored = model.quantiles(&contexts, 7, None).and_then(|rows| {
                residuals.score(TIMESFM, &windows, &prepared, &rows);
                if model.supports_covariates() {
                    let covariates: Vec<Vec<f64>> = windows
                        .iter()
                        .map(|w| prepared[w.index].past_planned[..w.origin + 7].to_vec())
                        .collect();
                    let rows = model.quantiles(&contexts, 7, Some(&covariates))?;
                    residuals.score(TIMESFM_PLANNED, &windows, &prepared, &rows);
                }
                Ok(())
            });
            // The baseline keeps forecasts alive when the model fails.
            if scored.is_err() {
                forecaster = None;
            }
        }
    }

    let live_contexts: Vec<Vec<f64>> = prepared.iter().map(|p| p.series.clone()).collect();
    let mut live: Vec<(&str, Vec<Vec<[f64; 3]>>)> = vec![(
        BASELINE,
        live_contexts
            .iter()
            .map(|c| weekday_baseline(c, horizon, 4))
            .collect(),
    )];
    if let Some(model) = forecaster {
        let forecasts = model
            .quantiles(&live_contexts, horizon, None)
            .and_then(|plain| {
                let mut out = vec![(TIMESFM, plain)];
                if model.supports_covariates() {
                    let covariates: Vec<Vec<f64>> = prepared
                        .iter()
                        .map(|p| [&p.past_planned[..], &p.future_planned[..]].concat())
                        .collect();
                    out.push((
                        TIMESFM_PLANNED,
                        model.quantiles(&live_contexts, horizon, Some(&covariates))?,
                    ));
                }
                Ok(out)
            });
        if let Ok(forecasts) = forecasts {
            live.extend(forecasts);
        }
    }
    let available: Vec<&str> = live.iter().map(|(name, _)| *name).collect();

    let mut results = Vec::new();
    for (index, p) in prepared.iter().enumerate() {
        // Rank methods after removing each one's own average miss.
        let errors: Vec<(&str, f64)> = residuals
            .methods
            .iter()
            .filter_map(|(name, by_series)| {
                let misses = by_series.get(&index).filter(|m| !m.is_empty())?;
                let center = np::mean(misses);
                let spread: Vec<f64> = misses.iter().map(|m| (m - center).abs()).collect();
                Some((*name, np::mean(&spread)))
            })
            .collect();
        let error_of = |name: &str| errors.iter().find(|(n, _)| *n == name).map(|e| e.1);
        let scored: Vec<(&str, f64)> = errors
            .iter()
            .copied()
            .filter(|(name, _)| available.contains(name))
            .collect();
        let method = choose(&scored, &available);
        let misses = residuals.misses(method, index);
        let daily = &live
            .iter()
            .find(|(name, _)| *name == method)
            .map_or(&[][..], |(_, rows)| &rows[..])[index];
        let (rows, bias) = calibrated(daily, misses);
        let next_week = np::sum_iter(rows.iter().take(7).map(|r| r[1]));
        let (week_low, week_high) = band(next_week, &rows, misses, bias, 7);
        let habit_days = habit.get(&p.currency).unwrap_or(&empty);
        let months = month_forecasts(&MonthInputs {
            currency: &p.currency,
            today,
            first: p.first,
            length: p.series.len(),
            horizon,
            spans: &spans,
            plans: &plans,
            habit: habit_days,
            grouped: &grouped,
            model_grouped: &model_grouped,
            rows: &rows,
            forecaster: forecaster.filter(|_| method != BASELINE),
        });
        let days = (0..shown)
            .map(|step| DailyPoint {
                day: add_days(today, step as i64).to_string(),
                habit_expected: money(rows[step][1]),
                habit_lower: money(rows[step][0]),
                habit_upper: money(rows[step][2]),
                planned: money(p.future_planned[step]),
                expected_income: money(p.future_income[step]),
            })
            .collect();
        let in_month = usize::try_from(days_between(today, month_end) + 1).unwrap_or(0);
        let spent = habit_days.sum_where(|d| month_start <= d && d < today);
        let planned_paid = planned
            .get(&p.currency)
            .unwrap_or(&empty)
            .sum_where(|d| month_start <= d && d <= today);
        let training = p.series.len();
        let actual: Vec<f64> = (0..training)
            .map(|i| py_max(0.0, habit_days.get(add_days(p.first, i as i64))))
            .collect();
        let window = training.min(84);
        let day_counts = counts.get(&p.currency);
        let spend_days: Vec<bool> = (0..84)
            .map(|i| {
                let day = add_days(today, -(84 - i));
                day_counts.and_then(|c| c.get(&day)).is_some_and(|n| *n > 0)
            })
            .skip(84 - window)
            .collect();
        let first_weekday = add_days(today, -(window as i64))
            .weekday()
            .to_monday_zero_offset();
        let habits = weekday_habits(
            &actual[training - window..],
            &spend_days,
            usize::try_from(first_weekday).unwrap_or(0),
        );
        let rest = np::sum_iter(rows.iter().take(in_month).map(|r| r[1]));
        let (rest_low, rest_high) = band(rest, &rows, misses, bias, in_month);
        let sum_head = |values: &[f64], n: usize| np::sum(&values[..n.min(values.len())]);
        results.push(DailySpendingForecast {
            currency: p.currency.clone(),
            as_of: today.to_string(),
            method: method.into(),
            model: match forecaster {
                Some(model) if method != BASELINE => model.name().into(),
                _ => "weekday-baseline".into(),
            },
            habit_spent_this_month: money(spent),
            planned_paid_this_month: money(planned_paid),
            days,
            next_week_habit: money(next_week),
            next_week_lower: money(week_low),
            next_week_upper: money(week_high),
            next_week_planned: money(sum_head(&p.future_planned, 7)),
            month_end_habit: money(spent + rest),
            month_end_lower: money(spent + rest_low),
            month_end_upper: money(spent + rest_high),
            month_remaining_planned: money(sum_head(&p.future_planned, in_month)),
            month_remaining_income: money(sum_head(&p.future_income, in_month)),
            upcoming: p
                .upcoming
                .iter()
                .filter(|(day, _)| *day <= shown_end)
                .map(|(_, flow)| flow.clone())
                .collect(),
            habits,
            training_days: training,
            backtest_windows: windows.iter().filter(|w| w.index == index).count(),
            model_error: error_of(TIMESFM).map(money),
            planned_model_error: error_of(TIMESFM_PLANNED).map(money),
            baseline_error: error_of(BASELINE).map(money),
            months,
            excluded: Some(p.exclusions.clone()),
        });
    }
    results
}

/// One weekly backtest window: forecast the 7 days from `origin` on.
struct Window {
    index: usize,
    origin: usize,
}

/// Up to [`BACKTEST_WEEKS`] weekly windows per series, newest first, each
/// with at least [`MIN_BACKTEST_CONTEXT`] days of history.
fn backtest_windows(prepared: &[Prepared]) -> Vec<Window> {
    let mut windows = Vec::new();
    for (index, p) in prepared.iter().enumerate() {
        for week in 1..=BACKTEST_WEEKS {
            let Some(origin) = p.series.len().checked_sub(7 * week) else {
                break;
            };
            if origin < MIN_BACKTEST_CONTEXT {
                break;
            }
            windows.push(Window { index, origin });
        }
    }
    windows
}

/// Signed weekly misses (actual minus expected) per method and series, in
/// the order the methods were scored.
#[derive(Default)]
struct Residuals {
    methods: Vec<(&'static str, HashMap<usize, Vec<f64>>)>,
}

impl Residuals {
    fn score(
        &mut self,
        method: &'static str,
        windows: &[Window],
        prepared: &[Prepared],
        predictions: &[Vec<[f64; 3]>],
    ) {
        let mut by_series: HashMap<usize, Vec<f64>> = HashMap::new();
        for (window, prediction) in windows.iter().zip(predictions) {
            let series = &prepared[window.index].series;
            let actual = np::sum(&series[window.origin..window.origin + 7]);
            let expected = np::sum_iter(prediction.iter().map(|r| r[1]));
            by_series
                .entry(window.index)
                .or_default()
                .push(actual - expected);
        }
        self.methods.push((method, by_series));
    }

    fn misses(&self, method: &str, index: usize) -> &[f64] {
        self.methods
            .iter()
            .find(|(name, _)| *name == method)
            .and_then(|(_, by_series)| by_series.get(&index))
            .map_or(&[], Vec::as_slice)
    }
}

/// Lowest backtest error wins; ties favour using the planned payments. With
/// too little history to backtest, TimesFM is preferred over the baseline.
fn choose(scored: &[(&'static str, f64)], available: &[&'static str]) -> &'static str {
    let mut best: Option<(&'static str, f64)> = None;
    for &(name, error) in scored {
        let key = (error, name != TIMESFM_PLANNED);
        let better = best.is_none_or(|(best_name, best_error)| {
            key.0
                .total_cmp(&best_error)
                .then(key.1.cmp(&(best_name != TIMESFM_PLANNED)))
                .is_lt()
        });
        if better {
            best = Some((name, error));
        }
    }
    match best {
        Some((name, _)) => name,
        None if available.len() > 1 => {
            if available.contains(&TIMESFM_PLANNED) {
                TIMESFM_PLANNED
            } else {
                TIMESFM
            }
        }
        None => BASELINE,
    }
}

struct MonthInputs<'a> {
    currency: &'a Currency,
    today: Date,
    first: Date,
    length: usize,
    horizon: usize,
    spans: &'a [(Date, Date, Date); 2],
    plans: &'a HashMap<(Date, Currency), &'a BudgetPlan>,
    habit: &'a DayTotals,
    grouped: &'a HashMap<GroupKey, DayTotals>,
    model_grouped: &'a HashMap<GroupKey, DayTotals>,
    rows: &'a [[f64; 3]],
    /// The model for group series: only when the backtest chose a model.
    forecaster: Option<&'a dyn SeriesForecaster>,
}

/// Spent so far plus the forecast rest of each budget month and group. Each
/// group's own everyday history is forecast with the method the backtest
/// chose for all habit spending (without planned-payment covariates).
fn month_forecasts(m: &MonthInputs) -> Vec<MonthHabitForecast> {
    let today = m.today;
    let rest = |start: Date, stop: Date, daily: &[[f64; 3]]| -> f64 {
        let from = usize::try_from(days_between(today, start).max(0)).unwrap_or(0);
        let to = usize::try_from((days_between(today, stop) + 1).max(0)).unwrap_or(0);
        let to = to.min(daily.len());
        np::sum_iter(daily[from.min(to)..to].iter().map(|r| r[1]))
    };
    let total = |start: Date, stop: Date, actual: &DayTotals, daily: &[[f64; 3]]| -> Decimal {
        let spent = actual.sum_where(|d| start <= d && d < today);
        money(spent + rest(start, stop, daily))
    };
    let empty = DayTotals::default();
    let mut keys: Vec<(Date, &str, &DayTotals)> = Vec::new();
    let mut contexts: Vec<Vec<f64>> = Vec::new();
    for &(target, _, _) in m.spans {
        let groups = m
            .plans
            .get(&(target, m.currency.clone()))
            .map_or(&[][..], |p| &p.groups[..]);
        for group in groups {
            let key = (m.currency.clone(), target, group.id.clone());
            let learned = m.model_grouped.get(&key).unwrap_or(&empty);
            keys.push((target, &group.id, m.grouped.get(&key).unwrap_or(&empty)));
            contexts.push(
                (0..m.length)
                    .map(|i| py_max(0.0, learned.get(add_days(m.first, i as i64))))
                    .collect(),
            );
        }
    }
    let modelled = match m.forecaster {
        Some(model) if !contexts.is_empty() => model
            .quantiles(&contexts, m.horizon, None)
            .ok()
            .filter(|rows| !rows.is_empty()),
        _ => None,
    };
    let group_rows: Vec<Vec<[f64; 3]>> = modelled
        .unwrap_or_else(|| {
            contexts
                .iter()
                .map(|c| weekday_baseline(c, m.horizon, 4))
                .collect()
        })
        .into_iter()
        .map(|rows| rows.into_iter().map(|r| r.map(|v| v.max(0.0))).collect())
        .collect();
    m.spans
        .iter()
        .map(|&(target, start, stop)| MonthHabitForecast {
            month: target.to_string(),
            habit: total(start, stop, m.habit, m.rows),
            remaining: money(rest(start, stop, m.rows)),
            groups: keys
                .iter()
                .zip(&group_rows)
                .filter(|((month, _, _), _)| *month == target)
                .map(|((_, group_id, actual), daily)| GroupMonthForecast {
                    group_id: (*group_id).to_owned(),
                    expected: total(start, stop, actual, daily),
                    remaining: money(rest(start, stop, daily)),
                })
                .collect(),
        })
        .collect()
}

fn weekday_habits(values: &[f64], spend_days: &[bool], first_weekday: usize) -> Vec<WeekdayHabit> {
    (0..7)
        .filter_map(|weekday| {
            let picked: Vec<usize> = (0..values.len())
                .filter(|i| (first_weekday + i) % 7 == weekday)
                .collect();
            if picked.is_empty() {
                return None;
            }
            let average: Vec<f64> = picked.iter().map(|&i| values[i]).collect();
            let active: Vec<f64> = picked
                .iter()
                .map(|&i| f64::from(u8::from(spend_days[i])))
                .collect();
            Some(WeekdayHabit {
                weekday: WEEKDAYS[weekday].into(),
                average: money(np::mean(&average)),
                spend_day_share: money(np::mean(&active)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::UnitMoney;
    use finance_types::report::subscriptions::ScheduledBillPayment;
    use jiff::civil::Weekday;
    use jiff::civil::date;

    fn now() -> Zoned {
        "2026-09-24T18:00:00+00:00[UTC]".parse().unwrap()
    }

    fn eur(amount: &str) -> UnitMoney {
        UnitMoney {
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
        }
    }

    fn tx(day: Date, amount: &str, id: &str) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 1,
            account_id: "bank".into(),
            counterparty_account_id: None,
            kind: Kind::Outflow,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: day.at(12, 0, 0, 0).in_tz("UTC").unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: None,
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    /// Saturdays cost 60, every other day 10.
    fn history(days: i64) -> Vec<Tx> {
        let today = now().date();
        (1..=days)
            .map(|back| {
                let day = add_days(today, -back);
                let amount = if day.weekday() == Weekday::Saturday {
                    "60"
                } else {
                    "10"
                };
                tx(day, amount, &format!("t{back}"))
            })
            .collect()
    }

    fn planned(
        id: &str,
        name: &str,
        due: &str,
        status: PlanStatus,
        direction: Direction,
    ) -> PlannedPaymentView {
        PlannedPaymentView {
            plan_id: id.into(),
            name: name.into(),
            match_category: "Health".into(),
            direction,
            amount: eur("120"),
            due_on: due.into(),
            created_at: "2026-09-24T18:00:00Z".into(),
            reimbursement_for: None,
            pending: false,
            account_id: None,
            match_description: None,
            status,
            paid_on: None,
            transaction_id: None,
            candidate_count: 0,
            candidates: Vec::new(),
        }
    }

    fn bills(payments: Vec<ScheduledBillPayment>) -> SubscriptionReport {
        SubscriptionReport {
            fetched_at: None,
            through_date: "2026-09-24".into(),
            stale: false,
            rows: Vec::new(),
            expected_costs: Vec::new(),
            remaining_costs: Vec::new(),
            scheduled_payments: payments,
            unmatched_count: 0,
            evidence_start: "2026-09-24".into(),
            category_matched_names: Vec::new(),
        }
    }

    fn run(
        transactions: &[Tx],
        payments: &[PlannedPaymentView],
        subscriptions: &[SubscriptionReport],
        fixed: &HashSet<String>,
        excluded: &[String],
    ) -> Vec<DailySpendingForecast> {
        let now = now();
        daily_spending_forecasts(&DailyInputs {
            transactions,
            payments,
            subscriptions,
            fixed_ids: fixed,
            now: &now,
            current_month: date(2026, 9, 1),
            month_start: date(2026, 9, 1),
            budget_plans: &[],
            mappings: &[],
            excluded_categories: excluded,
            forecaster: None,
        })
    }

    #[test]
    fn weekday_baseline_repeats_the_same_weekday() {
        let week = [1.0, 2.0, 3.0, 4.0, 5.0, 60.0, 7.0];
        let context: Vec<f64> = week.iter().copied().cycle().take(28).collect();
        let rows = weekday_baseline(&context, 9, 4);
        let means: Vec<f64> = rows.iter().map(|r| r[1]).collect();
        assert_eq!(means, [1.0, 2.0, 3.0, 4.0, 5.0, 60.0, 7.0, 1.0, 2.0]);
    }

    #[test]
    fn baseline_learns_the_saturday_habit_and_adds_planned_flows() {
        let gym = ScheduledBillPayment {
            bill_id: "b1".into(),
            name: "Gym".into(),
            due_on: "2026-09-30".into(),
            paid_on: None,
            amount: eur("35"),
            candidate_ids: Vec::new(),
        };
        // The same bill appears in overlapping reports but is counted once.
        let report = bills(vec![gym.clone(), gym]);
        let payments = [
            planned(
                "p1",
                "Dentist",
                "2026-09-27",
                PlanStatus::Upcoming,
                Direction::Outflow,
            ),
            planned(
                "old",
                "Overdue parking fine",
                "2026-09-01",
                PlanStatus::Overdue,
                Direction::Outflow,
            ),
            planned(
                "pay",
                "Refund from friend",
                "2026-09-28",
                PlanStatus::Upcoming,
                Direction::Inflow,
            ),
        ];
        let forecast = run(
            &history(120),
            &payments,
            &[report.clone(), report],
            &HashSet::new(),
            &[],
        )
        .remove(0);
        assert_eq!(forecast.method, "weekday_baseline");
        let days: Vec<&str> = forecast.days.iter().map(|d| &d.day[8..]).collect();
        assert_eq!(days, ["24", "25", "26", "27", "28", "29", "30"]);
        let saturday = forecast
            .days
            .iter()
            .find(|d| d.day == "2026-09-26")
            .unwrap();
        assert_eq!(saturday.habit_expected, Decimal::new(6000, 2));
        assert_eq!(forecast.next_week_habit, Decimal::new(12000, 2));
        assert_eq!(
            forecast.month_end_habit,
            forecast.habit_spent_this_month + Decimal::from(120)
        );
        // Overdue plans are still owed today; a late deposit is not assumed.
        assert_eq!(forecast.days[0].planned, Decimal::from(120));
        assert_eq!(forecast.next_week_planned, Decimal::from(275));
        assert_eq!(forecast.month_remaining_income, Decimal::from(120));
        let names: Vec<&str> = forecast.upcoming.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Overdue parking fine",
                "Dentist",
                "Refund from friend",
                "Gym"
            ]
        );
        let saturday = forecast
            .habits
            .iter()
            .find(|h| h.weekday == "Saturday")
            .unwrap();
        assert_eq!(saturday.average, Decimal::from(60));
        let monday = forecast
            .habits
            .iter()
            .find(|h| h.weekday == "Monday")
            .unwrap();
        assert_eq!(monday.spend_day_share, Decimal::ONE);
        assert_eq!(forecast.baseline_error, Some(Decimal::ZERO));
        assert_eq!(forecast.model_error, None);
    }

    #[test]
    fn bills_and_planned_payments_stay_out_of_habit_spending() {
        let baseline = run(&history(120), &[], &[], &HashSet::new(), &[]).remove(0);
        let mut rows = history(120);
        let mut bill = tx(date(2026, 9, 3), "999", "bill");
        bill.subscription_id = Some("s1".into());
        rows.push(bill);
        rows.push(tx(date(2026, 9, 4), "500", "plan"));
        rows.push(tx(date(2026, 9, 5), "777", "fixed"));
        let mut refund = tx(date(2026, 9, 6), "10", "refund");
        refund.kind = Kind::Refund;
        rows.push(refund);
        let mut paid = planned(
            "p1",
            "Dentist",
            "2026-09-27",
            PlanStatus::Paid,
            Direction::Outflow,
        );
        paid.transaction_id = Some("plan".into());
        let fixed = HashSet::from(["fixed".to_owned()]);
        let forecast = run(&rows, &[paid], &[], &fixed, &[]).remove(0);
        assert_eq!(
            forecast.habit_spent_this_month,
            baseline.habit_spent_this_month - Decimal::from(10)
        );
        assert_eq!(forecast.planned_paid_this_month, Decimal::from(1499));
    }

    #[test]
    fn recurring_debits_do_not_inflate_next_months_habits() {
        let baseline = run(&history(120), &[], &[], &HashSet::new(), &[]).remove(0);
        let mut rows = history(120);
        let mut card = tx(date(2026, 9, 12), "613", "card");
        card.description = Some("SEPA Incasso algemeen doorlopend Credit card".into());
        rows.push(card);
        let forecast = run(&rows, &[], &[], &HashSet::new(), &[]).remove(0);
        assert_eq!(forecast.months[1].habit, baseline.months[1].habit);
    }

    #[test]
    fn everyday_inputs_drop_large_purchases() {
        let day = date(2026, 9, 1);
        let row = |amount: f64| HabitOutflow {
            day,
            amount,
            category: Some("Food".into()),
            owners: Vec::new(),
        };
        let rows = [row(10.0), row(150.0), row(400.0), row(-5.0)];
        let (kept, summary) = everyday_inputs(&rows, day, &[]);
        // The limit is exclusive and a refund stays in.
        let amounts: Vec<f64> = kept.iter().map(|r| r.amount).collect();
        assert_eq!(amounts, [10.0, 150.0, -5.0]);
        assert_eq!(
            (summary.large_count, summary.large_amount),
            (1, Decimal::from(400))
        );
    }

    #[test]
    fn excluded_categories_stay_out_of_the_forecast_only() {
        let baseline = run(&history(120), &[], &[], &HashSet::new(), &[]).remove(0);
        let mut rows = history(120);
        let today = now().date();
        rows.extend((1..120).step_by(7).map(|back| {
            let mut t = tx(add_days(today, -back), "90", &format!("f{back}"));
            t.category = Some("Flights".into());
            t
        }));
        let forecast = run(&rows, &[], &[], &HashSet::new(), &[" flights".into()]).remove(0);
        assert_eq!(forecast.months[1].habit, baseline.months[1].habit);
        // Spent so far keeps every habit outflow.
        assert!(forecast.habit_spent_this_month > baseline.habit_spent_this_month);
        let excluded = forecast.excluded.unwrap();
        assert_eq!(excluded.categories.len(), 1);
        assert_eq!(excluded.categories[0].category, "Flights");
        assert_eq!(excluded.categories[0].count, 17);
    }

    /// Seasonal-naive model with a two-week period; records its calls.
    #[derive(Debug)]
    struct Fortnightly {
        covariates: bool,
        fail: bool,
        calls: std::cell::RefCell<Vec<(usize, usize, bool)>>,
    }

    impl Fortnightly {
        fn new(covariates: bool, fail: bool) -> Self {
            Self {
                covariates,
                fail,
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl SeriesForecaster for Fortnightly {
        fn name(&self) -> &str {
            "fortnightly"
        }

        fn supports_covariates(&self) -> bool {
            self.covariates
        }

        fn quantiles(
            &self,
            contexts: &[Vec<f64>],
            horizon: usize,
            covariates: Option<&[Vec<f64>]>,
        ) -> Result<Vec<Vec<[f64; 3]>>, String> {
            self.calls
                .borrow_mut()
                .push((contexts.len(), horizon, covariates.is_some()));
            if let Some(covariates) = covariates {
                assert!(
                    contexts
                        .iter()
                        .zip(covariates)
                        .all(|(c, v)| v.len() == c.len() + horizon)
                );
            }
            if self.fail {
                return Err("no model".into());
            }
            Ok(contexts
                .iter()
                .map(|c| {
                    (0..horizon)
                        .map(|step| {
                            let v = c[c.len() - 14 + step % 14];
                            [v, v, v]
                        })
                        .collect()
                })
                .collect())
        }
    }

    /// Whole weeks (counted back from today) alternate between 30 and 10 a day.
    fn alternating(days: i64) -> Vec<Tx> {
        let today = now().date();
        (1..=days)
            .map(|back| {
                let amount = if (back - 1) / 7 % 2 == 0 { "30" } else { "10" };
                tx(add_days(today, -back), amount, &format!("t{back}"))
            })
            .collect()
    }

    fn run_model(transactions: &[Tx], model: &dyn SeriesForecaster) -> DailySpendingForecast {
        let now = now();
        let plans = forecast_plans(&[], date(2026, 9, 1), &[Currency::eur()]);
        daily_spending_forecasts(&DailyInputs {
            transactions,
            payments: &[],
            subscriptions: &[],
            fixed_ids: &HashSet::new(),
            now: &now,
            current_month: date(2026, 9, 1),
            month_start: date(2026, 9, 1),
            budget_plans: &plans,
            mappings: &[],
            excluded_categories: &[],
            forecaster: Some(model),
        })
        .remove(0)
    }

    #[test]
    fn a_more_accurate_model_wins_the_backtest() {
        let model = Fortnightly::new(false, false);
        let forecast = run_model(&alternating(140), &model);
        assert_eq!(forecast.method, "timesfm");
        assert_eq!(forecast.model, "fortnightly");
        assert_eq!(forecast.model_error, Some(Decimal::ZERO));
        assert_eq!(forecast.baseline_error, Some(Decimal::from(70)));
        assert_eq!(forecast.planned_model_error, None);
        assert_eq!(forecast.backtest_windows, 8);
        // Last week cost 30 a day, so the coming one is a 10-a-day week.
        assert_eq!(forecast.next_week_habit, Decimal::from(70));
        // One backtest call for all windows, the live forecast, then the
        // budget groups' own series with the same model.
        let calls = model.calls.borrow();
        let groups = forecast
            .months
            .iter()
            .map(|m| m.groups.len())
            .sum::<usize>();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], (8, 7, false));
        assert_eq!(calls[1].0, 1);
        assert_eq!(calls[2], (groups, calls[1].1, false));
    }

    #[test]
    fn planned_payments_win_ties() {
        let model = Fortnightly::new(true, false);
        let forecast = run_model(&alternating(140), &model);
        assert_eq!(forecast.method, "timesfm_planned");
        assert_eq!(forecast.model_error, forecast.planned_model_error);
        let calls = model.calls.borrow();
        assert_eq!(
            calls.iter().map(|c| c.2).collect::<Vec<_>>(),
            [false, true, false, true, false]
        );
    }

    #[test]
    fn a_failing_model_leaves_the_baseline() {
        let model = Fortnightly::new(true, true);
        let forecast = run_model(&alternating(140), &model);
        assert_eq!(forecast.method, "weekday_baseline");
        assert_eq!(forecast.model, "weekday-baseline");
        assert_eq!(forecast.model_error, None);
        // The backtest failed, so neither the live forecast nor the groups ask.
        assert_eq!(model.calls.borrow().len(), 1);
    }

    #[test]
    fn without_backtest_scores_the_model_is_preferred() {
        assert_eq!(choose(&[], &[BASELINE]), BASELINE);
        assert_eq!(choose(&[], &[BASELINE, TIMESFM]), TIMESFM);
        assert_eq!(
            choose(&[], &[BASELINE, TIMESFM, TIMESFM_PLANNED]),
            TIMESFM_PLANNED
        );
        // Equal errors: the first scored wins, unless planned payments tie.
        assert_eq!(choose(&[(BASELINE, 1.0), (TIMESFM, 1.0)], &[]), BASELINE);
        assert_eq!(
            choose(
                &[(BASELINE, 1.0), (TIMESFM, 2.0), (TIMESFM_PLANNED, 1.0)],
                &[]
            ),
            TIMESFM_PLANNED
        );
    }

    #[test]
    fn short_history_is_not_forecast() {
        assert!(run(&history(20), &[], &[], &HashSet::new(), &[]).is_empty());
    }

    #[test]
    fn forecast_plans_carry_the_previous_month_forward() {
        let mut september = default_budget(date(2026, 9, 1), &Currency::eur());
        september.groups.truncate(1);
        let plans = forecast_plans(&[september], date(2026, 9, 1), &[Currency::eur()]);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[1].month, "2026-10-01");
        assert_eq!(plans[1].groups.len(), 1);
    }
}
