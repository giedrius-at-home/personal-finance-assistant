//! Month-end forecasts for reviewed, variable spending groups (Python
//! `spending_forecast.py`).
//!
//! Python trained a CatBoost quantile model once three complete months
//! existed and fell back to a historical blend otherwise. The model is not
//! ported: every group uses that fallback — this month's pace averaged with
//! the group's median month, the gap between them as the range.

use std::collections::{BTreeSet, HashMap, HashSet};

use finance_types::gateway::GroupMapping;
use finance_types::report::Currency;
use finance_types::report::trends::{ForecastMethod, SpendingForecast};
use jiff::Zoned;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::groups::group_index;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::reporting::{py_div, py_round};
use crate::trends::{cents, median, py_div_mul};

/// Confidence Python gave its fallback forecasts.
const FALLBACK_CONFIDENCE: Decimal = Decimal::from_parts(35, 0, 0, false, 2);

/// The budget month a forecast covers: `current_month` (its first day) runs
/// from `period_start` (an earlier cutoff day, or the 1st) to its calendar end.
#[derive(Clone, Copy, Debug)]
pub struct ForecastPeriod {
    pub current_month: Date,
    pub period_start: Date,
}

fn signed(t: &Tx) -> Decimal {
    if t.kind == Kind::Outflow {
        t.amount
    } else {
        -t.amount
    }
}

/// Expected month-end totals per currency and reviewed group, sorted by
/// currency code then group. `excluded_ids` are fixed spending and paid
/// planned payments, which are known amounts rather than habits.
/// `month_of` assigns each transaction its budget month (closure-aware).
pub fn spending_forecasts(
    transactions: &[Tx],
    mappings: &[GroupMapping],
    now: &Zoned,
    excluded_ids: &HashSet<String>,
    month_of: impl Fn(&Tx) -> Date,
    period: ForecastPeriod,
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

    type Key = (Currency, String, Date);
    let mut totals: HashMap<Key, Decimal> = HashMap::new();
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
        *totals
            .entry((t.currency.clone(), group.to_owned(), month))
            .or_default() += signed(t);
        seen.insert((
            t.currency.code.clone(),
            group.to_owned(),
            t.currency.clone(),
        ));
    }

    let mut results = Vec::new();
    let mut codes: Vec<&str> = seen.iter().map(|(code, _, _)| code.as_str()).collect();
    codes.dedup();
    for code in codes {
        let complete_months: BTreeSet<Date> = totals
            .keys()
            .filter(|(c, _, m)| c.code == code && *m < current_month)
            .map(|(_, _, m)| *m)
            .collect();
        for (_, group, currency) in seen.iter().filter(|(c, _, _)| c == code) {
            let total = |month: Date| totals.get(&(currency.clone(), group.clone(), month));
            let spent = total(current_month).copied().unwrap_or_default();
            let history: Vec<Decimal> = complete_months
                .iter()
                .filter_map(|m| total(*m).copied())
                .collect();
            let previous = total(shift_month(current_month, -1))
                .copied()
                .unwrap_or_default();
            let historical = median(&history).unwrap_or(previous);
            let pace = py_div_mul(
                spent,
                u64::try_from(progress).unwrap_or(1),
                u64::try_from(period_days).unwrap_or(1),
            );
            let expected = spent.max(py_div(py_round(pace + historical), Decimal::TWO));
            let spread = py_div(py_round(pace - historical).abs(), Decimal::TWO);
            let lower = spent.max(py_round(expected - spread));
            let upper = py_round(expected + spread);
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
        }
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
}
