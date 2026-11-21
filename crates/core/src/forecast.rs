//! Month-end forecasts for reviewed, variable spending groups (Python
//! `spending_forecast.py`).
//!
//! Every group starts from Python's historical blend: this month's pace
//! averaged with the group's median month, the gap between them as the range
//! (`historical_fallback`, confidence 0.35). Python replaced it with a
//! CatBoost quantile model once three complete months existed; this port
//! uses TimesFM there instead (`timesfm`), through the app's
//! [`SeriesForecaster`]. Without a model every group keeps the fallback.
//!
//! With a model and at least three complete months:
//!
//! * Each group's daily net spending (refunds negative, excluded ids left
//!   out) is laid out month by month as Python keyed it: budget months from
//!   the resolver, days of the month, spending dated before its month on
//!   its first day. TimesFM forecasts the days left in the current period
//!   from that series; expected = spent + Σ daily means, never below spent.
//! * The 10–90% band treats days as independent: each day's half-range
//!   (q90 − q10) / 2 is a normal 1.28σ, and a sum of independent days has
//!   σ = √Σσ², so the half-width is √Σ((q90 − q10) / 2)². Summing the daily
//!   deciles instead would assume every day lands in its tail together — far
//!   too wide for weeks of mostly independent purchases. Lower is floored at
//!   spent, as Python did.
//! * Per group, both methods are backtested on the last three complete months
//!   at the same day of the month: forecast that month's total from its first
//!   days. The lower mean absolute error wins (ties keep the fallback), as
//!   the daily forecast's backtest chooses. Confidence is then the chosen
//!   method's accuracy on those months, 1 − MAE / mean |actual total|,
//!   bounded to 0.05–0.95, instead of CatBoost's 0.45 + 0.06 per month.
//!
//! One batched model call covers every group's live and backtest contexts.

use std::collections::{BTreeSet, HashMap, HashSet};

use finance_types::gateway::GroupMapping;
use finance_types::report::Currency;
use finance_types::report::trends::{ForecastMethod, SpendingForecast};
use jiff::Zoned;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::daily::SeriesForecaster;
use crate::groups::group_index;
use crate::model::{Kind, Tx};
use crate::outlook::np;
use crate::period::shift_month;
use crate::reporting::{py_div, py_round};
use crate::trends::{cents, median, py_div_mul};

/// Confidence Python gave its fallback forecasts.
const FALLBACK_CONFIDENCE: Decimal = Decimal::from_parts(35, 0, 0, false, 2);
/// Complete months a currency needs before the model is tried.
const MIN_COMPLETE_MONTHS: usize = 3;
/// Recent complete months each group is backtested on.
const BACKTEST_MONTHS: usize = 3;
/// The model reads at most two years of daily history.
const MAX_CONTEXT_MONTHS: i32 = 24;
/// Backtest contexts shorter than four weeks are skipped.
const MIN_CONTEXT_DAYS: usize = 28;

/// The budget month a forecast covers: `current_month` (its first day) runs
/// from `period_start` (an earlier cutoff day, or the 1st) to its calendar end.
#[derive(Clone, Copy, Debug)]
pub struct ForecastPeriod {
    pub current_month: Date,
    pub period_start: Date,
}

pub(crate) fn signed(t: &Tx) -> Decimal {
    if t.kind == Kind::Outflow {
        t.amount
    } else {
        -t.amount
    }
}

/// Python's fallback: `(lower, expected, upper)` from this month's pace and
/// the group's history.
fn historical(
    spent: Decimal,
    progress: i32,
    period_days: i32,
    history: &[Decimal],
    previous: Decimal,
) -> (Decimal, Decimal, Decimal) {
    let historical = median(history).unwrap_or(previous);
    let pace = py_div_mul(
        spent,
        u64::try_from(progress).unwrap_or(1),
        u64::try_from(period_days).unwrap_or(1),
    );
    let expected = spent.max(py_div(py_round(pace + historical), Decimal::TWO));
    let spread = py_div(py_round(pace - historical).abs(), Decimal::TWO);
    let lower = spent.max(py_round(expected - spread));
    let upper = py_round(expected + spread);
    (lower, expected, upper)
}

fn days_in(month: Date) -> usize {
    usize::try_from(month.days_in_month()).unwrap_or(28)
}

type Key = (Currency, String, Date);

/// Net spending per budget month and day of month.
#[derive(Default)]
struct Ledger {
    totals: HashMap<Key, Decimal>,
    days: HashMap<Key, HashMap<usize, Decimal>>,
}

impl Ledger {
    fn total(&self, currency: &Currency, group: &str, month: Date) -> Option<Decimal> {
        self.totals
            .get(&(currency.clone(), group.to_owned(), month))
            .copied()
    }

    /// Spending in `month` on days 1 through `through`.
    fn spent_through(
        &self,
        currency: &Currency,
        group: &str,
        month: Date,
        through: usize,
    ) -> Decimal {
        self.days
            .get(&(currency.clone(), group.to_owned(), month))
            .map(|days| {
                days.iter()
                    .filter(|(d, _)| **d <= through)
                    .map(|(_, v)| *v)
                    .sum()
            })
            .unwrap_or_default()
    }

    /// Daily values of `month`, days 1 through `through`.
    fn month_days(
        &self,
        currency: &Currency,
        group: &str,
        month: Date,
        through: usize,
    ) -> Vec<f64> {
        let days = self.days.get(&(currency.clone(), group.to_owned(), month));
        (1..=through)
            .map(|d| days.and_then(|m| m.get(&d)).map_or(0.0, |v| np::to_f64(*v)))
            .collect()
    }
}

/// One model request: a daily context and the days to forecast after it.
struct Request {
    context: Vec<f64>,
    horizon: usize,
}

/// One backtest month: the request, and what each method must explain.
struct Backtest {
    request: usize,
    spent: f64,
    actual: f64,
    fallback: f64,
}

struct Plan {
    /// Index into the results.
    result: usize,
    live: Option<usize>,
    backtests: Vec<Backtest>,
}

/// Expected month-end totals per currency and reviewed group, sorted by
/// currency code then group. `excluded_ids` are fixed spending and paid
/// planned payments, which are known amounts rather than habits.
/// `month_of` assigns each transaction its budget month (closure-aware).
/// `forecaster` (TimesFM) replaces the fallback where it backtests better.
pub fn spending_forecasts(
    transactions: &[Tx],
    mappings: &[GroupMapping],
    now: &Zoned,
    excluded_ids: &HashSet<String>,
    month_of: impl Fn(&Tx) -> Date,
    period: ForecastPeriod,
    forecaster: Option<&dyn SeriesForecaster>,
) -> Vec<SpendingForecast> {
    let today = now.date();
    let ForecastPeriod {
        current_month,
        period_start: start,
    } = period;
    let period_end = current_month.last_of_month();
    let period_days = ((period_end - start).get_days() + 1).max(1);
    let progress = period_days.min(((today - start).get_days() + 1).max(1));
    let reviewed = group_index(mappings);

    let mut ledger = Ledger::default();
    let mut seen: BTreeSet<(String, String, Currency)> = BTreeSet::new();
    for t in transactions {
        let Some(group) = reviewed
            .get(t.category.as_deref().unwrap_or(""))
            .copied()
            .flatten()
        else {
            continue;
        };
        if !matches!(t.kind, Kind::Outflow | Kind::Refund)
            || excluded_ids.contains(&t.transaction_id)
            || t.occurred_at.timestamp() > now.timestamp()
        {
            continue;
        }
        let month = month_of(t);
        let key = (t.currency.clone(), group.to_owned(), month);
        *ledger.totals.entry(key.clone()).or_default() += signed(t);
        // Spending assigned to a month but dated before it counts on day 1.
        let occurred = t.effective_on();
        let day = if occurred >= month {
            usize::try_from(occurred.day()).unwrap_or(1)
        } else {
            1
        };
        *ledger.days.entry(key).or_default().entry(day).or_default() += signed(t);
        seen.insert((
            t.currency.code.clone(),
            group.to_owned(),
            t.currency.clone(),
        ));
    }

    let mut results = Vec::new();
    let mut plans: Vec<Plan> = Vec::new();
    let mut requests: Vec<Request> = Vec::new();
    // The day of the month the period has reached (Python `model_day`).
    let model_day = usize::try_from(progress)
        .unwrap_or(1)
        .min(days_in(current_month));
    // The current month's days so far, as the month-keyed layout has them.
    let today_day = if today >= current_month {
        usize::try_from(today.day()).unwrap_or(1)
    } else {
        1
    };
    let remaining = usize::try_from((period_end - today).get_days()).unwrap_or(0);
    let mut codes: Vec<&str> = seen.iter().map(|(code, _, _)| code.as_str()).collect();
    codes.dedup();
    for code in codes {
        let complete_months: BTreeSet<Date> = ledger
            .totals
            .keys()
            .filter(|(c, _, m)| c.code == code && *m < current_month)
            .map(|(_, _, m)| *m)
            .collect();
        for (_, group, currency) in seen.iter().filter(|(c, _, _)| c == code) {
            let spent = ledger
                .total(currency, group, current_month)
                .unwrap_or_default();
            let history_before = |month: Date| -> Vec<Decimal> {
                complete_months
                    .iter()
                    .filter(|m| **m < month)
                    .filter_map(|m| ledger.total(currency, group, *m))
                    .collect()
            };
            let history = history_before(current_month);
            let previous = |month: Date| {
                ledger
                    .total(currency, group, shift_month(month, -1))
                    .unwrap_or_default()
            };
            let (lower, expected, upper) = historical(
                spent,
                progress,
                period_days,
                &history,
                previous(current_month),
            );
            results.push(SpendingForecast {
                group: group.clone(),
                currency: currency.clone(),
                spent,
                lower: cents(lower),
                expected: cents(expected),
                upper: cents(upper),
                confidence: FALLBACK_CONFIDENCE,
                method: ForecastMethod::HistoricalFallback,
                training_months: complete_months.len(),
            });
            if forecaster.is_none() || complete_months.len() < MIN_COMPLETE_MONTHS {
                continue;
            }

            // The group's months, oldest first, laid out day by day.
            let first_month = ledger
                .totals
                .keys()
                .filter(|(c, g, _)| c == currency && g == group)
                .map(|(_, _, m)| *m)
                .min()
                .unwrap_or(current_month)
                .max(shift_month(current_month, -MAX_CONTEXT_MONTHS));
            let mut months = Vec::new();
            let mut month = first_month;
            while month < current_month {
                months.push(month);
                month = shift_month(month, 1);
            }
            let mut layout: Vec<f64> = Vec::new();
            let mut starts: HashMap<Date, usize> = HashMap::new();
            for m in &months {
                starts.insert(*m, layout.len());
                layout.extend(ledger.month_days(currency, group, *m, days_in(*m)));
            }
            let mut plan = Plan {
                result: results.len() - 1,
                live: None,
                backtests: Vec::new(),
            };
            if remaining > 0 {
                let mut context = layout.clone();
                context.extend(ledger.month_days(currency, group, current_month, today_day));
                plan.live = Some(requests.len());
                requests.push(Request {
                    context,
                    horizon: remaining,
                });
            }
            for m in complete_months.iter().rev().take(BACKTEST_MONTHS) {
                let Some(&offset) = starts.get(m) else {
                    continue;
                };
                let cut = model_day.min(days_in(*m));
                if offset + cut < MIN_CONTEXT_DAYS || cut >= days_in(*m) {
                    continue;
                }
                let spent_at = ledger.spent_through(currency, group, *m, cut);
                let actual = ledger.total(currency, group, *m).map_or(0.0, np::to_f64);
                let (_, fallback, _) = historical(
                    spent_at,
                    i32::try_from(cut).unwrap_or(1),
                    i32::try_from(days_in(*m)).unwrap_or(1),
                    &history_before(*m),
                    previous(*m),
                );
                plan.backtests.push(Backtest {
                    request: requests.len(),
                    spent: np::to_f64(spent_at),
                    actual,
                    fallback: np::to_f64(fallback),
                });
                requests.push(Request {
                    context: layout[..offset + cut].to_vec(),
                    horizon: days_in(*m) - cut,
                });
            }
            plans.push(plan);
        }
    }

    let Some(model) = forecaster else {
        return results;
    };
    let horizon = requests.iter().map(|r| r.horizon).max().unwrap_or(0);
    if horizon == 0 {
        return results;
    }
    let contexts: Vec<Vec<f64>> = requests.iter().map(|r| r.context.clone()).collect();
    // A failing model leaves every group on the fallback.
    let Ok(rows) = model.quantiles(&contexts, horizon, None) else {
        return results;
    };
    let rest = |request: usize| -> &[[f64; 3]] {
        let r = &rows[request];
        &r[..requests[request].horizon.min(r.len())]
    };
    for plan in plans {
        if plan.backtests.is_empty() {
            continue;
        }
        let errors = |predict: &dyn Fn(&Backtest) -> f64| -> f64 {
            let misses: Vec<f64> = plan
                .backtests
                .iter()
                .map(|b| (predict(b) - b.actual).abs())
                .collect();
            np::mean(&misses)
        };
        let model_error = errors(&|b| {
            b.spent
                .max(b.spent + np::sum_iter(rest(b.request).iter().map(|d| d[1])))
        });
        let fallback_error = errors(&|b| b.fallback);
        let use_model = model_error < fallback_error && plan.live.is_some();
        let scale = np::mean(
            &plan
                .backtests
                .iter()
                .map(|b| b.actual.abs())
                .collect::<Vec<_>>(),
        );
        let chosen = if use_model {
            model_error
        } else {
            fallback_error
        };
        let forecast = &mut results[plan.result];
        if scale > 0.0 {
            let accuracy = (1.0 - chosen / scale).clamp(0.05, 0.95);
            forecast.confidence = np::round_decimal(accuracy, 2);
        }
        let (true, Some(live)) = (use_model, plan.live) else {
            continue;
        };
        let days = rest(live);
        let spent = np::to_f64(forecast.spent);
        let expected = spent.max(spent + np::sum_iter(days.iter().map(|d| d[1])));
        let half = np::sum_iter(days.iter().map(|d| {
            let h = (d[2] - d[0]) / 2.0;
            h * h
        }))
        .sqrt();
        forecast.expected = cents(np::round_decimal(expected, 2));
        forecast.lower = cents(np::round_decimal(spent.max(expected - half), 2));
        forecast.upper = cents(np::round_decimal(expected + half, 2));
        forecast.method = ForecastMethod::Timesfm;
    }
    results
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;
    use crate::period::month_start;
    use crate::trends::tests::{mapping, out, tx};

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn calendar(t: &Tx) -> Date {
        month_start(t.effective_on())
    }

    fn period(month: Date) -> ForecastPeriod {
        ForecastPeriod {
            current_month: month,
            period_start: month,
        }
    }

    #[test]
    fn fallback_blends_pace_with_history() {
        let now: Zoned = "2026-09-10T12:00:00+00:00[UTC]".parse().unwrap();
        let rows = vec![
            out(
                "jul",
                "200",
                Some("Jumbo"),
                "2026-07-05T12:00:00+00:00[UTC]",
            ),
            out(
                "aug",
                "300",
                Some("Jumbo"),
                "2026-08-05T12:00:00+00:00[UTC]",
            ),
            out("a", "60", Some("Jumbo"), "2026-09-02T12:00:00+00:00[UTC]"),
            tx(
                "r",
                "10",
                Kind::Refund,
                Some("Jumbo"),
                "2026-09-03T12:00:00+00:00[UTC]",
            ),
            out(
                "fixed",
                "500",
                Some("Jumbo"),
                "2026-09-03T12:00:00+00:00[UTC]",
            ),
            out(
                "later",
                "500",
                Some("Jumbo"),
                "2026-09-10T13:00:00+00:00[UTC]",
            ),
            out(
                "unmapped",
                "5",
                Some("Kiosk"),
                "2026-09-03T12:00:00+00:00[UTC]",
            ),
        ];
        let excluded = HashSet::from(["fixed".to_owned()]);
        let result = spending_forecasts(
            &rows,
            &[mapping("Jumbo", "Groceries")],
            &now,
            &excluded,
            calendar,
            period(date(2026, 9, 1)),
            None,
        );
        assert_eq!(result.len(), 1);
        let f = &result[0];
        // pace 50/10*30 = 150, history median 250: expected 200 ± 50.
        assert_eq!(f.spent, dec("50"));
        assert_eq!(
            (f.lower, f.expected, f.upper),
            (dec("150"), dec("200"), dec("250"))
        );
        assert_eq!(f.expected.to_string(), "200.00");
        assert_eq!(f.method, ForecastMethod::HistoricalFallback);
        assert_eq!((f.confidence, f.training_months), (dec("0.35"), 2));
    }

    #[test]
    fn early_cutoff_period_and_floor_at_spent() {
        // Month of October starts on 28 September; today is 30 September.
        let now: Zoned = "2026-09-30T12:00:00+00:00[UTC]".parse().unwrap();
        let rows = vec![
            out("big", "90", Some("Jumbo"), "2026-09-29T12:00:00+00:00[UTC]"),
            out("old", "10", Some("Jumbo"), "2026-08-05T12:00:00+00:00[UTC]"),
        ];
        let october = date(2026, 10, 1);
        let result = spending_forecasts(
            &rows,
            &[mapping("Jumbo", "Groceries")],
            &now,
            &HashSet::new(),
            |t| {
                if t.effective_on() >= date(2026, 9, 28) {
                    october
                } else {
                    month_start(t.effective_on())
                }
            },
            ForecastPeriod {
                current_month: october,
                period_start: date(2026, 9, 28),
            },
            None,
        );
        let f = &result[0];
        // 34 days in the period, 3 elapsed: pace 1020, previous (September)
        // is empty, so history is August's 10.
        assert_eq!(f.spent, dec("90"));
        assert_eq!(f.expected, dec("515.00"));
        assert_eq!((f.lower, f.upper), (dec("90.00"), dec("1020.00")));
        assert_eq!(f.training_months, 1);
    }

    #[test]
    fn groups_without_current_spending_still_forecast() {
        let now: Zoned = "2026-09-10T12:00:00+00:00[UTC]".parse().unwrap();
        let rows = vec![
            out("a", "30", Some("Cinema"), "2026-08-05T12:00:00+00:00[UTC]"),
            out("b", "8", Some("Jumbo"), "2026-09-01T12:00:00+00:00[UTC]"),
        ];
        let result = spending_forecasts(
            &rows,
            &[
                mapping("Cinema", "Entertainment"),
                mapping("Jumbo", "Groceries"),
            ],
            &now,
            &HashSet::new(),
            calendar,
            period(date(2026, 9, 1)),
            None,
        );
        let groups: Vec<_> = result.iter().map(|f| f.group.as_str()).collect();
        assert_eq!(groups, ["Entertainment", "Groceries"]);
        assert_eq!(
            (
                result[0].spent,
                result[0].lower,
                result[0].expected,
                result[0].upper
            ),
            (Decimal::ZERO, dec("0"), dec("15"), dec("30"))
        );
        // Groceries has no history: historical falls back to last month (0).
        assert_eq!(
            (result[1].lower, result[1].expected, result[1].upper),
            (dec("8"), dec("12"), dec("24"))
        );
    }

    /// Forecasts 10 a day with a 4–16 band, or fails.
    #[derive(Debug)]
    struct Steady {
        fail: bool,
        calls: std::cell::Cell<usize>,
    }

    impl SeriesForecaster for Steady {
        fn name(&self) -> &str {
            "steady"
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
            Ok(vec![vec![[4.0, 10.0, 16.0]; horizon]; contexts.len()])
        }
    }

    /// Ten a day from June through 10 October.
    fn steady_spending() -> Vec<Tx> {
        let mut day = date(2026, 6, 1);
        let mut rows = Vec::new();
        while day <= date(2026, 10, 10) {
            rows.push(out(
                &format!("t{day}"),
                "10",
                Some("Jumbo"),
                &format!("{day}T08:00:00+00:00[UTC]"),
            ));
            day = day.tomorrow().unwrap();
        }
        rows
    }

    fn with_model(model: &Steady) -> SpendingForecast {
        let now: Zoned = "2026-10-10T12:00:00+00:00[UTC]".parse().unwrap();
        spending_forecasts(
            &steady_spending(),
            &[mapping("Jumbo", "Groceries")],
            &now,
            &HashSet::new(),
            calendar,
            period(date(2026, 10, 1)),
            Some(model),
        )
        .remove(0)
    }

    #[test]
    fn the_model_replaces_the_fallback_when_it_backtests_better() {
        let model = Steady {
            fail: false,
            calls: std::cell::Cell::new(0),
        };
        let f = with_model(&model);
        // One call for the live and every backtest context.
        assert_eq!(model.calls.get(), 1);
        assert_eq!(f.method, ForecastMethod::Timesfm);
        assert_eq!(f.spent, dec("100"));
        // 21 days left at 10 a day; half-width 6 × √21 for independent days.
        assert_eq!(f.expected, dec("310.00"));
        assert_eq!((f.lower, f.upper), (dec("282.50"), dec("337.50")));
        // The model was exact on the backtest months.
        assert_eq!(f.confidence, dec("0.95"));
        assert_eq!(f.training_months, 4);
    }

    #[test]
    fn a_failing_model_keeps_the_fallback() {
        let model = Steady {
            fail: true,
            calls: std::cell::Cell::new(0),
        };
        let f = with_model(&model);
        assert_eq!(f.method, ForecastMethod::HistoricalFallback);
        assert_eq!(f.confidence, dec("0.35"));
    }
}
