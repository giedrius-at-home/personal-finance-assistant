//! Spending over time: six calendar months of confirmed spending by group,
//! category, day and weekday, never predictions (Python
//! `domain/time_trends.py`).

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::gateway::GroupMapping;
use finance_types::report::Currency;
use finance_types::report::trends::{
    CategoryTrend, CurrencyTrends, DailySpending, GroupTrend, MonthSpending, TimeTrends,
    WeekdaySpending,
};
use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::forecast::signed;
use crate::groups::{group_index, net};
use crate::model::{Kind, Tx};
use crate::period::{month_start, shift_month};
use crate::reporting::{digit_count, py_div, py_round};
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

/// `a * b` as Python's `Decimal` multiplies: exact, then 28 significant
/// digits (half-even).
pub(crate) fn py_mul(a: Decimal, b: Decimal) -> Decimal {
    let product = a
        .mantissa()
        .unsigned_abs()
        .checked_mul(b.mantissa().unsigned_abs());
    let Some(mut digits) = product else {
        return py_round(a * b);
    };
    let mut scale = a.scale() + b.scale();
    let mut length: u32 = 0;
    let mut rest = digits;
    while rest > 0 {
        rest /= 10;
        length += 1;
    }
    // Keep 28 significant digits and at most 28 decimal places.
    let excess = (length.saturating_sub(28)).max(scale.saturating_sub(28));
    if excess > 0 {
        if excess > scale {
            return py_round(a * b);
        }
        let unit = 10u128.pow(excess);
        let (kept, dropped) = (digits / unit, digits % unit);
        digits = kept;
        if dropped * 2 > unit || (dropped * 2 == unit && kept % 2 == 1) {
            digits += 1;
        }
        scale -= excess;
    }
    let Ok(signed) = i128::try_from(digits) else {
        return py_round(a * b);
    };
    let magnitude =
        Decimal::try_from_i128_with_scale(signed, scale).unwrap_or_else(|_| py_round(a * b).abs());
    if a.is_sign_negative() != b.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// `digits × 10^exp` cut to 28 significant digits, half-even; `sticky` says
/// a nonzero remainder lies below the last digit.
fn round_significant(mut digits: u128, mut exp: i64, sticky: bool) -> (u128, i64) {
    let excess = digit_count(digits).saturating_sub(28);
    if excess > 0 {
        let unit = 10u128.pow(excess);
        let dropped = digits % unit;
        digits /= unit;
        exp += i64::from(excess);
        let half = (dropped * 2).cmp(&unit);
        if half.is_gt() || (half.is_eq() && (sticky || digits % 2 == 1)) {
            digits += 1;
            if digit_count(digits) > 28 {
                digits /= 10;
                exp += 1;
            }
        }
    }
    (digits, exp)
}

/// Python's `a / divisor * factor` (both rounded to 28 significant digits)
/// with positive integers. The quotient is kept outside `Decimal`, whose 28
/// decimal places would cut a digit of a small quotient (0.03…).
pub(crate) fn py_div_mul(a: Decimal, divisor: u64, factor: u64) -> Decimal {
    if divisor == 0 || a.is_zero() {
        return Decimal::ZERO;
    }
    let divisor = u128::from(divisor);
    let numerator = a.mantissa().unsigned_abs();
    let ideal_exp = -i64::from(a.scale());
    let mut exp = ideal_exp;
    let mut digits = numerator / divisor;
    let mut rest = numerator % divisor;
    while digit_count(digits) < 29 && !(rest == 0 && digits > 0) {
        rest *= 10;
        digits = digits * 10 + rest / divisor;
        rest %= divisor;
        exp -= 1;
    }
    let exact = rest == 0;
    let (quotient, exp) = round_significant(digits, exp, !exact);
    let (mut digits, mut exp) = round_significant(quotient * u128::from(factor), exp, false);
    if exact {
        while exp < ideal_exp && digits % 10 == 0 && digits > 0 {
            digits /= 10;
            exp += 1;
        }
    }
    let magnitude = if exp >= 0 {
        Decimal::from_i128_with_scale(digits as i128, 0) * Decimal::from(10u64.pow(exp as u32))
    } else {
        let scale = (-exp) as u32;
        if scale > 28 {
            Decimal::from_i128_with_scale((digits / 10u128.pow(scale - 28)) as i128, 28)
        } else {
            Decimal::from_i128_with_scale(digits as i128, scale)
        }
    };
    if a.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// Python's `statistics.median` over decimals: the middle value, or the
/// mean of the two middle ones.
pub(crate) fn median(values: &[Decimal]) -> Option<Decimal> {
    let mut sorted = values.to_vec();
    sorted.sort();
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some(py_div(
            py_round(sorted[n / 2 - 1] + sorted[n / 2]),
            Decimal::TWO,
        )),
    }
}

/// `d` quantized to cents (half-even) and shown with two decimals.
pub(crate) fn cents(d: Decimal) -> Decimal {
    let mut rounded = d.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
    rounded.rescale(2);
    rounded
}

fn local_day(t: &Tx, zone: &TimeZone) -> Date {
    t.occurred_at.with_time_zone(zone.clone()).date()
}

/// Six calendar months (the current one partial) of spending and refunds,
/// per currency. `excluded_ids` (fixed spending, for the variable view) are
/// left out of the sums but still mark a month as observed.
pub fn time_trends(
    transactions: &[Tx],
    mappings: &[GroupMapping],
    now: &Zoned,
    excluded_ids: &HashSet<String>,
) -> TimeTrends {
    let zone = now.time_zone().clone();
    let today = now.date();
    let starts: Vec<Date> = (-5..=0)
        .map(|o| shift_month(month_start(today), o))
        .collect();
    let first = starts[0];
    let current_start = starts[5];
    let reviewed = group_index(mappings);
    let mut grouped: BTreeMap<Currency, Vec<&Tx>> = BTreeMap::new();
    let mut income: HashMap<Currency, Vec<&Tx>> = HashMap::new();
    for t in transactions {
        if t.occurred_at.timestamp() > now.timestamp() {
            continue;
        }
        let on = local_day(t, &zone);
        if t.kind == Kind::Inflow && current_start <= on && on <= today {
            income.entry(t.currency.clone()).or_default().push(t);
        }
        if matches!(t.kind, Kind::Outflow | Kind::Refund) && first <= on && on <= today {
            grouped.entry(t.currency.clone()).or_default().push(t);
        }
    }
    let currencies = grouped
        .into_iter()
        .map(|(currency, rows)| {
            let incoming = income.remove(&currency).unwrap_or_default();
            currency_trends(
                currency,
                &rows,
                &incoming,
                &reviewed,
                &starts,
                today,
                &zone,
                excluded_ids,
            )
        })
        .collect();
    TimeTrends {
        start: first.to_string(),
        through: today.to_string(),
        timezone: zone.iana_name().unwrap_or("UTC").to_owned(),
        currencies,
        time_of_day_available: false,
    }
}

/// Month-keyed rows of one group or category, in first-seen key order.
struct Buckets<K> {
    order: Vec<K>,
    rows: HashMap<K, HashMap<Date, Vec<Tx>>>,
}

impl<K: std::hash::Hash + Eq + Clone> Buckets<K> {
    fn new() -> Self {
        Self {
            order: Vec::new(),
            rows: HashMap::new(),
        }
    }

    fn push(&mut self, key: K, month: Date, t: &Tx) {
        if !self.rows.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.rows
            .entry(key)
            .or_default()
            .entry(month)
            .or_default()
            .push(t.clone());
    }
}

#[allow(clippy::too_many_arguments)]
fn currency_trends(
    currency: Currency,
    rows: &[&Tx],
    incoming: &[&Tx],
    reviewed: &HashMap<&str, Option<&str>>,
    starts: &[Date],
    today: Date,
    zone: &TimeZone,
    excluded_ids: &HashSet<String>,
) -> CurrencyTrends {
    let mut observed: HashMap<Date, Vec<&Tx>> = HashMap::new();
    for t in rows {
        observed
            .entry(month_start(local_day(t, zone)))
            .or_default()
            .push(t);
    }
    let mut by_month: HashMap<Date, Vec<&Tx>> = HashMap::new();
    let mut by_group: Buckets<Option<String>> = Buckets::new();
    let mut by_category: Buckets<Option<String>> = Buckets::new();
    let mut by_weekday: [Vec<&Tx>; 7] = Default::default();
    for t in rows
        .iter()
        .copied()
        .filter(|t| !excluded_ids.contains(&t.transaction_id))
    {
        let on = local_day(t, zone);
        let start = month_start(on);
        by_month.entry(start).or_default().push(t);
        by_category.push(t.category.clone(), start, t);
        let group = t
            .category
            .as_deref()
            .and_then(|c| reviewed.get(c).copied().flatten())
            .map(str::to_owned);
        by_group.push(group, start, t);
        by_weekday[on.weekday().to_monday_zero_offset() as usize].push(t);
    }
    let is_observed = |start: &Date| observed.get(start).is_some_and(|r| !r.is_empty());
    let month_rows = |start: &Date| by_month.get(start).cloned().unwrap_or_default();
    let current_start = starts[5];
    let months = starts
        .iter()
        .map(|start| MonthSpending {
            start: start.to_string(),
            through: if *start == current_start {
                today
            } else {
                start.last_of_month()
            }
            .to_string(),
            partial_month: *start == current_start,
            spending: is_observed(start).then(|| net(&month_rows(start))),
        })
        .collect();

    let mut group_keys = by_group.order.clone();
    group_keys.sort_by(|a, b| a.as_deref().unwrap_or("").cmp(b.as_deref().unwrap_or("")));
    let groups = group_keys
        .into_iter()
        .map(|group| {
            let trend = trend(&by_group.rows[&group], starts, &is_observed, today);
            GroupTrend {
                group,
                values: trend.values,
                total: trend.total,
                projected_month_end: trend.projected_month_end,
                historical_median: trend.historical_median,
                history_month_count: trend.history_month_count,
                purchase_count: trend.purchase_count,
            }
        })
        .collect();
    let mut category_keys = by_category.order.clone();
    category_keys.sort_by_key(|c| (c.is_none(), casefold(c.as_deref().unwrap_or(""))));
    let categories = category_keys
        .into_iter()
        .map(|category| {
            let trend = trend(&by_category.rows[&category], starts, &is_observed, today);
            CategoryTrend { category, ..trend }
        })
        .collect();

    let previous_start = starts[4];
    let previous_observed = observed
        .get(&previous_start)
        .is_some_and(|r| r.iter().any(|t| local_day(t, zone).day() <= today.day()));
    let daily = daily(
        &month_rows(&current_start),
        &month_rows(&previous_start),
        today,
        zone,
        is_observed(&current_start),
        previous_observed,
    );
    let weekdays = WEEKDAYS
        .iter()
        .zip(&by_weekday)
        .map(|(label, rows)| weekday(label, rows, zone))
        .collect();
    let weekday_rows: Vec<&Tx> = by_weekday[..5].iter().flatten().copied().collect();
    let weekend_rows: Vec<&Tx> = by_weekday[5..].iter().flatten().copied().collect();
    CurrencyTrends {
        currency,
        income: incoming.iter().map(|t| t.amount).sum(),
        income_transaction_count: incoming.len(),
        months,
        groups,
        categories,
        daily,
        weekdays,
        day_types: vec![
            weekday("Weekdays", &weekday_rows, zone),
            weekday("Weekend", &weekend_rows, zone),
        ],
    }
}

/// One group's or category's history (Python `_category_trend`).
fn trend(
    history: &HashMap<Date, Vec<Tx>>,
    starts: &[Date],
    is_observed: &dyn Fn(&Date) -> bool,
    today: Date,
) -> CategoryTrend {
    let month = |start: &Date| -> Vec<&Tx> {
        history
            .get(start)
            .map(|rows| rows.iter().collect())
            .unwrap_or_default()
    };
    let previous: Vec<Decimal> = starts[..starts.len() - 1]
        .iter()
        .map(&month)
        .filter(|rows| !rows.is_empty())
        .map(|rows| net(&rows))
        .collect();
    let current = month(&starts[starts.len() - 1]);
    let all: Vec<&Tx> = history.values().flatten().collect();
    CategoryTrend {
        category: None,
        values: starts
            .iter()
            .map(|s| is_observed(s).then(|| net(&month(s))))
            .collect(),
        total: net(&all),
        projected_month_end: py_div_mul(
            net(&current),
            u64::from(today.day().unsigned_abs()),
            u64::from(today.days_in_month().unsigned_abs()),
        ),
        historical_median: if previous.len() >= 2 {
            median(&previous)
        } else {
            None
        },
        history_month_count: previous.len(),
        purchase_count: current.iter().filter(|t| t.kind == Kind::Outflow).count(),
    }
}

/// Cumulative spending per day of month through today, against the same
/// days of the previous month.
fn daily(
    current: &[&Tx],
    previous: &[&Tx],
    today: Date,
    zone: &TimeZone,
    current_observed: bool,
    previous_observed: bool,
) -> Vec<DailySpending> {
    let prior_start = shift_month(month_start(today), -1);
    let prior_days = prior_start.days_in_month();
    let mut totals = [[Decimal::ZERO; 32]; 2];
    for t in current {
        totals[0][local_day(t, zone).day() as usize] += signed(t);
    }
    for t in previous {
        let day = local_day(t, zone).day();
        if day <= today.day().min(prior_days) {
            totals[1][day as usize] += signed(t);
        }
    }
    let (mut current_sum, mut previous_sum) = (Decimal::ZERO, Decimal::ZERO);
    (1..=today.day())
        .map(|day| {
            current_sum += totals[0][day as usize];
            previous_sum += totals[1][day as usize];
            let in_prior = day <= prior_days;
            DailySpending {
                day: day as u8,
                current_on: today.with().day(day).build().unwrap_or(today).to_string(),
                previous_on: in_prior.then(|| {
                    prior_start
                        .with()
                        .day(day)
                        .build()
                        .unwrap_or(prior_start)
                        .to_string()
                }),
                current: current_observed.then_some(current_sum),
                previous: (previous_observed && in_prior).then_some(previous_sum),
            }
        })
        .collect()
}

fn weekday(label: &str, rows: &[&Tx], zone: &TimeZone) -> WeekdaySpending {
    let purchases: Vec<&Tx> = rows
        .iter()
        .copied()
        .filter(|t| t.kind == Kind::Outflow)
        .collect();
    let days: HashSet<Date> = purchases.iter().map(|t| local_day(t, zone)).collect();
    let gross: Decimal = purchases.iter().map(|t| t.amount).sum();
    WeekdaySpending {
        label: label.to_owned(),
        spending: (!rows.is_empty()).then(|| net(rows)),
        gross,
        refunds: gross - net(rows),
        purchase_count: purchases.len(),
        purchase_days: days.len(),
        average_purchase_day: (!days.is_empty())
            .then(|| py_div(gross, Decimal::from(days.len() as u64))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn tx(id: &str, amount: &str, kind: Kind, category: Option<&str>, at: &str) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 0,
            account_id: "firefly:1".into(),
            counterparty_account_id: (kind == Kind::Transfer).then(|| "firefly:2".into()),
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: at.parse().unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: category.map(str::to_owned),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    pub(crate) fn out(id: &str, amount: &str, category: Option<&str>, at: &str) -> Tx {
        tx(id, amount, Kind::Outflow, category, at)
    }

    pub(crate) fn mapping(category: &str, group: &str) -> GroupMapping {
        GroupMapping {
            category: category.into(),
            group: Some(group.into()),
            revision: 0,
        }
    }

    const NOW: &str = "2026-09-10T12:00:00+00:00[UTC]";

    fn now() -> Zoned {
        NOW.parse().unwrap()
    }

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn six_months_refunds_unknowns_and_groups() {
        let now: Zoned = "2026-09-10T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        let at = "2026-09-10T14:00:00+02:00[Europe/Amsterdam]";
        let mut usd = out("usd", "4", Some("Synthetic shop"), at);
        usd.currency = Currency::new("USD");
        let rows = vec![
            out("a", "30", Some("Synthetic shop"), at),
            tx("b", "5", Kind::Refund, Some("Synthetic shop"), at),
            out("c", "7", None, at),
            out(
                "prior",
                "40",
                Some("Synthetic shop"),
                "2026-08-10T14:00:00+02:00[Europe/Amsterdam]",
            ),
            out(
                "old",
                "999",
                Some("Synthetic shop"),
                "2026-03-10T14:00:00+01:00[Europe/Amsterdam]",
            ),
            out(
                "future",
                "999",
                Some("Synthetic shop"),
                "2026-09-10T15:00:00+02:00[Europe/Amsterdam]",
            ),
            tx("income", "999", Kind::Inflow, None, at),
            tx("transfer", "999", Kind::Transfer, None, at),
            usd,
        ];
        let report = time_trends(
            &rows,
            &[mapping("Synthetic shop", "Groceries")],
            &now,
            &HashSet::new(),
        );
        assert_eq!(report.start, "2026-04-01");
        assert_eq!(report.timezone, "Europe/Amsterdam");
        let (euro, dollar) = (&report.currencies[0], &report.currencies[1]);
        let spending: Vec<_> = euro.months.iter().map(|m| m.spending).collect();
        assert_eq!(
            spending,
            [None, None, None, None, Some(dec("40")), Some(dec("32"))]
        );
        assert!(euro.months[5].partial_month && !euro.months[4].partial_month);
        let groceries = euro
            .groups
            .iter()
            .find(|g| g.group.as_deref() == Some("Groceries"))
            .unwrap();
        assert_eq!(groceries.values[5], Some(dec("25")));
        assert_eq!(euro.groups[0].group, None);
        assert_eq!(dollar.months[5].spending, Some(dec("4")));
        assert_eq!(dollar.months[4].spending, None);
        let unknown = euro.categories.last().unwrap();
        assert_eq!(unknown.category, None);
        assert_eq!(unknown.values[4..], [Some(Decimal::ZERO), Some(dec("7"))]);
        assert_eq!(euro.categories[0].total, dec("65"));
        let last = euro.daily.last().unwrap();
        assert_eq!(
            (last.current, last.previous),
            (Some(dec("32")), Some(dec("40")))
        );
        assert_eq!(
            (euro.income, euro.income_transaction_count),
            (dec("999"), 1)
        );
    }

    #[test]
    fn zero_net_is_not_missing_and_daily_can_fall() {
        let rows = vec![
            out("a", "10", Some("x"), "2026-09-01T12:00:00+00:00[UTC]"),
            tx(
                "b",
                "10",
                Kind::Refund,
                Some("x"),
                "2026-09-03T12:00:00+00:00[UTC]",
            ),
        ];
        let report = time_trends(&rows, &[], &now(), &HashSet::new());
        let item = &report.currencies[0];
        assert_eq!(item.months[5].spending, Some(Decimal::ZERO));
        assert_eq!(item.months[4].spending, None);
        assert_eq!(item.categories[0].values[4..], [None, Some(Decimal::ZERO)]);
        let current: Vec<_> = item.daily[..3].iter().map(|p| p.current).collect();
        assert_eq!(
            current,
            [Some(dec("10")), Some(dec("10")), Some(Decimal::ZERO)]
        );
        assert!(item.daily.iter().all(|p| p.previous.is_none()));
        assert!(
            time_trends(&[], &[], &now(), &HashSet::new())
                .currencies
                .is_empty()
        );
    }

    #[test]
    fn leap_short_month_and_timezone_boundaries() {
        let now: Zoned = "2024-03-31T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        let rows = vec![
            out(
                "march",
                "3",
                None,
                "2024-03-01T00:30:00+01:00[Europe/Amsterdam]",
            ),
            out(
                "feb",
                "8",
                None,
                "2024-02-29T13:00:00+01:00[Europe/Amsterdam]",
            ),
        ];
        let report = time_trends(&rows, &[], &now, &HashSet::new());
        assert_eq!(report.start, "2023-10-01");
        let points = &report.currencies[0].daily;
        assert_eq!(points[0].current, Some(dec("3")));
        assert_eq!(points[28].previous_on.as_deref(), Some("2024-02-29"));
        assert_eq!(points[28].previous, Some(dec("8")));
        assert_eq!(
            (points[29].previous_on.as_ref(), points[29].previous),
            (None, None)
        );
        assert_eq!(points[30].current, Some(dec("3")));
    }

    #[test]
    fn weekday_patterns_use_active_purchase_dates() {
        let rows = vec![
            out("a", "10", None, "2026-09-05T12:00:00+00:00[UTC]"),
            out("b", "20", None, "2026-09-05T13:00:00+00:00[UTC]"),
            tx(
                "c",
                "4",
                Kind::Refund,
                None,
                "2026-09-05T14:00:00+00:00[UTC]",
            ),
            out("d", "9", None, "2026-09-06T12:00:00+00:00[UTC]"),
            out("e", "7", None, "2026-09-07T12:00:00+00:00[UTC]"),
        ];
        let item = &time_trends(&rows, &[], &now(), &HashSet::new()).currencies[0];
        let saturday = &item.weekdays[5];
        assert_eq!(
            (saturday.gross, saturday.spending, saturday.refunds),
            (dec("30"), Some(dec("26")), dec("4"))
        );
        assert_eq!((saturday.purchase_count, saturday.purchase_days), (2, 1));
        assert_eq!(saturday.average_purchase_day, Some(dec("30")));
        assert_eq!(item.weekdays[1].spending, None);
        assert_eq!(item.day_types[0].gross, dec("7"));
        assert_eq!(item.day_types[1].average_purchase_day, Some(dec("19.5")));
    }

    #[test]
    fn previous_later_records_do_not_create_false_zero_comparison() {
        let rows = vec![
            out("old", "10", None, "2026-08-20T12:00:00+00:00[UTC]"),
            out("current", "10", None, NOW),
        ];
        let report = time_trends(&rows, &[], &now(), &HashSet::new());
        assert_eq!(report.currencies[0].months[4].spending, Some(dec("10")));
        assert!(
            report.currencies[0]
                .daily
                .iter()
                .all(|p| p.previous.is_none())
        );
    }

    #[test]
    fn category_pace_uses_net_spending_and_active_history() {
        let rows = vec![
            out("july", "20", None, "2026-07-10T12:00:00+00:00[UTC]"),
            out("aug", "40", None, "2026-08-10T12:00:00+00:00[UTC]"),
            out("current", "15", None, NOW),
            tx("refund", "5", Kind::Refund, None, NOW),
        ];
        let item = &time_trends(&rows, &[], &now(), &HashSet::new()).currencies[0].categories[0];
        assert_eq!(item.historical_median, Some(dec("30")));
        assert_eq!(item.history_month_count, 2);
        assert_eq!(item.projected_month_end, dec("30"));
        assert_eq!(item.purchase_count, 1);
    }

    #[test]
    fn variable_view_excludes_fixed_rows_but_keeps_known_zero_months() {
        let rows = vec![
            out("fixed-old", "800", None, "2026-08-10T12:00:00+00:00[UTC]"),
            out("fixed", "800", None, NOW),
            out("coffee", "20", Some("Coffee"), NOW),
            tx("refund", "5", Kind::Refund, Some("Coffee"), NOW),
        ];
        let excluded = HashSet::from(["fixed-old".to_owned(), "fixed".to_owned()]);
        let report = &time_trends(&rows, &[], &now(), &excluded).currencies[0];
        assert_eq!(report.months[4].spending, Some(Decimal::ZERO));
        assert_eq!(report.months[5].spending, Some(dec("15")));
        let last = report.daily.last().unwrap();
        assert_eq!(
            (last.current, last.previous),
            (Some(dec("15")), Some(Decimal::ZERO))
        );
        assert_eq!(report.categories[0].category.as_deref(), Some("Coffee"));
        assert_eq!(report.months[0].spending, None);
    }

    #[test]
    fn group_combines_categories_and_recomputes_history() {
        let rows = vec![
            out("jumbo", "20", Some("Jumbo"), NOW),
            out("ah", "10", Some("Albert Heijn"), NOW),
            out("aug", "40", Some("Jumbo"), "2026-08-10T12:00:00+00:00[UTC]"),
            out(
                "july",
                "20",
                Some("Albert Heijn"),
                "2026-07-10T12:00:00+00:00[UTC]",
            ),
        ];
        let mappings = [
            mapping("Jumbo", "Groceries"),
            mapping("Albert Heijn", "Groceries"),
        ];
        let group = &time_trends(&rows, &mappings, &now(), &HashSet::new()).currencies[0].groups[0];
        assert_eq!(group.values[5], Some(dec("30")));
        assert_eq!(group.projected_month_end, dec("90"));
        assert_eq!(group.historical_median, Some(dec("30")));
        assert_eq!(group.purchase_count, 2);
    }

    #[test]
    fn python_decimal_arithmetic() {
        // 735.48 / 26 * 31 in Python's 28-digit context.
        assert_eq!(
            py_div_mul(dec("735.48"), 26, 31),
            dec("876.9184615384615384615384616")
        );
        assert_eq!(
            py_div_mul(dec("0.88"), 26, 31),
            dec("1.049230769230769230769230769")
        );
        assert_eq!(py_div_mul(dec("-150"), 10, 30).to_string(), "-450");
        assert_eq!(
            py_div_mul(dec("1"), 3, 3),
            dec("0.9999999999999999999999999999")
        );
        assert_eq!(
            median(&[dec("8.35"), dec("4.30"), dec("20"), dec("4.30")]),
            Some(dec("6.325"))
        );
        assert_eq!(cents(dec("2.345")).to_string(), "2.34");
        assert_eq!(cents(dec("0.9")).to_string(), "0.90");
    }
}
