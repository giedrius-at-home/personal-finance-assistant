//! Plots over the owner's transactions: a `PlotSpec` in, the series a chart
//! draws out. The rows are the spending every other screen counts
//! (`budget::spending_transactions`), so a plot agrees with them.

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::plot::{
    Axis, Filters, Measure, MeasureSpec, Period, PeriodBasis, PlotData, PlotSeries, PlotSpec,
    Split, Unit,
};
use finance_types::report::Currency;
use jiff::Span;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::text::casefold;
use crate::trends::{cents, local_day, time_trends};
use crate::writes::review::normalize_merchant;
use crate::{Ctx, Error, Result, budget, groups, plans};

const OTHER: &str = "Other";
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A counted transaction, with the day it counts on and its merchant key.
struct Row<'a> {
    t: &'a Tx,
    day: Date,
    merchant: String,
}

/// What an axis or a split groups rows by.
#[derive(Clone, Copy)]
enum Dim {
    Time(Axis),
    Weekday,
    DayOfMonth,
    Year,
    Category,
    Group,
    Merchant,
    Account,
    FixedOrVariable,
}

impl From<Axis> for Dim {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Weekday => Self::Weekday,
            Axis::DayOfMonth => Self::DayOfMonth,
            Axis::Category => Self::Category,
            Axis::Group => Self::Group,
            Axis::Merchant => Self::Merchant,
            Axis::Account => Self::Account,
            Axis::FixedOrVariable => Self::FixedOrVariable,
            time => Self::Time(time),
        }
    }
}

impl From<Split> for Dim {
    fn from(split: Split) -> Self {
        match split {
            Split::Category => Self::Category,
            Split::Group => Self::Group,
            Split::Merchant => Self::Merchant,
            Split::Account => Self::Account,
            Split::Weekday => Self::Weekday,
            Split::Year => Self::Year,
            Split::FixedOrVariable => Self::FixedOrVariable,
        }
    }
}

/// The lookups that turn a row into a key and a key into a label.
struct Names<'a> {
    groups: HashMap<&'a str, Option<&'a str>>,
    accounts: HashMap<&'a str, &'a str>,
    /// Merchant key → its most frequent spelling.
    merchants: HashMap<String, String>,
    /// Not variable spending, as the dashboard's variable trends leave out.
    fixed: HashSet<String>,
}

impl Names<'_> {
    fn key(&self, dim: Dim, r: &Row) -> String {
        match dim {
            Dim::Time(axis) => bucket_start(axis, r.day).to_string(),
            Dim::Weekday => r.day.weekday().to_monday_zero_offset().to_string(),
            Dim::DayOfMonth => r.day.day().to_string(),
            Dim::Year => r.day.year().to_string(),
            Dim::Category => r.t.category.as_deref().unwrap_or("Uncategorised").into(),
            Dim::Group => match r.t.category.as_deref() {
                None => "Uncategorised".into(),
                Some(c) => self
                    .groups
                    .get(c)
                    .copied()
                    .flatten()
                    .unwrap_or("No group")
                    .into(),
            },
            Dim::Merchant => r.merchant.clone(),
            Dim::Account => r.t.account_id.clone(),
            Dim::FixedOrVariable => if self.fixed.contains(&r.t.transaction_id) {
                "Fixed"
            } else {
                "Variable"
            }
            .into(),
        }
    }

    fn label(&self, dim: Dim, key: &str) -> String {
        match dim {
            Dim::Merchant if key.is_empty() => "Unknown".into(),
            Dim::Merchant => self.merchants.get(key).map_or(key, String::as_str).into(),
            Dim::Account => self.accounts.get(key).copied().unwrap_or(key).into(),
            Dim::Weekday => key
                .parse::<usize>()
                .ok()
                .and_then(|i| WEEKDAYS.get(i))
                .map_or(key, |w| w)
                .into(),
            _ => key.into(),
        }
    }

    /// Every given filter matches; one naming nothing that exists matches nothing.
    fn passes(&self, f: &Filters, r: &Row) -> bool {
        let merchant = casefold(f.merchant.trim());
        (f.categories.is_empty() || f.categories.contains(&self.key(Dim::Category, r)))
            && (f.groups.is_empty() || f.groups.contains(&self.key(Dim::Group, r)))
            && (f.accounts.is_empty() || f.accounts.contains(&r.t.account_id))
            && (merchant.is_empty() || casefold(&r.merchant).contains(&merchant))
            && f.min.is_none_or(|min| r.t.amount >= min)
            && f.max.is_none_or(|max| r.t.amount <= max)
            && !(f.variable_only && self.fixed.contains(&r.t.transaction_id))
    }
}

/// One entry along the axis: its rows are those whose axis key is in `keys`.
struct Column {
    label: String,
    title: String,
    keys: Vec<String>,
}

impl Column {
    fn named(label: String, keys: Vec<String>) -> Self {
        Self {
            title: label.clone(),
            label,
            keys,
        }
    }
}

pub fn plot_query(ctx: &Ctx<'_>, spec: &PlotSpec) -> Result<PlotData> {
    spec.validate().map_err(Error::Rejected)?;
    let snap = ctx.snap;
    let payments = plans::planned_payment_views(ctx);
    let spending = budget::spending_transactions(ctx, &payments);
    let mut fixed_categories =
        groups::fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    fixed_categories.extend(groups::predicted_category_names(
        &snap.settings.predicted_bill_categories,
    ));
    let currency = match &spec.currency {
        Some(code) => Currency::new(code),
        // The main currency, as the pages pick it.
        None => time_trends(&spending, &snap.group_mappings, &ctx.now, &HashSet::new())
            .currencies
            .into_iter()
            .next()
            .map_or_else(Currency::eur, |c| c.currency),
    };
    let zone = ctx.now.time_zone();
    let counted: Vec<Row> = spending
        .iter()
        .filter(|t| t.currency == currency && t.kind != Kind::Transfer)
        .filter(|t| t.occurred_at.timestamp() <= ctx.now.timestamp())
        .map(|t| Row {
            t,
            day: local_day(t, zone),
            merchant: t
                .counterparty_name
                .as_deref()
                .map(normalize_merchant)
                .unwrap_or_default(),
        })
        .collect();
    let names = Names {
        groups: groups::group_index(&snap.group_mappings),
        accounts: snap
            .accounts
            .iter()
            .map(|a| (a.account_id.as_str(), a.name.as_str()))
            .collect(),
        merchants: spellings(&counted),
        fixed: groups::non_variable_ids(&snap.transactions, &fixed_categories),
    };
    let filtered: Vec<&Row> = counted
        .iter()
        .filter(|r| names.passes(&spec.filters, r))
        .collect();
    let today = ctx.today();
    let (from, to) = period_days(&spec.period, today, &filtered)?;
    let rows: Vec<&Row> = filtered
        .into_iter()
        .filter(|r| from <= r.day && r.day <= to)
        .collect();

    let first = spec.measures[0].measure;
    let axis = Dim::from(spec.by);
    let mut left_out = 0;
    let mut starts = Vec::new();
    let columns: Vec<Column> = match spec.by {
        Axis::Weekday => WEEKDAYS
            .iter()
            .enumerate()
            .map(|(i, w)| Column {
                label: w[..3].into(),
                title: (*w).into(),
                keys: vec![i.to_string()],
            })
            .collect(),
        Axis::DayOfMonth => (1..=31)
            .map(|d| Column {
                label: d.to_string(),
                title: format!("Day {d}"),
                keys: vec![d.to_string()],
            })
            .collect(),
        _ if spec.time_axis() => {
            starts = buckets(spec.by, from, to)?;
            let years = from.year() != to.year();
            starts
                .iter()
                .map(|&d| bucket_column(spec.by, d, years))
                .collect()
        }
        _ => {
            let mut kept = ranked(&names, axis, &rows, first);
            let rest = kept.split_off(kept.len().min(usize::from(spec.top)));
            left_out += rest.len();
            let mut columns: Vec<Column> = kept
                .into_iter()
                .map(|k| Column::named(names.label(axis, &k), vec![k]))
                .collect();
            if !rest.is_empty() {
                columns.push(Column::named(OTHER.into(), rest));
            }
            columns
        }
    };
    let index: HashMap<&str, usize> = columns
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.keys.iter().map(move |k| (k.as_str(), i)))
        .collect();
    let mut cells: Vec<Vec<&Row>> = vec![Vec::new(); columns.len()];
    for r in &rows {
        if let Some(&i) = index.get(names.key(axis, r).as_str()) {
            cells[i].push(r);
        }
    }
    let unit = |m: Measure| {
        if m == Measure::Count {
            Unit::Count
        } else {
            Unit::Money
        }
    };
    // `keys`: the split values a series holds (all rows when `None`).
    let series_of =
        |m: &MeasureSpec, key: String, label: String, keys: Option<(Dim, HashSet<String>)>| {
            PlotSeries {
                key,
                label,
                draw: m.draw,
                values: cells
                    .iter()
                    .map(|cell| {
                        let txs: Vec<&Tx> = cell
                            .iter()
                            .filter(|r| {
                                keys.as_ref()
                                    .is_none_or(|(dim, set)| set.contains(&names.key(*dim, r)))
                            })
                            .map(|r| r.t)
                            .collect();
                        measure(m.measure, &txs)
                    })
                    .collect(),
                reference: false,
                unit: unit(m.measure),
            }
        };
    let series = if counted.is_empty() {
        Vec::new()
    } else if let Some(split) = spec.split {
        let dim = Dim::from(split);
        let mut kept = ranked(&names, dim, &rows, first);
        // At most six series, "Other" included.
        let keep = usize::from(spec.split_top);
        let rest = kept.split_off(if kept.len() > keep {
            keep.min(5)
        } else {
            keep.min(kept.len())
        });
        left_out += rest.len();
        let mut series: Vec<PlotSeries> = kept
            .into_iter()
            .map(|k| {
                let label = names.label(dim, &k);
                series_of(
                    &spec.measures[0],
                    k.clone(),
                    label,
                    Some((dim, HashSet::from([k]))),
                )
            })
            .collect();
        if !rest.is_empty() {
            series.push(series_of(
                &spec.measures[0],
                "other".into(),
                OTHER.into(),
                Some((dim, rest.into_iter().collect())),
            ));
        }
        series
    } else {
        spec.measures
            .iter()
            .map(|m| {
                let (key, label) = measure_name(m.measure);
                series_of(m, key.into(), label.into(), None)
            })
            .collect()
    };
    let partial_last = starts.last().is_some_and(|&start| {
        start <= today && today <= to && next_bucket(spec.by, start).is_none_or(|n| today < n)
    });
    // Task 3 adds here, on this plain result: budget months (`period_basis`),
    // the running total, compare, the average line, `Value::Change`,
    // `Measure::AveragePerDay` (`measure` gives `None` for it now) and the insight.
    Ok(PlotData {
        categories: columns.iter().map(|c| c.label.clone()).collect(),
        titles: columns.into_iter().map(|c| c.title).collect(),
        series,
        partial_last,
        insight: None,
        currency,
        unit: if spec.measures.iter().all(|m| m.measure == Measure::Count) {
            Unit::Count
        } else {
            Unit::Money
        },
        subtitle: subtitle(spec),
        left_out,
    })
}

/// One measure over a bucket's rows; `None` where it has no value.
fn measure(m: Measure, txs: &[&Tx]) -> Option<Decimal> {
    let spending: Vec<&Tx> = txs
        .iter()
        .copied()
        .filter(|t| t.kind != Kind::Inflow)
        .collect();
    let spent = groups::net(&spending);
    let income: Decimal = txs
        .iter()
        .filter(|t| t.kind == Kind::Inflow)
        .map(|t| t.amount)
        .sum();
    let outflows = || {
        txs.iter()
            .filter(|t| t.kind == Kind::Outflow)
            .map(|t| t.amount)
    };
    let count = outflows().count();
    match m {
        Measure::Spent => Some(spent),
        Measure::Income => Some(income),
        Measure::Net => Some(income - spent),
        Measure::Count => Some(Decimal::from(count)),
        Measure::Average => (count > 0).then(|| cents(spent / Decimal::from(count))),
        Measure::Largest => outflows().max(),
        Measure::AveragePerDay => None,
    }
}

fn measure_name(m: Measure) -> (&'static str, &'static str) {
    match m {
        Measure::Spent => ("spent", "Spent"),
        Measure::Income => ("income", "Income"),
        Measure::Net => ("net", "Net"),
        Measure::Count => ("count", "Purchases"),
        Measure::Average => ("average", "Average purchase"),
        Measure::Largest => ("largest", "Largest purchase"),
        Measure::AveragePerDay => ("average_per_day", "Average per day"),
    }
}

/// The keys of `dim` among `rows`, by `m` over each key's rows, largest first.
fn ranked(names: &Names, dim: Dim, rows: &[&Row], m: Measure) -> Vec<String> {
    let mut by_key: BTreeMap<String, Vec<&Tx>> = BTreeMap::new();
    for r in rows {
        by_key.entry(names.key(dim, r)).or_default().push(r.t);
    }
    let mut ranked: Vec<(String, Option<Decimal>)> = by_key
        .into_iter()
        .map(|(k, txs)| {
            let value = measure(m, &txs);
            (k, value)
        })
        .collect();
    // Stable: equal values keep key order; `None` ranks last.
    ranked.sort_by_key(|r| std::cmp::Reverse(r.1));
    ranked.into_iter().map(|(k, _)| k).collect()
}

/// Each merchant key's most frequent original spelling (ties: the first in order).
fn spellings(rows: &[Row]) -> HashMap<String, String> {
    let mut counts: HashMap<&str, BTreeMap<&str, usize>> = HashMap::new();
    for r in rows {
        if let Some(name) = r.t.counterparty_name.as_deref() {
            *counts
                .entry(&r.merchant)
                .or_default()
                .entry(name)
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(key, names)| {
            let best = names
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))?;
            Some((key.to_owned(), best.0.to_owned()))
        })
        .collect()
}

/// The first and last day of `period`, both included (calendar months).
fn period_days(period: &Period, today: Date, rows: &[&Row]) -> Result<(Date, Date)> {
    let month = today.first_of_month();
    let months = |n: i32| (shift_month(month, 1 - n), today);
    Ok(match period {
        Period::ThisMonth => (month, today),
        Period::LastMonth => {
            let start = shift_month(month, -1);
            (start, start.last_of_month())
        }
        Period::Last3Months => months(3),
        Period::Last6Months => months(6),
        Period::Last12Months => months(12),
        Period::ThisYear => (today.first_of_year(), today),
        Period::AllTime => (rows.iter().map(|r| r.day).min().unwrap_or(today), today),
        Period::Between { from, to } => {
            let day = |s: &str| {
                s.parse::<Date>().map_err(|_| {
                    Error::Rejected("Dates must be real days written YYYY-MM-DD.".into())
                })
            };
            (day(from)?, day(to)?)
        }
    })
}

fn bucket_start(axis: Axis, day: Date) -> Date {
    match axis {
        Axis::Week => day
            .checked_sub(Span::new().days(day.weekday().to_monday_zero_offset()))
            .unwrap_or(day),
        Axis::Month => day.first_of_month(),
        Axis::Quarter => Date::new(day.year(), (day.month() - 1) / 3 * 3 + 1, 1).unwrap_or(day),
        Axis::Year => day.first_of_year(),
        _ => day,
    }
}

/// The start of the bucket after the one starting at `start`; `None` past the calendar's end.
fn next_bucket(axis: Axis, start: Date) -> Option<Date> {
    let next = match axis {
        Axis::Week => start.checked_add(Span::new().days(7)).ok()?,
        Axis::Month => shift_month(start, 1),
        Axis::Quarter => shift_month(start, 3),
        Axis::Year => shift_month(start, 12),
        _ => start.checked_add(Span::new().days(1)).ok()?,
    };
    (next > start).then_some(next)
}

/// Every bucket from `from` through `to`, refused past the limit for its unit.
fn buckets(axis: Axis, from: Date, to: Date) -> Result<Vec<Date>> {
    let (limit, unit) = match axis {
        Axis::Day => (366, "days"),
        Axis::Week => (260, "weeks"),
        Axis::Month => (240, "months"),
        _ => (usize::MAX, ""),
    };
    let mut starts = Vec::new();
    let mut at = Some(bucket_start(axis, from));
    while let Some(start) = at.filter(|&s| s <= to) {
        if starts.len() == limit {
            return Err(Error::Rejected(format!(
                "A plot can show at most {limit} {unit}."
            )));
        }
        starts.push(start);
        at = next_bucket(axis, start);
    }
    Ok(starts)
}

/// A time bucket's short label and long title; `years` when the period spans more than one.
fn bucket_column(axis: Axis, d: Date, years: bool) -> Column {
    let (label, title) = match axis {
        Axis::Day => (
            if years { "%-d %b %Y" } else { "%-d %b" },
            "%A %-d %B %Y".to_owned(),
        ),
        Axis::Week => (
            if years { "%-d %b %Y" } else { "%-d %b" },
            "Week of %-d %B %Y".to_owned(),
        ),
        Axis::Month => (if years { "%b %Y" } else { "%b" }, "%B %Y".to_owned()),
        Axis::Quarter => {
            let q = format!("Q{} %Y", (d.month() - 1) / 3 + 1);
            return Column::named(d.strftime(&q).to_string(), vec![d.to_string()]);
        }
        _ => ("%Y", "%Y".to_owned()),
    };
    Column {
        label: d.strftime(label).to_string(),
        title: d.strftime(&title).to_string(),
        keys: vec![d.to_string()],
    }
}

fn subtitle(spec: &PlotSpec) -> String {
    let period = match &spec.period {
        Period::ThisMonth => "This month".to_owned(),
        Period::LastMonth => "Last month".to_owned(),
        Period::Last3Months => "Last 3 months".to_owned(),
        Period::Last6Months => "Last 6 months".to_owned(),
        Period::Last12Months => "Last 12 months".to_owned(),
        Period::ThisYear => "This year".to_owned(),
        Period::AllTime => "All time".to_owned(),
        Period::Between { from, to } => format!("{from} to {to}"),
    };
    let basis = match spec.period_basis {
        PeriodBasis::Calendar => "calendar months",
        PeriodBasis::Budget => "budget months",
    };
    let scope = if spec.filters.variable_only {
        "variable only"
    } else {
        "all spending"
    };
    format!("{period} · {basis} · {scope}")
}

#[cfg(test)]
#[path = "plot_tests.rs"]
mod tests;
