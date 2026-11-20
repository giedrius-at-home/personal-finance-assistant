//! Three-calendar-month cash outlook, behavior evidence and three-day
//! warnings (Python `savings_forecast.py` and `behavior_model.py`).
//!
//! The Python service tried a CatBoost weekly model and kept it only when it
//! beat the weighted weekday baseline on held-out months. This port tries
//! TimesFM there instead (`timesfm`, through the app's [`SeriesForecaster`])
//! with Python's selection unchanged: 28-day blocks held out from
//! max(112, n − 84) days on, the model kept only when its total error is
//! below 0.95 × the baseline's, the same 90-day coverage checks (a model
//! candidate per check, chosen on the 28 days before it) and the same
//! block-bootstrap scaling to the chosen point forecast. TimesFM forecasts
//! the planned and unplanned daily series directly (daily means, never
//! negative), where CatBoost predicted weekly totals spread over the
//! baseline's weekday shape. Without a model (or when it fails) the
//! baseline path runs exactly as before (`weighted_weekday_baseline`, no
//! model error). The simulation reproduces NumPy's float arithmetic and its
//! seeded PCG64 draws, so the bands match Python's.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{BudgetGroup, BudgetPlan, Direction, GroupMapping};
use finance_types::report::Currency;
use finance_types::report::outlook::{
    BehaviorDeduction, ForecastMonth, SavingsOutlook, ThreeDaySignal, UnplannedMonth,
};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::subscriptions::SubscriptionReport;
use jiff::civil::Date;
use jiff::{Timestamp, ToSpan, Zoned};
use rust_decimal::Decimal;

use crate::budget::{budget_owner, plan_for};
use crate::daily::SeriesForecaster;
use crate::groups::group_index;
use crate::model::{Account, Kind, Tx};
use crate::period::{PeriodResolver, shift_month};

/// Simulated futures per outlook.
const SIMULATIONS: usize = 600;
/// Bank data older than this (36 hours) pauses behavioral judgments.
const FRESH_SECONDS: f64 = 129_600.0;

/// Everything the outlook reads. `now` is in the owner's time zone.
#[derive(Clone, Copy, Debug)]
pub struct OutlookInputs<'a> {
    pub transactions: &'a [Tx],
    pub accounts: &'a [Account],
    /// Every saved monthly budget plan.
    pub plans: &'a [BudgetPlan],
    pub mappings: &'a [GroupMapping],
    pub payments: &'a [PlannedPaymentView],
    /// Calendar-month bill reports for this month and the three after it.
    pub subscriptions: &'a [SubscriptionReport],
    pub fixed_ids: &'a HashSet<String>,
    pub payday_day: Option<u8>,
    pub now: &'a Zoned,
    pub data_fresh: bool,
    pub current_month: Date,
    /// Decides which budget month a refund belongs to.
    pub resolver: &'a PeriodResolver,
    /// The model tried against the weekday baseline (TimesFM); `None` keeps
    /// the baseline.
    pub forecaster: Option<&'a dyn SeriesForecaster>,
}

/// Whether the last bank refresh succeeded within 36 hours; stale data pauses
/// behavior scores and alerts.
pub fn data_fresh(sync_health: &(Option<Timestamp>, Option<Timestamp>, bool), now: &Zoned) -> bool {
    let (last_success, _, failed) = sync_health;
    last_success.is_some_and(|at| {
        !failed && now.timestamp().duration_since(at).as_secs_f64() <= FRESH_SECONDS
    })
}

/// Saved plans by (month, currency).
pub(crate) fn plans_by_month(plans: &[BudgetPlan]) -> HashMap<(Date, Currency), &BudgetPlan> {
    plans
        .iter()
        .filter_map(|p| Some(((p.month.parse().ok()?, p.currency.clone()), p)))
        .collect()
}

pub(crate) fn add_days(day: Date, days: i64) -> Date {
    day.saturating_add(days.days())
}

pub(crate) fn days_between(from: Date, to: Date) -> i64 {
    i64::from((to - from).get_days())
}

/// Python's `Decimal(str(float(value))).quantize(Decimal("0.01"))`.
fn money(value: f64) -> Decimal {
    let mut cents = np::repr_decimal(value).round_dp(2);
    cents.rescale(2);
    cents
}

fn money_dec(value: Decimal) -> Decimal {
    money(np::to_f64(value))
}

/// The savings outlook per currency of the included cash accounts.
pub fn savings_outlooks(input: &OutlookInputs) -> Vec<SavingsOutlook> {
    let now = input.now;
    let today = now.date();
    let this_month = input.current_month;
    let calendar_month = today.first_of_month();
    let end_month = shift_month(calendar_month, 3);
    let end = end_month
        .with()
        .day(today.day().min(end_month.days_in_month()))
        .build()
        .unwrap_or(end_month);
    let horizon = usize::try_from(days_between(today, end)).unwrap_or(0);
    let group_map = group_index(input.mappings);
    let source_group = |category: Option<&str>| -> Option<&str> {
        group_map.get(category.unwrap_or("")).copied().flatten()
    };
    let paid = |p: &&PlannedPaymentView| p.status == PlanStatus::Paid;
    let reimbursement_origins: HashMap<&str, &str> = input
        .payments
        .iter()
        .filter(paid)
        .filter_map(|p| {
            Some((
                p.transaction_id.as_deref()?,
                p.reimbursement_for.as_deref()?,
            ))
        })
        .collect();
    let saved = plans_by_month(input.plans);
    let mut explicit_ids: HashSet<&str> = input
        .payments
        .iter()
        .filter(paid)
        .filter(|p| p.direction == Direction::Outflow)
        .filter_map(|p| p.transaction_id.as_deref())
        .collect();
    let reimbursing: Vec<&str> = input
        .payments
        .iter()
        .filter(paid)
        .filter(|p| {
            p.reimbursement_for
                .as_deref()
                .is_some_and(|r| explicit_ids.contains(r))
        })
        .filter_map(|p| p.transaction_id.as_deref())
        .collect();
    explicit_ids.extend(reimbursing);
    let mut fixed_ids: HashSet<&str> = input.fixed_ids.iter().map(String::as_str).collect();
    fixed_ids.extend(
        input
            .payments
            .iter()
            .filter(paid)
            .filter(|p| {
                p.reimbursement_for
                    .as_deref()
                    .is_some_and(|r| input.fixed_ids.contains(r))
            })
            .filter_map(|p| p.transaction_id.as_deref()),
    );
    let mut currencies: Vec<&Currency> = input
        .accounts
        .iter()
        .filter(|a| a.include_net_worth)
        .map(Account::currency)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    currencies.sort_by(|a, b| a.code.cmp(&b.code));

    let mut results = Vec::new();
    for currency in currencies {
        let fallback = plan_for(input.plans, this_month, currency);
        let current = saved
            .get(&(this_month, currency.clone()))
            .copied()
            .unwrap_or(&fallback);
        let plan_for = |month: Date| -> &[BudgetGroup] {
            &saved
                .get(&(month, currency.clone()))
                .copied()
                .unwrap_or(current)
                .groups
        };

        let evidence: Vec<&Tx> = input
            .transactions
            .iter()
            .filter(|t| {
                t.currency == *currency
                    && t.occurred_at.timestamp() <= now.timestamp()
                    && matches!(t.kind, Kind::Outflow | Kind::Refund)
            })
            .collect();
        let earliest = evidence
            .iter()
            .map(|t| t.occurred_at.date())
            .min()
            .unwrap_or(today);
        let start = earliest.max(add_days(today, -730));
        let days = usize::try_from(days_between(start, today)).unwrap_or(0);
        let mut values = vec![[0.0_f64; 2]; days];
        let mut daily: HashMap<Date, [Decimal; 2]> = HashMap::new();
        let mut counts: HashMap<Date, usize> = HashMap::new();
        let mut refunds = Decimal::ZERO;
        for t in &evidence {
            let on = t.occurred_at.date();
            let amount = if t.kind == Kind::Refund {
                -t.amount
            } else {
                t.amount
            };
            if t.kind == Kind::Refund && input.resolver.month_of(t) == this_month {
                refunds += t.amount;
            }
            let id = t.transaction_id.as_str();
            if fixed_ids.contains(id) || explicit_ids.contains(id) {
                continue;
            }
            let owner = budget_owner(
                plan_for(on.first_of_month()),
                t.category.as_deref(),
                source_group(t.category.as_deref()),
                Some(reimbursement_origins.get(id).copied().unwrap_or(id)),
            );
            let group = usize::from(owner.is_none());
            daily.entry(on).or_default()[group] += amount;
            if group == 1 && t.kind == Kind::Outflow {
                *counts.entry(on).or_default() += 1;
            }
            if start <= on && on < today {
                let index = usize::try_from(days_between(start, on)).unwrap_or(0);
                values[index][group] += np::to_f64(amount);
            }
        }
        let model = fit_behavior(&values, start, horizon + 1, input.forecaster);
        // Day zero of the simulation is today, which the balance already holds.
        let paths: Vec<&[[f64; 2]]> = model.paths.iter().map(|p| &p[1..]).collect();
        let balance: Decimal = input
            .accounts
            .iter()
            .filter(|a| a.include_net_worth && a.currency() == currency)
            .map(|a| a.balance.amount)
            .sum();
        let balance_f = np::to_f64(balance);

        let mut income = vec![0.0_f64; horizon];
        let mut obligations = vec![0.0_f64; horizon];
        let mut planned_variable = vec![0.0_f64; horizon];
        let mut scheduled_by_month: HashMap<Date, Decimal> = HashMap::new();
        let mut assumed_months = Vec::new();
        let slot = |due: Date| usize::try_from(days_between(today, due) - 1).unwrap_or(0);
        for p in input.payments {
            if p.amount.currency != *currency || p.status == PlanStatus::Paid {
                continue;
            }
            let Ok(due_on) = p.due_on.parse::<Date>() else {
                continue;
            };
            // An overdue expected deposit is not silently assumed to arrive tomorrow.
            if p.direction == Direction::Inflow
                && (due_on <= today || p.status == PlanStatus::NeedsReview)
            {
                continue;
            }
            // Like the monthly plan: a payment left unpaid in an earlier
            // month is only still owed while it is a pending reservation.
            if due_on < today.first_of_month() && !p.pending {
                continue;
            }
            let due = add_days(today, 1).max(due_on);
            if due > end {
                continue;
            }
            let amount = np::to_f64(p.amount.amount);
            if p.direction == Direction::Inflow {
                income[slot(due)] += amount;
            } else {
                obligations[slot(due)] += amount;
                let owner = budget_owner(
                    plan_for(due.first_of_month()),
                    Some(&p.match_category),
                    source_group(Some(&p.match_category)),
                    None,
                );
                if owner.is_some() {
                    *scheduled_by_month.entry(due.first_of_month()).or_default() += p.amount.amount;
                }
            }
        }
        let mut seen_bills: HashSet<(&str, &str)> = HashSet::new();
        for bill in input
            .subscriptions
            .iter()
            .flat_map(|r| &r.scheduled_payments)
        {
            if bill.amount.currency != *currency || bill.paid_on.is_some() {
                continue;
            }
            if !seen_bills.insert((&bill.bill_id, &bill.due_on)) {
                continue;
            }
            let Ok(due_on) = bill.due_on.parse::<Date>() else {
                continue;
            };
            let due = add_days(today, 1).max(due_on);
            if due <= end {
                obligations[slot(due)] += np::to_f64(bill.amount.amount);
            }
        }
        for offset in 0..4 {
            let month = shift_month(calendar_month, offset);
            let groups = plan_for(month);
            if !saved.contains_key(&(month, currency.clone())) {
                assumed_months.push(month.to_string());
            }
            let mut spent: Decimal = daily
                .iter()
                .filter(|(day, _)| day.first_of_month() == month && **day <= today)
                .map(|(_, amounts)| amounts[0])
                .sum();
            // Explicit payments assigned to a group already consumed its allowance.
            spent += evidence
                .iter()
                .filter(|t| {
                    explicit_ids.contains(t.transaction_id.as_str())
                        && t.effective_on().first_of_month() == month
                        && budget_owner(
                            groups,
                            t.category.as_deref(),
                            source_group(t.category.as_deref()),
                            Some(&t.transaction_id),
                        )
                        .is_some()
                })
                .map(|t| t.amount)
                .sum::<Decimal>();
            let planned: Decimal = groups.iter().map(|g| g.amount).sum();
            let allowance =
                (planned - spent - scheduled_by_month.get(&month).copied().unwrap_or_default())
                    .max(Decimal::ZERO);
            let month_days = i64::from(month.days_in_month());
            let remaining_days = if month == calendar_month {
                month_days - i64::from(today.day())
            } else {
                month_days
            };
            if remaining_days != 0 {
                let share = np::to_f64(allowance) / remaining_days as f64;
                for (index, value) in planned_variable.iter_mut().enumerate() {
                    if add_days(today, index as i64 + 1).first_of_month() == month {
                        *value = share;
                    }
                }
            }
        }
        let net_fixed: Vec<f64> = income
            .iter()
            .zip(&obligations)
            .map(|(i, o)| i - o)
            .collect();
        let planned_curve: Vec<f64> =
            np::cumsum(net_fixed.iter().zip(&planned_variable).map(|(n, v)| n - v))
                .into_iter()
                .map(|c| balance_f + c)
                .collect();
        let curves: Vec<Vec<f64>> = paths
            .iter()
            .map(|path| {
                np::cumsum(
                    net_fixed
                        .iter()
                        .zip(path.iter())
                        .map(|(n, day)| n - (day[0] + day[1])),
                )
                .into_iter()
                .map(|c| balance_f + c)
                .collect()
            })
            .collect();
        let column = |i: usize| -> Vec<f64> { curves.iter().map(|c| c[i]).collect() };
        let expected: Vec<f64> = (0..horizon)
            .map(|i| np::seq_sum(curves.iter().map(|c| c[i])) / curves.len() as f64)
            .collect();
        let (lower, upper): (Vec<f64>, Vec<f64>) = (0..horizon)
            .map(|i| {
                let col = column(i);
                (np::quantile(&col, 0.1), np::quantile(&col, 0.9))
            })
            .unzip();
        let checkpoints: Vec<usize> = (0..horizon)
            .filter(|&i| i + 1 == horizon || add_days(today, i as i64 + 2).day() == 1)
            .collect();
        let months = checkpoints
            .iter()
            .map(|&i| ForecastMonth {
                through: add_days(today, i as i64 + 1).to_string(),
                planned_balance: money(planned_curve[i]),
                expected_balance: money(expected[i]),
                lower_balance: money(lower[i]),
                upper_balance: money(upper[i]),
            })
            .collect();

        let in_month = |day: &Date, month: Date| day.first_of_month() == month;
        let history: Vec<UnplannedMonth> = (-5..=0)
            .map(|offset| {
                let month = shift_month(calendar_month, offset);
                let unplanned: Decimal = daily
                    .iter()
                    .filter(|(d, _)| in_month(d, month))
                    .map(|(_, v)| v[1])
                    .sum();
                let total: Decimal = daily
                    .iter()
                    .filter(|(d, _)| in_month(d, month))
                    .map(|(_, v)| v[0] + v[1])
                    .sum();
                let same_elapsed: Decimal = daily
                    .iter()
                    .filter(|(d, _)| in_month(d, month) && d.day() <= today.day())
                    .map(|(_, v)| v[1])
                    .sum();
                let share = if total > Decimal::ZERO {
                    let ratio = np::to_f64(unplanned.max(Decimal::ZERO) / total);
                    np::round_decimal(ratio, 4)
                } else {
                    Decimal::ZERO
                };
                UnplannedMonth {
                    month: month.to_string(),
                    net_spending: money_dec(unplanned),
                    same_elapsed_spending: money_dec(same_elapsed),
                    share,
                    purchase_count: counts
                        .iter()
                        .filter(|(d, _)| in_month(d, month))
                        .map(|(_, n)| n)
                        .sum(),
                    reconstructed: !saved.contains_key(&(month, currency.clone())),
                }
            })
            .collect();

        let window = |end_day: Date, group: Option<usize>| -> Decimal {
            (0..3)
                .map(|i| {
                    let v = daily
                        .get(&add_days(end_day, -i))
                        .copied()
                        .unwrap_or_default();
                    group.map_or(v[0] + v[1], |g| v[g])
                })
                .sum()
        };
        let recent = window(add_days(today, -1), None);
        let previous = window(add_days(today, -4), None);
        let unplanned = window(add_days(today, -1), Some(1));
        let previous_unplanned = window(add_days(today, -4), Some(1));
        let weeks = 1..(days / 7).min(13);
        let historical: Vec<f64> = weeks
            .clone()
            .map(|w| np::to_f64(window(add_days(today, -1 - 7 * w as i64), None)))
            .collect();
        let historical_unplanned: Vec<f64> = weeks
            .map(|w| np::to_f64(window(add_days(today, -1 - 7 * w as i64), Some(1))))
            .collect();
        let upper_normal = if historical.is_empty() {
            recent
        } else {
            np::repr_decimal(np::quantile(&historical, 0.9))
        };
        let unplanned_upper = if historical_unplanned.is_empty() {
            unplanned
        } else {
            np::repr_decimal(np::quantile(&historical_unplanned, 0.9))
        };
        let allowance3: Decimal = (1..=3)
            .map(|i| {
                let day = add_days(today, -i);
                let planned: Decimal = plan_for(day.first_of_month())
                    .iter()
                    .map(|g| g.amount)
                    .sum();
                planned / Decimal::from(day.days_in_month())
            })
            .sum();
        let fifty = Decimal::from(50);
        let half_again = Decimal::new(15, 1);
        let speed = recent > allowance3 * half_again && recent - allowance3 >= fifty;
        let acceleration = recent - previous >= fifty
            && (previous <= Decimal::ZERO || recent >= previous * half_again);
        let unusual = historical_unplanned.len() >= 4
            && unplanned > unplanned_upper
            && unplanned - previous_unplanned >= fifty;
        let signal = ThreeDaySignal {
            start: add_days(today, -3).to_string(),
            end: add_days(today, -1).to_string(),
            previous_start: add_days(today, -6).to_string(),
            previous_end: add_days(today, -4).to_string(),
            spending: money_dec(recent),
            previous_spending: money_dec(previous),
            unplanned: money_dec(unplanned),
            previous_unplanned: money_dec(previous_unplanned),
            allowance: money_dec(allowance3),
            historical_upper: money_dec(upper_normal),
            excessive_speed: speed,
            accelerating: acceleration,
            unusual_unplanned: unusual,
            alert: input.data_fresh && days >= 28 && (speed || acceleration || unusual),
        };
        let mut deductions = Vec::new();
        let deduct = |reason: &str, points: u32| BehaviorDeduction {
            reason: reason.into(),
            points,
        };
        if speed {
            deductions.push(deduct("Three-day spending materially exceeds the plan", 20));
        }
        if acceleration {
            deductions.push(deduct(
                "Spending rose at least 50% and 50 currency units over the preceding three days",
                15,
            ));
        }
        if unusual {
            deductions.push(deduct(
                "Unplanned purchases exceed your usual comparable three-day windows",
                15,
            ));
        }
        let month_net = history.last().map_or(Decimal::ZERO, |h| h.net_spending);
        if month_net >= fifty {
            let steps = u32::try_from((month_net / fifty).trunc().mantissa()).unwrap_or(u32::MAX);
            deductions.push(deduct(
                "At least 50 currency units of net unplanned spending this month",
                steps.saturating_mul(5).min(30),
            ));
        }
        // Each checkpoint must preserve the plan's cumulative saving trajectory.
        let successes = curves
            .iter()
            .filter(|c| checkpoints.iter().all(|&i| c[i] >= planned_curve[i]))
            .count();
        let reliable = input.data_fresh
            && model.horizon_windows >= 6
            && model.coverage.is_some_and(|c| c >= 0.66)
            && days >= 652;
        let mut assumptions: Vec<String> = ASSUMPTIONS.iter().map(|a| (*a).to_owned()).collect();
        if !assumed_months.is_empty() {
            assumptions.push(format!(
                "Current budget carried forward where no plan exists: {}",
                assumed_months.join(", ")
            ));
        }
        if !reliable {
            assumptions.push(
                "Limited evidence: a calibrated three-month success probability is not yet available."
                    .into(),
            );
        }
        if !input.data_fresh {
            assumptions.push(
                "Bank data is stale or its last refresh failed; behavioral judgments and email alerts are paused."
                    .into(),
            );
        }
        if currency.code != "EUR" {
            assumptions.push(
                "Alert materiality is 50 units of this currency; currencies are never combined."
                    .into(),
            );
        }
        let last = |series: &[f64]| series.last().copied().unwrap_or(balance_f);
        let unplanned_totals: Vec<f64> = paths
            .iter()
            .map(|p| np::sum_iter(p.iter().map(|d| d[1])))
            .collect();
        // Days left in the running budget month after today; path day i is
        // today + 1 + i.
        let next_start = input
            .resolver
            .period_start_date(shift_month(input.current_month, 1));
        let month_days = usize::try_from(days_between(today, next_start) - 1).unwrap_or(0);
        let month_unplanned: Vec<f64> = paths
            .iter()
            .map(|p| np::sum_iter(p.iter().take(month_days).map(|d| d[1].max(0.0))))
            .collect();
        results.push(SavingsOutlook {
            currency: currency.clone(),
            as_of: today.to_string(),
            through: end.to_string(),
            starting_balance: money_dec(balance),
            planned_savings: money(last(&planned_curve) - balance_f),
            expected_savings: money(last(&expected) - balance_f),
            planned_final: money(last(&planned_curve)),
            expected_final: money(last(&expected)),
            lower_final: money(last(&lower)),
            upper_final: money(last(&upper)),
            expected_unplanned: money(np::mean(&unplanned_totals)),
            expected_unplanned_month: money(np::mean(&month_unplanned)),
            expected_income: money(np::sum(&income)),
            received_refunds: money_dec(refunds),
            on_track_probability: reliable
                .then(|| np::round_decimal(successes as f64 / curves.len() as f64, 2)),
            behavior_score: (input.data_fresh && days >= 28)
                .then(|| 100_u32.saturating_sub(deductions.iter().map(|d| d.points).sum::<u32>())),
            deductions: if input.data_fresh {
                deductions
            } else {
                Vec::new()
            },
            signal,
            months,
            unplanned_history: history,
            method: model.method.into(),
            training_days: days,
            training_seconds: money(model.training_seconds),
            validation_windows: model.validation_windows,
            horizon_validation_windows: model.horizon_windows,
            baseline_error: model.baseline_error.map(money),
            model_error: model.model_error.map(money),
            interval_coverage: model.coverage.map(|c| np::round_decimal(c, 2)),
            assumptions,
            data_fresh: input.data_fresh,
        });
    }
    results
}

const ASSUMPTIONS: [&str; 8] = [
    "Uncategorised or unreconciled outflows remain included. Link reimbursements and resolve transfers to improve the outlook.",
    "Intervals come from resampled historical weeks; they are estimates, not guaranteed bounds.",
    "Final balance uses included Firefly cash accounts; transfers between them do not create savings.",
    "Plan scenario has no additional unplanned purchases. Current behavior includes historical unplanned spending and recorded refunds.",
    "Salary and entered future income are expectations; received deposits are already in the starting balance. Overdue deposits are excluded.",
    "Future bills use the recorded schedule; unknown future income, bills or reimbursements are not invented.",
    "Behavior score is a transparent rule-based coaching score, not a probability. Recent deductions recover as the rolling window and monthly spending improve.",
    "Unplanned history uses each saved monthly plan, otherwise the current plan; reconstructed history is labelled.",
];

/// Simulated planned/unplanned daily spending and how well the method did on
/// held-out history (Python `behavior_model.ModelResult`).
struct Behavior {
    method: &'static str,
    /// Simulation × future day × (planned, unplanned).
    paths: Vec<Vec<[f64; 2]>>,
    baseline_error: Option<f64>,
    model_error: Option<f64>,
    validation_windows: usize,
    horizon_windows: usize,
    coverage: Option<f64>,
    training_seconds: f64,
}

type Day = [f64; 2];

fn flat_sum(rows: &[Day]) -> f64 {
    let flat: Vec<f64> = rows.iter().flatten().copied().collect();
    np::sum(&flat)
}

/// Weighted same-weekday averages of the last twelve weeks, for the seven
/// days from `day` on. All days, zero-spend ones too, count.
fn weekday_week(values: &[Day], start: Date, day: Date) -> [Day; 7] {
    let recent = &values[values.len().saturating_sub(84)..];
    let weights: Vec<f64> = np::linspace(-2.0, 0.0, recent.len())
        .into_iter()
        .map(f64::exp)
        .collect();
    let offset = values.len() - recent.len();
    let weekdays: Vec<jiff::civil::Weekday> = (0..recent.len())
        .map(|i| add_days(start, (offset + i) as i64).weekday())
        .collect();
    let mut result = [[0.0; 2]; 7];
    for (k, row) in result.iter_mut().enumerate() {
        let target = add_days(day, k as i64).weekday();
        let rows: Vec<usize> = (0..recent.len())
            .filter(|&i| weekdays[i] == target)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let scale = np::sum(&rows.iter().map(|&i| weights[i]).collect::<Vec<_>>());
        for (g, value) in row.iter_mut().enumerate() {
            let weighted = np::seq_sum(rows.iter().map(|&i| recent[i][g] * weights[i]));
            *value = (weighted / scale).max(0.0);
        }
    }
    result
}

fn predict(values: &[Day], start: Date, horizon: usize) -> Vec<Day> {
    let mut history = values.to_vec();
    let mut output = Vec::with_capacity(horizon + 7);
    for _ in (0..horizon).step_by(7) {
        let day = add_days(start, history.len() as i64);
        let week = weekday_week(&history, start, day);
        output.extend_from_slice(&week);
        history.extend_from_slice(&week);
    }
    output.truncate(horizon);
    output
}

/// Python's `range(start, stop, step)` over signed bounds.
fn py_range(start: i64, stop: i64, step: usize) -> impl Iterator<Item = usize> {
    (start..stop.max(start))
        .step_by(step)
        .map(|i| usize::try_from(i).unwrap_or(0))
}

/// The model's daily forecasts (means, never negative) of both series,
/// `horizon` days after each context `values[..end]`, in one call.
fn model_forecasts(
    model: &dyn SeriesForecaster,
    values: &[Day],
    requests: &[(usize, usize)],
) -> Option<Vec<Vec<Day>>> {
    let horizon = requests.iter().map(|r| r.1).max()?;
    let contexts: Vec<Vec<f64>> = requests
        .iter()
        .flat_map(|&(end, _)| (0..2).map(move |g| values[..end].iter().map(|d| d[g]).collect()))
        .collect();
    let rows = model.quantiles(&contexts, horizon, None).ok()?;
    if rows.len() != contexts.len() {
        return None;
    }
    Some(
        requests
            .iter()
            .enumerate()
            .map(|(i, &(_, h))| {
                (0..h)
                    .map(|day| {
                        [0, 1].map(|g| rows[2 * i + g].get(day).map_or(0.0, |r| r[1].max(0.0)))
                    })
                    .collect()
            })
            .collect(),
    )
}

fn fit_behavior(
    values: &[Day],
    start: Date,
    horizon: usize,
    forecaster: Option<&dyn SeriesForecaster>,
) -> Behavior {
    let began = std::time::Instant::now();
    let n = values.len();
    let signed = n as i64;
    if n == 0 {
        return Behavior {
            method: "insufficient_history",
            paths: vec![vec![[0.0; 2]; horizon]; SIMULATIONS],
            baseline_error: None,
            model_error: None,
            validation_windows: 0,
            horizon_windows: 0,
            coverage: None,
            training_seconds: began.elapsed().as_secs_f64(),
        };
    }
    // Entire validation blocks are held out; no shuffled day-level split.
    let origins: Vec<usize> = py_range((signed - 84).max(112), signed - 27, 28).collect();
    let baseline_errors: Vec<f64> = origins
        .iter()
        .map(|&origin| {
            let actual = flat_sum(&values[origin..origin + 28]);
            (flat_sum(&predict(&values[..origin], start, 28)) - actual).abs()
        })
        .collect();
    let coverage_origins: Vec<usize> = py_range(112, signed - 89, 90).collect();
    // Coverage checks choose their candidate on the 28 days before them.
    let selections: Vec<usize> = coverage_origins
        .iter()
        .filter_map(|o| o.checked_sub(28).filter(|s| *s >= 112))
        .collect();
    // One call for every 28-day block the model is judged on.
    let judged = forecaster.and_then(|m| {
        let requests: Vec<(usize, usize)> = origins
            .iter()
            .chain(&selections)
            .map(|&end| (end, 28))
            .collect();
        if requests.is_empty() {
            return None;
        }
        model_forecasts(m, values, &requests)
    });
    let model_errors: Vec<f64> = judged.as_ref().map_or_else(Vec::new, |f| {
        origins
            .iter()
            .zip(f)
            .map(|(&origin, forecast)| {
                (flat_sum(forecast) - flat_sum(&values[origin..origin + 28])).abs()
            })
            .collect()
    });
    let mut use_model = !model_errors.is_empty()
        && np::py_sum(model_errors.iter().copied())
            < np::py_sum(baseline_errors.iter().copied()) * 0.95;
    // Which coverage checks would have picked the model.
    let candidates: Vec<bool> = coverage_origins
        .iter()
        .map(|&origin| {
            let Some(judged) = &judged else {
                return false;
            };
            let Some(selection) = origin.checked_sub(28).filter(|s| *s >= 112) else {
                return false;
            };
            let Some(index) = selections.iter().position(|s| *s == selection) else {
                return false;
            };
            let training = &values[..origin];
            let actual = flat_sum(&training[selection..]);
            let simple = (flat_sum(&predict(&training[..selection], start, 28)) - actual).abs();
            let candidate = (flat_sum(&judged[origins.len() + index]) - actual).abs();
            candidate < simple * 0.95
        })
        .collect();
    // One more call: the live forecast and the chosen 90-day candidates.
    let mut later: Vec<(usize, usize)> = coverage_origins
        .iter()
        .zip(&candidates)
        .filter(|(_, c)| **c)
        .map(|(&origin, _)| (origin, 90))
        .collect();
    if use_model {
        later.push((n, horizon));
    }
    let mut forecasts = match forecaster {
        Some(m) if !later.is_empty() => model_forecasts(m, values, &later),
        _ => None,
    };
    if forecasts.is_none() {
        // The model failed: the baseline everywhere, as without one.
        use_model = false;
    }
    let point = match forecasts.as_mut().filter(|_| use_model).and_then(Vec::pop) {
        Some(forecast) => forecast,
        None => predict(values, start, horizon),
    };
    let mut candidate_forecasts = forecasts.into_iter().flatten();
    // Joint moving blocks retain zero days, bursty purchases and group correlation.
    let blocks: Vec<&[Day]> = py_range((signed - 182).max(0), signed - 6, 7)
        .map(|i| &values[i..i + 7])
        .collect();
    let paths = if blocks.len() < 2 {
        vec![point.clone(); SIMULATIONS]
    } else {
        let mut rng = np::Pcg64::new(26);
        let per_path = horizon.div_ceil(7);
        let mut paths: Vec<Vec<Day>> = (0..SIMULATIONS)
            .map(|_| {
                let mut path: Vec<Day> = (0..per_path)
                    .flat_map(|_| {
                        blocks[rng.below(blocks.len() as u64) as usize]
                            .iter()
                            .copied()
                    })
                    .collect();
                path.truncate(horizon);
                path
            })
            .collect();
        // Align group expectations to the chosen point forecast, retaining joint variation.
        let count = (blocks.len() * 7) as f64;
        let point_count = point.len() as f64;
        for g in 0..2 {
            let mean = np::seq_sum(blocks.iter().flat_map(|b| b.iter().map(|d| d[g]))) / count;
            let point_mean = np::seq_sum(point.iter().map(|d| d[g])) / point_count;
            let scale = if mean > 0.0 { point_mean / mean } else { 1.0 };
            for path in &mut paths {
                for (day, base) in path.iter_mut().zip(&point) {
                    day[g] = if mean <= 0.0 { base[g] } else { day[g] * scale };
                }
            }
        }
        paths
    };
    // Separate 90-day validation; monthly accuracy is not a 3-month guarantee.
    let coverage_checks: Vec<f64> = coverage_origins
        .iter()
        .zip(&candidates)
        .map(|(&origin, &candidate)| {
            let training = &values[..origin];
            let model_total = if candidate {
                candidate_forecasts.next().map(|f| flat_sum(&f))
            } else {
                None
            };
            let predicted = model_total.unwrap_or_else(|| flat_sum(&predict(training, start, 90)));
            let weeks: Vec<f64> = py_range((origin as i64 - 182).max(0), origin as i64 - 6, 7)
                .map(|i| flat_sum(&training[i..i + 7]))
                .collect();
            let mut rng = np::Pcg64::new(26);
            let mut samples: Vec<f64> = (0..SIMULATIONS)
                .map(|_| {
                    let draw: Vec<f64> = (0..13)
                        .map(|_| weeks[rng.below(weeks.len() as u64) as usize])
                        .collect();
                    np::sum(&draw) * 90.0 / 91.0
                })
                .collect();
            let mean = np::mean(&samples);
            if mean > 0.0 {
                let factor = predicted / mean;
                samples.iter_mut().for_each(|s| *s *= factor);
            }
            let (lower, upper) = (np::quantile(&samples, 0.1), np::quantile(&samples, 0.9));
            let actual = flat_sum(&values[origin..origin + 90]);
            f64::from(u8::from(lower <= actual && actual <= upper))
        })
        .collect();
    Behavior {
        method: if use_model {
            "timesfm"
        } else {
            "weighted_weekday_baseline"
        },
        paths,
        baseline_error: (!baseline_errors.is_empty()).then(|| np::mean(&baseline_errors)),
        model_error: (!model_errors.is_empty()).then(|| np::mean(&model_errors)),
        validation_windows: baseline_errors.len(),
        horizon_windows: coverage_checks.len(),
        coverage: (!coverage_checks.is_empty()).then(|| np::mean(&coverage_checks)),
        training_seconds: began.elapsed().as_secs_f64(),
    }
}

/// NumPy and CPython float arithmetic, reproduced bit for bit so forecasts
/// match the Python service: pairwise sums, linear quantiles, compensated
/// builtin sums, `round`, and the seeded PCG64 generator.
pub(crate) mod np {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    /// `float(Decimal)`: correctly rounded.
    pub fn to_f64(value: Decimal) -> f64 {
        value.to_string().parse().unwrap_or(0.0)
    }

    /// `Decimal(str(value))`: the shortest repr, exactly.
    pub fn repr_decimal(value: f64) -> Decimal {
        Decimal::from_str(&format!("{value}"))
            .or_else(|_| Decimal::from_scientific(&format!("{value:e}")))
            .unwrap_or_default()
    }

    /// `Decimal(str(round(value, digits)))`: rounded half-even on the exact
    /// binary value, as CPython's `round` does.
    pub fn round_decimal(value: f64, digits: usize) -> Decimal {
        Decimal::from_str(&format!("{value:.digits$}"))
            .unwrap_or_default()
            .normalize()
    }

    fn pairwise(a: &[f64]) -> f64 {
        let n = a.len();
        if n < 8 {
            let mut res = 0.0;
            for x in a {
                res += x;
            }
            res
        } else if n <= 128 {
            let mut r = [0.0; 8];
            r.copy_from_slice(&a[..8]);
            let mut i = 8;
            while i < n - n % 8 {
                for j in 0..8 {
                    r[j] += a[i + j];
                }
                i += 8;
            }
            let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            while i < n {
                res += a[i];
                i += 1;
            }
            res
        } else {
            let mut half = n / 2;
            half -= half % 8;
            pairwise(&a[..half]) + pairwise(&a[half..])
        }
    }

    /// `np.sum` of a one-dimensional array.
    pub fn sum(a: &[f64]) -> f64 {
        0.0 + pairwise(a)
    }

    pub fn sum_iter(values: impl Iterator<Item = f64>) -> f64 {
        sum(&values.collect::<Vec<_>>())
    }

    /// `np.mean` of a one-dimensional array.
    pub fn mean(a: &[f64]) -> f64 {
        sum(a) / a.len() as f64
    }

    /// A reduction along an outer axis: plain left-to-right addition.
    pub fn seq_sum(values: impl Iterator<Item = f64>) -> f64 {
        values.fold(0.0, |acc, x| acc + x)
    }

    pub fn cumsum(values: impl Iterator<Item = f64>) -> Vec<f64> {
        let mut acc = 0.0;
        values
            .map(|x| {
                acc += x;
                acc
            })
            .collect()
    }

    /// CPython's builtin `sum` of floats (Neumaier-compensated since 3.12).
    pub fn py_sum(values: impl Iterator<Item = f64>) -> f64 {
        let mut total = 0.0_f64;
        let mut c = 0.0_f64;
        for x in values {
            let t = total + x;
            if total.abs() >= x.abs() {
                c += (total - t) + x;
            } else {
                c += (x - t) + total;
            }
            total = t;
        }
        if c != 0.0 && c.is_finite() {
            total += c;
        }
        total
    }

    /// `np.linspace(start, stop, num)`.
    pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
        if num == 1 {
            return vec![start];
        }
        let step = (stop - start) / (num - 1) as f64;
        let mut out: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
        if let Some(last) = out.last_mut() {
            *last = stop;
        }
        out
    }

    /// `np.quantile(values, q)` with the default linear method.
    pub fn quantile(values: &[f64], q: f64) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let virtual_index = (n - 1) as f64 * q;
        if virtual_index >= (n - 1) as f64 {
            return sorted[n - 1];
        }
        let previous = virtual_index.floor();
        let gamma = virtual_index - previous;
        let below = sorted[previous as usize];
        let above = sorted[previous as usize + 1];
        let diff = above - below;
        if gamma >= 0.5 {
            above - diff * (1.0 - gamma)
        } else {
            below + diff * gamma
        }
    }

    /// NumPy's `default_rng(seed)`: PCG64 seeded through `SeedSequence`.
    pub struct Pcg64 {
        state: u128,
        inc: u128,
        spare: Option<u32>,
    }

    const MULTIPLIER: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

    impl Pcg64 {
        pub fn new(seed: u32) -> Self {
            let words = seed_sequence(seed);
            let word = |i: usize| u64::from(words[2 * i]) | (u64::from(words[2 * i + 1]) << 32);
            let initstate = (u128::from(word(0)) << 64) | u128::from(word(1));
            let initseq = (u128::from(word(2)) << 64) | u128::from(word(3));
            let mut rng = Self {
                state: 0,
                inc: (initseq << 1) | 1,
                spare: None,
            };
            rng.step();
            rng.state = rng.state.wrapping_add(initstate);
            rng.step();
            rng
        }

        fn step(&mut self) {
            self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(self.inc);
        }

        pub fn next_u64(&mut self) -> u64 {
            self.step();
            let high = (self.state >> 64) as u64;
            let low = self.state as u64;
            (high ^ low).rotate_right((self.state >> 122) as u32)
        }

        fn next_u32(&mut self) -> u32 {
            if let Some(spare) = self.spare.take() {
                return spare;
            }
            let next = self.next_u64();
            self.spare = Some((next >> 32) as u32);
            next as u32
        }

        /// `integers(0, bound)` for a bound below 2^32 (Lemire's method).
        pub fn below(&mut self, bound: u64) -> u64 {
            let range = bound.saturating_sub(1);
            if range == 0 {
                return 0;
            }
            let range = u32::try_from(range).unwrap_or(u32::MAX);
            if range == u32::MAX {
                return u64::from(self.next_u32());
            }
            let excl = range + 1;
            let mut m = u64::from(self.next_u32()) * u64::from(excl);
            let mut leftover = m as u32;
            if leftover < excl {
                let threshold = (u32::MAX - range) % excl;
                while leftover < threshold {
                    m = u64::from(self.next_u32()) * u64::from(excl);
                    leftover = m as u32;
                }
            }
            m >> 32
        }
    }

    /// `SeedSequence(seed).generate_state(4, uint64)` as eight 32-bit words.
    fn seed_sequence(seed: u32) -> [u32; 8] {
        const INIT_A: u32 = 0x43b0_d7e5;
        const MULT_A: u32 = 0x931e_8875;
        const INIT_B: u32 = 0x8b51_f9dd;
        const MULT_B: u32 = 0x58f3_8ded;
        const MIX_MULT_L: u32 = 0xca01_f9dd;
        const MIX_MULT_R: u32 = 0x4973_f715;
        let mut hash_const = INIT_A;
        let mut hashmix = |value: u32| {
            let mut value = value ^ hash_const;
            hash_const = hash_const.wrapping_mul(MULT_A);
            value = value.wrapping_mul(hash_const);
            value ^ (value >> 16)
        };
        let mix = |x: u32, y: u32| {
            let result = MIX_MULT_L
                .wrapping_mul(x)
                .wrapping_sub(MIX_MULT_R.wrapping_mul(y));
            result ^ (result >> 16)
        };
        let mut pool = [0_u32; 4];
        for (i, slot) in pool.iter_mut().enumerate() {
            *slot = hashmix(if i == 0 { seed } else { 0 });
        }
        for src in 0..4 {
            for dst in 0..4 {
                if src != dst {
                    let hashed = hashmix(pool[src]);
                    pool[dst] = mix(pool[dst], hashed);
                }
            }
        }
        let mut hash_const = INIT_B;
        let mut out = [0_u32; 8];
        for (i, slot) in out.iter_mut().enumerate() {
            let mut value = pool[i % 4] ^ hash_const;
            hash_const = hash_const.wrapping_mul(MULT_B);
            value = value.wrapping_mul(hash_const);
            *slot = value ^ (value >> 16);
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn pcg64_matches_numpy_default_rng() {
            // np.random.default_rng(26).bit_generator.random_raw(3)
            let mut rng = Pcg64::new(26);
            let raw: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
            assert_eq!(
                raw,
                [
                    9068399419970302404,
                    4323908029266571262,
                    1247143012771080907
                ]
            );
            // default_rng(26).integers(0, 7, size=10), then size=3
            let mut rng = Pcg64::new(26);
            let ints: Vec<u64> = (0..13).map(|_| rng.below(7)).collect();
            assert_eq!(ints, [6, 3, 3, 1, 2, 0, 2, 4, 5, 5, 3, 3, 6]);
        }

        #[test]
        fn rounding_follows_python() {
            assert_eq!(round_decimal(0.125, 2).to_string(), "0.12");
            assert_eq!(round_decimal(0.375, 2).to_string(), "0.38");
            assert_eq!(round_decimal(2.675, 2).to_string(), "2.67");
            assert_eq!(repr_decimal(2.675).round_dp(2).to_string(), "2.68");
            assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.9), 3.7);
        }

        #[test]
        fn compensated_sum_beats_naive_addition() {
            let values = [0.1; 10];
            assert_eq!(py_sum(values.iter().copied()), 1.0);
            assert_ne!(values.iter().fold(0.0, |a, b| a + b), 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::default_budget;
    use finance_types::report::UnitMoney;
    use jiff::civil::date;

    fn now() -> Zoned {
        "2026-09-28T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap()
    }

    fn purchase(id: &str, amount: &str, days_ago: i64, category: Option<&str>, kind: Kind) -> Tx {
        let at = now().saturating_sub(days_ago.days());
        Tx {
            transaction_id: id.into(),
            ledger_id: 1,
            account_id: "bank".into(),
            counterparty_account_id: None,
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: at,
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: category.map(Into::into),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn account() -> Account {
        Account {
            account_id: "firefly:1".into(),
            ledger_id: 1,
            name: "Bank".into(),
            balance: UnitMoney {
                amount: Decimal::from(1000),
                currency: Currency::eur(),
            },
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    fn plan() -> BudgetPlan {
        BudgetPlan {
            month: "2026-09-01".into(),
            currency: Currency::eur(),
            revision: 0,
            groups: vec![BudgetGroup {
                id: "food".into(),
                name: "Food".into(),
                amount: Decimal::from(300),
                source_groups: Vec::new(),
                categories: vec!["Jumbo".into()],
                transaction_ids: Vec::new(),
                account_id: None,
                importance: None,
                floor: None,
            }],
        }
    }

    fn forecast(transactions: &[Tx], fresh: bool) -> SavingsOutlook {
        let now = now();
        let accounts = [account()];
        let plans = [plan()];
        let fixed = HashSet::new();
        let resolver = PeriodResolver::new(&[], HashMap::new(), now.time_zone().clone());
        savings_outlooks(&OutlookInputs {
            transactions,
            accounts: &accounts,
            plans: &plans,
            mappings: &[],
            payments: &[],
            subscriptions: &[],
            fixed_ids: &fixed,
            payday_day: Some(26),
            now: &now,
            data_fresh: fresh,
            current_month: date(2026, 9, 1),
            resolver: &resolver,
            forecaster: None,
        })
        .remove(0)
    }

    #[test]
    fn unplanned_refunds_three_day_windows_and_stale_suppression() {
        let mut rows: Vec<Tx> = (1..10)
            .map(|i| {
                purchase(
                    &format!("old-{i}"),
                    "5",
                    7 * i,
                    Some("Unknown"),
                    Kind::Outflow,
                )
            })
            .collect();
        rows.extend([
            purchase("a", "100", 1, None, Kind::Outflow),
            purchase("b", "100", 2, None, Kind::Outflow),
            purchase("refund", "20", 2, None, Kind::Refund),
            purchase("prior", "10", 4, None, Kind::Outflow),
            purchase("today", "999", 0, None, Kind::Outflow),
        ]);
        let result = forecast(&rows, true);
        // Today is incomplete and excluded.
        assert_eq!(result.signal.spending, Decimal::from(180));
        assert_eq!(result.signal.previous_spending, Decimal::from(10));
        assert_eq!(result.signal.unplanned, Decimal::from(180));
        assert!(result.signal.accelerating && result.signal.alert);
        assert!(result.expected_unplanned > Decimal::ZERO);
        assert_eq!(result.received_refunds, Decimal::from(20));
        assert!(result.behavior_score.unwrap() < 100);
        assert_eq!(result.on_track_probability, None);
        let stale = forecast(&rows, false);
        assert!(!stale.signal.alert);
        assert!(stale.behavior_score.is_none() && stale.deductions.is_empty());
    }

    /// Continues each context's last daily step, or fails; counts calls.
    #[derive(Debug)]
    struct Trend {
        fail: bool,
        calls: std::cell::Cell<usize>,
    }

    impl SeriesForecaster for Trend {
        fn name(&self) -> &str {
            "trend"
        }

        fn supports_covariates(&self) -> bool {
            false
        }

        fn quantiles(
            &self,
            contexts: &[Vec<f64>],
            horizon: usize,
            _: Option<&[Vec<f64>]>,
        ) -> Result<Vec<Vec<[f64; 3]>>, String> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Err("no model".into());
            }
            Ok(contexts
                .iter()
                .map(|c| {
                    let (last, step) = (c[c.len() - 1], c[c.len() - 1] - c[c.len() - 2]);
                    (1..=horizon)
                        .map(|k| {
                            let v = last + step * k as f64;
                            [v, v, v]
                        })
                        .collect()
                })
                .collect())
        }
    }

    #[test]
    fn a_model_that_beats_the_baseline_drives_the_outlook() {
        // Unplanned spending grows steadily; the weekday baseline lags it.
        let values: Vec<Day> = (0..300).map(|i| [1.0, 0.1 * f64::from(i)]).collect();
        let start = date(2025, 1, 1);
        let model = Trend {
            fail: false,
            calls: std::cell::Cell::new(0),
        };
        let result = fit_behavior(&values, start, 90, Some(&model));
        assert_eq!(result.method, "timesfm");
        assert!(result.model_error.unwrap() < 1e-6);
        assert!(result.baseline_error.unwrap() > 100.0);
        // Judged blocks in one call, the live forecast and candidates in another.
        assert_eq!(model.calls.get(), 2);
        assert_eq!(result.horizon_windows, 2);

        let failing = Trend {
            fail: true,
            calls: std::cell::Cell::new(0),
        };
        let fallback = fit_behavior(&values, start, 90, Some(&failing));
        let plain = fit_behavior(&values, start, 90, None);
        assert_eq!(fallback.method, "weighted_weekday_baseline");
        assert_eq!(fallback.model_error, None);
        assert_eq!(fallback.paths, plain.paths);
        assert_eq!(fallback.coverage, plain.coverage);
    }

    #[test]
    fn transfers_never_create_spending() {
        let ordinary: Vec<Tx> = (1..35)
            .map(|i| purchase(&i.to_string(), "10", i, None, Kind::Outflow))
            .collect();
        let a = forecast(&ordinary, true);
        let mut with_transfer = ordinary.clone();
        with_transfer.push(purchase("move", "5000", 2, None, Kind::Transfer));
        let b = forecast(&with_transfer, true);
        assert_eq!(a.expected_savings, b.expected_savings);
        assert_eq!(a.signal, b.signal);
    }

    #[test]
    fn empty_history_is_explicit() {
        let result = forecast(&[], true);
        assert_eq!(result.training_days, 0);
        assert_eq!(result.method, "insufficient_history");
        assert_eq!(result.on_track_probability, None);
        assert_eq!(result.behavior_score, None);
        assert_eq!(result.through, "2026-12-28");
        assert_eq!(result.starting_balance, Decimal::from(1000));
    }

    #[test]
    fn constant_history_keeps_the_baseline_level() {
        let values = vec![[5.0, 5.0]; 280];
        let result = fit_behavior(&values, date(2025, 12, 22), 90, None);
        assert_eq!(result.method, "weighted_weekday_baseline");
        assert_eq!(result.validation_windows, 3);
        assert_eq!(result.horizon_windows, 1);
        assert_eq!(result.paths.len(), SIMULATIONS);
        for path in &result.paths {
            assert_eq!(path.len(), 90);
            let total: f64 = path.iter().map(|d| d[0] + d[1]).sum();
            assert!((total - 900.0).abs() < 1e-6);
        }
    }

    #[test]
    fn budget_owner_prefers_purchase_then_category_then_group() {
        let mut groups = default_budget(date(2026, 9, 1), &Currency::eur()).groups;
        groups[1].categories.push("Lunch".into());
        groups[0].transaction_ids.push("t1".into());
        assert_eq!(
            budget_owner(&groups, Some("Lunch"), None, Some("t1")),
            Some("groceries")
        );
        assert_eq!(
            budget_owner(&groups, Some("Lunch"), Some("Groceries"), None),
            Some("food-out")
        );
        assert_eq!(
            budget_owner(&groups, None, Some("Coffee"), None),
            Some("food-out")
        );
        assert_eq!(budget_owner(&groups, Some("Rent"), None, None), None);
    }
}
