//! Plots over the owner's transactions: a `PlotSpec` in, the series a chart
//! draws out. The rows are the spending every other screen counts
//! (`budget::spending_transactions`), so a plot agrees with them.

mod buckets;
mod insight;
mod options;

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::plot::{
    Axis, Compare, Draw, Filters, Measure, MeasureSpec, Period, PeriodBasis, PlotData, PlotSeries,
    PlotSpec, Split, Unit, Value,
};
use finance_types::report::Currency;
use jiff::civil::Date;
use rust_decimal::Decimal;

use buckets::point_key;

use crate::model::{Kind, Tx};
use crate::text::casefold;
use crate::trends::{cents, local_day, time_trends};
use crate::writes::review::normalize_merchant;
use crate::{Ctx, Error, Result, budget, groups, plans};

const OTHER: &str = "Other";
/// The "Other" series key; a real split value's key is `SPLIT_KEY` + the value,
/// so the two never collide.
const OTHER_KEY: &str = "other";
const SPLIT_KEY: &str = "split:";
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A counted transaction, with the day and the month (budget or calendar)
/// it counts in, and its merchant key.
struct Row<'a> {
    t: &'a Tx,
    day: Date,
    month: Date,
    merchant: String,
}

/// What an axis or a split groups rows by.
#[derive(Clone, Copy)]
enum Dim {
    Time(Axis),
    Weekday,
    DayOfMonth,
    MonthOfYear,
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
            Axis::MonthOfYear => Self::MonthOfYear,
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
            Dim::Time(_) | Dim::Weekday | Dim::DayOfMonth | Dim::MonthOfYear | Dim::Year => {
                point_key(dim, r.day, r.month)
            }
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
    /// `merchant` is the filter's merchant text, normalised once by the caller.
    fn passes(&self, f: &Filters, merchant: &str, r: &Row) -> bool {
        (f.categories.is_empty() || f.categories.contains(&self.key(Dim::Category, r)))
            && (f.groups.is_empty() || f.groups.contains(&self.key(Dim::Group, r)))
            && (f.accounts.is_empty() || f.accounts.contains(&r.t.account_id))
            && (merchant.is_empty() || casefold(&r.merchant).contains(merchant))
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
    /// The folded rest of a named axis.
    other: bool,
}

impl Column {
    fn named(label: String, keys: Vec<String>) -> Self {
        Self {
            title: label.clone(),
            label,
            keys,
            other: false,
        }
    }
}

/// One series to fill: a measure over all rows, or over the rows whose split
/// value is in `only`.
struct Def {
    m: MeasureSpec,
    key: String,
    label: String,
    only: Option<(Dim, HashSet<String>)>,
}

impl Def {
    fn whole(m: MeasureSpec) -> Self {
        let (key, label) = measure_name(m.measure);
        Self {
            m,
            key: key.into(),
            label: label.into(),
            only: None,
        }
    }
}

/// What every period of one plot shares: the spec, the lookups and the rows
/// that pass the filters, of any date.
struct Frame<'a> {
    ctx: &'a Ctx<'a>,
    spec: &'a PlotSpec,
    names: Names<'a>,
    rows: Vec<&'a Row<'a>>,
    budget: bool,
}

pub fn plot_query(ctx: &Ctx<'_>, spec: &PlotSpec) -> Result<PlotData> {
    spec.validate().map_err(Error::Rejected)?;
    if spec.period == Period::AllTime
        && (spec.compare != Compare::None || spec.value == Value::Change)
    {
        return Err(Error::Rejected(
            "All time has no period before it to compare with.".into(),
        ));
    }
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
    let budget = spec.period_basis == PeriodBasis::Budget;
    let zone = ctx.now.time_zone();
    let counted: Vec<Row> = spending
        .iter()
        .filter(|t| t.currency == currency && t.kind != Kind::Transfer)
        .filter(|t| t.occurred_at.timestamp() <= ctx.now.timestamp())
        .map(|t| {
            let day = local_day(t, zone);
            Row {
                t,
                day,
                month: if budget {
                    ctx.resolver.month_of(t)
                } else {
                    day.first_of_month()
                },
                merchant: t
                    .counterparty_name
                    .as_deref()
                    .map(normalize_merchant)
                    .unwrap_or_default(),
            }
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
    let merchant = casefold(&normalize_merchant(&spec.filters.merchant));
    let rows = counted
        .iter()
        .filter(|r| names.passes(&spec.filters, &merchant, r))
        .collect();
    let frame = Frame {
        ctx,
        spec,
        names,
        rows,
        budget,
    };

    let window = frame.window(0)?;
    let rows = frame.rows_in(&window);
    let (defs, split_left_out) = frame.defs(&rows);
    let ranking = if !spec.named_axis() {
        Vec::new()
    } else if spec.value == Value::Change {
        options::change_ranking(&frame, &window, &rows)?
    } else {
        ranked(
            &frame.names,
            Dim::from(spec.by),
            &rows,
            spec.measures[0].measure,
        )
    };
    let (columns, starts, axis_left_out) = frame.columns(&window, ranking)?;
    let finished = frame.finished(&starts, columns.len());
    let mut values = frame.values(&window, &rows, &columns, &defs);
    if spec.by == Axis::MonthOfYear {
        // This year's months that have not started yet have no value, so its line stops at today.
        let current = frame.current();
        let this_year = current.year().to_string();
        for (def, values) in defs.iter().zip(&mut values) {
            if let Some((Dim::Year, years)) = &def.only
                && years.len() == 1
                && years.contains(&this_year)
            {
                values
                    .iter_mut()
                    .skip(usize::try_from(current.month()).unwrap_or(12))
                    .for_each(|v| *v = None);
            }
        }
    }
    if spec.value == Value::Change {
        options::change(&frame, &columns, &defs, &mut values)?;
    }
    let reference = options::reference(&frame, columns.len())?;
    let unit = |m: Measure| {
        if m == Measure::Count {
            Unit::Count
        } else {
            Unit::Money
        }
    };
    let first_unit = unit(spec.measures[0].measure);
    let insight = if counted.is_empty() {
        None
    } else {
        insight::insight(
            spec,
            &columns,
            &totals(&values, spec.split.is_some()),
            &finished,
            reference.as_ref().map(|r| r.2.as_slice()),
            first_unit,
            &currency,
        )
    };
    let mut series: Vec<PlotSeries> = if counted.is_empty() {
        Vec::new()
    } else {
        defs.into_iter()
            .zip(values)
            .map(|(def, values)| PlotSeries {
                key: def.key,
                label: def.label,
                draw: def.m.draw,
                values,
                reference: false,
                unit: unit(def.m.measure),
            })
            .collect()
    };
    if !series.is_empty() {
        let reference_series = |key: &str, label: &str, values| PlotSeries {
            key: key.into(),
            label: label.into(),
            draw: Draw::Line,
            values,
            reference: true,
            unit: first_unit,
        };
        if spec.cumulative {
            for s in &mut series {
                options::running(&mut s.values);
            }
        }
        if let Some((key, label, mut values)) = reference {
            if spec.cumulative {
                options::running(&mut values);
            }
            series.push(reference_series(key, label, values));
        }
        if spec.average_line {
            let shown: Vec<Vec<Option<Decimal>>> = series
                .iter()
                .filter(|s| !s.reference)
                .map(|s| s.values.clone())
                .collect();
            let average = options::average(&totals(&shown, spec.split.is_some()), &finished);
            series.push(reference_series(
                "average",
                "Average",
                vec![average; columns.len()],
            ));
        }
    }
    Ok(PlotData {
        partial_last: starts.last() == Some(&frame.today_key()) && frame.today() <= window.to,
        categories: columns.iter().map(|c| c.label.clone()).collect(),
        titles: columns.into_iter().map(|c| c.title).collect(),
        series,
        insight,
        currency,
        unit: if spec.measures.iter().all(|m| m.measure == Measure::Count) {
            Unit::Count
        } else {
            Unit::Money
        },
        subtitle: subtitle(spec),
        left_out: axis_left_out + split_left_out,
    })
}

impl Frame<'_> {
    fn today(&self) -> Date {
        self.ctx.today()
    }

    /// The series to fill: one per measure, or one per kept split value plus
    /// "Other"; and how many split values "Other" holds.
    fn defs(&self, rows: &[&Row]) -> (Vec<Def>, usize) {
        let spec = self.spec;
        let Some(split) = spec.split else {
            return (spec.measures.iter().cloned().map(Def::whole).collect(), 0);
        };
        let m = &spec.measures[0];
        let dim = Dim::from(split);
        let (kept, rest) = split_keys(
            split,
            ranked(&self.names, dim, rows, m.measure),
            spec.split_top,
        );
        let left_out = rest.len();
        let mut defs: Vec<Def> = kept
            .into_iter()
            .map(|k| Def {
                m: m.clone(),
                key: format!("{SPLIT_KEY}{k}"),
                label: self.names.label(dim, &k),
                only: Some((dim, HashSet::from([k]))),
            })
            .collect();
        if !rest.is_empty() {
            defs.push(Def {
                m: m.clone(),
                key: OTHER_KEY.into(),
                label: OTHER.into(),
                only: Some((dim, rest.into_iter().collect())),
            });
        }
        (defs, left_out)
    }
}

/// Per column, the sum of the series (split) or the first series.
fn totals(values: &[Vec<Option<Decimal>>], split: bool) -> Vec<Option<Decimal>> {
    if !split {
        return values.first().cloned().unwrap_or_default();
    }
    let n = values.first().map_or(0, Vec::len);
    (0..n)
        .map(|i| values.iter().filter_map(|v| v[i]).reduce(|a, b| a + b))
        .collect()
}

/// One measure over a bucket's rows that spans `days` days; `None` where it
/// has no value.
fn measure(m: Measure, txs: &[&Tx], days: i64) -> Option<Decimal> {
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
        Measure::AveragePerDay => (days > 0).then(|| cents(spent / Decimal::from(days))),
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

/// The split values shown as their own series, and those folded into "Other".
/// Weekdays are all seven, Monday first; years run in calendar order and keep
/// the most recent; other splits keep the largest. At most six series,
/// "Other" included.
fn split_keys(split: Split, ranked: Vec<String>, split_top: u8) -> (Vec<String>, Vec<String>) {
    let mut kept = match split {
        Split::Weekday => return ((0..7).map(|i: u8| i.to_string()).collect(), Vec::new()),
        Split::Year => {
            let mut years = ranked;
            years.sort();
            years
        }
        _ => ranked,
    };
    let keep = usize::from(split_top);
    let cut = if kept.len() > keep {
        keep.min(5)
    } else {
        kept.len()
    };
    let rest = if split == Split::Year {
        kept.drain(..kept.len() - cut).collect()
    } else {
        kept.split_off(cut)
    };
    (kept, rest)
}

/// The keys of `dim` among `rows`, by `m` over each key's rows, largest first.
fn ranked(names: &Names, dim: Dim, rows: &[&Row], m: Measure) -> Vec<String> {
    let mut by_key: BTreeMap<String, Vec<&Tx>> = BTreeMap::new();
    for r in rows {
        by_key.entry(names.key(dim, r)).or_default().push(r.t);
    }
    // Per day ranks as the spending does: every key shares the same days.
    let m = if m == Measure::AveragePerDay {
        Measure::Spent
    } else {
        m
    };
    let mut ranked: Vec<(String, Option<Decimal>)> = by_key
        .into_iter()
        .map(|(k, txs)| {
            let value = measure(m, &txs, 0);
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
