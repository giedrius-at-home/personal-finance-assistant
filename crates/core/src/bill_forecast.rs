//! Bills whose amount varies (an NS travel invoice): the next payment of each
//! owner-listed category, predicted from its past monthly totals with the
//! series model (TimesFM), or the average of the last three paid months
//! without it.

use std::collections::BTreeMap;

use finance_types::report::closure::PredictedBill;
use jiff::civil::Date;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use crate::daily::SeriesForecaster;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::text::casefold;

/// At most this many past months feed a prediction.
const HISTORY_MONTHS: i32 = 24;
/// The fallback averages this many of the latest paid months.
const AVERAGE_MONTHS: usize = 3;

/// The next payment of every `categories` entry with at least two paid
/// months. `month_of` gives a transaction's budget month; `current_month`
/// is the running one.
pub fn predicted_bills(
    transactions: &[Tx],
    categories: &[String],
    currency: &str,
    current_month: Date,
    month_of: impl Fn(&Tx) -> Date,
    forecaster: Option<&dyn SeriesForecaster>,
) -> Vec<PredictedBill> {
    let first = shift_month(current_month, -HISTORY_MONTHS);
    categories
        .iter()
        .filter_map(|category| {
            let wanted = casefold(category.trim());
            let paid: Vec<&Tx> = transactions
                .iter()
                .filter(|t| {
                    t.kind == Kind::Outflow
                        && t.currency.code == currency
                        && t.category
                            .as_deref()
                            .is_some_and(|c| casefold(c.trim()) == wanted)
                })
                .collect();
            let mut totals: BTreeMap<Date, Decimal> = BTreeMap::new();
            for t in &paid {
                let month = month_of(t);
                if first <= month && month <= current_month {
                    *totals.entry(month).or_default() += t.amount;
                }
            }
            let last = paid.iter().max_by_key(|t| t.effective_on())?;
            let paid_this_month = totals.get(&current_month).is_some_and(|v| !v.is_zero());
            let through = if paid_this_month {
                current_month
            } else {
                shift_month(current_month, -1)
            };
            let start = *totals.keys().next()?;
            let mut history = Vec::new();
            let mut month = start;
            while month <= through {
                history.push(totals.get(&month).copied().unwrap_or_default());
                month = shift_month(month, 1);
            }
            let paid_months: Vec<Decimal> =
                history.iter().copied().filter(|v| !v.is_zero()).collect();
            if paid_months.len() < 2 {
                return None;
            }
            let (amount, method) = forecaster
                .and_then(|f| model_prediction(f, &history))
                .map(|a| (a, f_name(forecaster)))
                .unwrap_or_else(|| (average(&paid_months), average_label()));
            let target = shift_month(through, 1);
            Some(PredictedBill {
                category: category.trim().to_owned(),
                account_id: last.account_id.clone(),
                amount,
                due_on: due_on(target, last.effective_on().day()).to_string(),
                paid_this_month,
                method,
                history,
            })
        })
        .collect()
}

fn f_name(forecaster: Option<&dyn SeriesForecaster>) -> String {
    forecaster.map_or_else(average_label, |f| f.name().to_owned())
}

fn average_label() -> String {
    format!("Average of the last {AVERAGE_MONTHS} months")
}

/// The model's expected next month; `None` when it fails or is not a
/// usable amount.
fn model_prediction(forecaster: &dyn SeriesForecaster, history: &[Decimal]) -> Option<Decimal> {
    let context: Vec<f64> = history.iter().map(|v| v.to_f64().unwrap_or(0.0)).collect();
    let rows = forecaster.quantiles(&[context], 1, None).ok()?;
    let mean = rows.first()?.first()?[1];
    if !mean.is_finite() || mean <= 0.0 {
        return None;
    }
    Decimal::from_f64(mean).map(|d| d.round_dp(2))
}

/// The mean of the latest paid months.
fn average(paid: &[Decimal]) -> Decimal {
    let latest = &paid[paid.len().saturating_sub(AVERAGE_MONTHS)..];
    (latest.iter().copied().sum::<Decimal>() / Decimal::from(latest.len())).round_dp(2)
}

/// `day` in `month`, or its last day.
fn due_on(month: Date, day: i8) -> Date {
    let last = month.last_of_month().day();
    month.with().day(day.min(last)).build().unwrap_or(month)
}

#[cfg(test)]
mod tests {
    use finance_types::report::Currency;
    use jiff::civil::date;

    use super::*;

    fn ns(amount: &str, on: Date) -> Tx {
        Tx {
            transaction_id: format!("{on}-{amount}"),
            ledger_id: 1,
            account_id: "abn".into(),
            counterparty_account_id: None,
            kind: Kind::Outflow,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: on.at(12, 0, 0, 0).in_tz("Europe/Amsterdam").unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: Some("NS".into()),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
            repays: None,
        }
    }

    fn by_calendar(t: &Tx) -> Date {
        t.effective_on().first_of_month()
    }

    #[derive(Debug)]
    struct Fixed(f64);

    impl SeriesForecaster for Fixed {
        fn name(&self) -> &str {
            "TimesFM"
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
            Ok(contexts
                .iter()
                .map(|_| vec![[0.0, self.0, 0.0]; horizon])
                .collect())
        }
    }

    fn categories() -> Vec<String> {
        vec![" ns ".into()]
    }

    #[test]
    fn paid_this_month_predicts_the_next_one() {
        let txs = [
            ns("20.00", date(2026, 8, 10)),
            ns("31.00", date(2026, 9, 10)),
            ns("24.50", date(2026, 10, 12)),
            ns("28.89", date(2026, 11, 10)),
        ];
        let got = predicted_bills(
            &txs,
            &categories(),
            "EUR",
            date(2026, 11, 1),
            by_calendar,
            None,
        );
        assert_eq!(got.len(), 1);
        let bill = &got[0];
        assert!(bill.paid_this_month);
        assert_eq!(bill.due_on, "2026-12-10");
        assert_eq!(bill.account_id, "abn");
        // (31 + 24.50 + 28.89) / 3
        assert_eq!(bill.amount, "28.13".parse::<Decimal>().unwrap());
        assert_eq!(bill.method, "Average of the last 3 months");
        assert_eq!(bill.history.len(), 4);
    }

    #[test]
    fn unpaid_month_is_predicted_from_the_months_before() {
        let txs = [ns("20", date(2026, 8, 31)), ns("30", date(2026, 10, 31))];
        let got = predicted_bills(
            &txs,
            &categories(),
            "EUR",
            date(2026, 11, 1),
            by_calendar,
            Some(&Fixed(27.456)),
        );
        let bill = &got[0];
        assert!(!bill.paid_this_month);
        // The 31st in a 30-day month falls on its last day.
        assert_eq!(bill.due_on, "2026-11-30");
        assert_eq!(bill.amount, "27.46".parse::<Decimal>().unwrap());
        assert_eq!(bill.method, "TimesFM");
        // August, an unpaid September, October.
        assert_eq!(bill.history.len(), 3);
        assert!(bill.history[1].is_zero());
    }

    #[test]
    fn a_failed_model_falls_back_and_one_month_is_too_little() {
        let txs = [ns("20", date(2026, 9, 10)), ns("30", date(2026, 10, 10))];
        let got = predicted_bills(
            &txs,
            &categories(),
            "EUR",
            date(2026, 11, 1),
            by_calendar,
            Some(&Fixed(f64::NAN)),
        );
        assert_eq!(got[0].amount, Decimal::from(25));
        assert_eq!(got[0].method, "Average of the last 3 months");
        let one = [ns("20", date(2026, 10, 10))];
        assert!(
            predicted_bills(
                &one,
                &categories(),
                "EUR",
                date(2026, 11, 1),
                by_calendar,
                None
            )
            .is_empty()
        );
    }
}
