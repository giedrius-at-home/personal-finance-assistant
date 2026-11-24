//! The periods of a plot and the buckets along its axis.

use std::collections::HashMap;

use finance_types::plot::{Axis, Period, Split};
use jiff::Span;
use jiff::civil::Date;
use rust_decimal::Decimal;

use super::{Column, Def, Dim, Frame, Row, WEEKDAYS, measure};
use crate::model::Tx;
use crate::period::shift_month;
use crate::{Error, Result};

/// The most buckets one plot shows, per time unit.
const LIMIT_DAYS: i64 = 366;
const LIMIT_WEEKS: i64 = 260;
const LIMIT_MONTHS: i64 = 240;

/// The days a period covers, both included, cut at today. Month-based
/// periods also hold their months: a row counts by its month then.
#[derive(Clone, Copy)]
pub(super) struct Window {
    pub from: Date,
    pub to: Date,
    pub months: Option<(Date, Date)>,
}

impl Frame<'_> {
    /// The month (budget or calendar) a day counts in.
    fn month_of_day(&self, day: Date) -> Date {
        if self.budget {
            self.ctx.resolver.period_for(day, None)
        } else {
            day.first_of_month()
        }
    }

    /// The first day of `month`.
    fn month_start(&self, month: Date) -> Date {
        if self.budget {
            self.ctx.resolver.period_start_date(month)
        } else {
            month
        }
    }

    /// Month, Quarter and Year buckets of budget months follow the month a row counts in.
    pub(super) fn by_month(&self) -> bool {
        matches!(self.spec.by, Axis::Month | Axis::Quarter | Axis::Year)
    }

    /// The month (budget or calendar) running today.
    pub(super) fn current(&self) -> Date {
        if self.budget {
            self.ctx.current_month()
        } else {
            self.today().first_of_month()
        }
    }

    /// All time on a Day, Week or Month axis holding more buckets than one
    /// plot shows: the first day of the most recent ones kept, and what is
    /// shown ("last 366 days"). None when everything fits.
    pub(super) fn all_time_cap(&self) -> Option<(Date, &'static str)> {
        if self.spec.period != Period::AllTime {
            return None;
        }
        let today = self.today();
        let (from, shown) = match self.spec.by {
            Axis::Day => (
                today.checked_sub(Span::new().days(LIMIT_DAYS - 1)).ok()?,
                "last 366 days",
            ),
            Axis::Week => (
                bucket_start(Axis::Week, today)
                    .checked_sub(Span::new().days((LIMIT_WEEKS - 1) * 7))
                    .ok()?,
                "last 260 weeks",
            ),
            Axis::Month => (
                self.month_start(shift_month(self.current(), -(LIMIT_MONTHS as i32 - 1))),
                "last 240 months",
            ),
            _ => return None,
        };
        let first = self.rows.iter().map(|r| r.day).min()?;
        (first < from).then_some((from, shown))
    }

    /// The plot's period (`back` 0), or the one of equal length `back`
    /// periods before it.
    pub(super) fn window(&self, back: i32) -> Result<Window> {
        let today = self.today();
        let current = self.current();
        let months = |first: Date, last: Date| {
            let n = month_number(last) - month_number(first) + 1;
            let (first, last) = (shift_month(first, -n * back), shift_month(last, -n * back));
            let end = self.month_start(shift_month(last, 1));
            Window {
                from: self.month_start(first),
                to: end.yesterday().unwrap_or(end).min(today),
                months: Some((first, last)),
            }
        };
        Ok(match &self.spec.period {
            Period::ThisMonth => months(current, current),
            Period::LastMonth => {
                let month = shift_month(current, -1);
                months(month, month)
            }
            Period::Last3Months => months(shift_month(current, -2), current),
            Period::Last6Months => months(shift_month(current, -5), current),
            Period::Last12Months => months(shift_month(current, -11), current),
            Period::ThisYear => months(current.first_of_year(), current),
            Period::AllTime => Window {
                from: self
                    .rows
                    .iter()
                    .map(|r| r.day)
                    .min()
                    .unwrap_or(today)
                    .max(self.all_time_cap().map_or(Date::MIN, |(from, _)| from)),
                to: today,
                months: None,
            },
            Period::Between { from, to } => {
                let day = |s: &str| {
                    s.parse::<Date>().map_err(|_| {
                        Error::Rejected("Dates must be real days written YYYY-MM-DD.".into())
                    })
                };
                let (from, to) = (day(from)?, day(to)?);
                let len = i64::from(to.since(from).map_or(0, |s| s.get_days())) + 1;
                let shift = |d: Date| {
                    d.checked_sub(Span::new().days(len * i64::from(back)))
                        .map_err(|_| Error::Rejected("The period before is out of range.".into()))
                };
                Window {
                    from: shift(from)?,
                    to: shift(to)?,
                    months: None,
                }
            }
        })
    }

    pub(super) fn rows_in(&self, w: &Window) -> Vec<&Row<'_>> {
        self.rows
            .iter()
            .copied()
            .filter(|r| match w.months {
                Some((first, last)) => first <= r.month && r.month <= last,
                None => w.from <= r.day && r.day <= w.to,
            })
            .collect()
    }

    /// The columns along the axis, the bucket starts of a time axis, and how
    /// many named items "Other" holds. `ranking`: a named axis's keys, first first.
    pub(super) fn columns(
        &self,
        w: &Window,
        ranking: Vec<String>,
    ) -> Result<(Vec<Column>, Vec<Date>, usize)> {
        let spec = self.spec;
        Ok(match spec.by {
            Axis::Weekday => (
                WEEKDAYS
                    .iter()
                    .enumerate()
                    .map(|(i, w)| Column {
                        label: w[..3].into(),
                        title: (*w).into(),
                        keys: vec![i.to_string()],
                        other: false,
                    })
                    .collect(),
                Vec::new(),
                0,
            ),
            Axis::MonthOfYear => (
                (1..=12)
                    .filter_map(|m| Date::new(2000, m, 1).ok())
                    .map(|d| Column {
                        label: d.strftime("%b").to_string(),
                        title: d.strftime("%B").to_string(),
                        keys: vec![d.month().to_string()],
                        other: false,
                    })
                    .collect(),
                Vec::new(),
                0,
            ),
            Axis::DayOfMonth => (
                (1..=31)
                    .map(|d| Column {
                        label: d.to_string(),
                        title: format!("Day {d}"),
                        keys: vec![d.to_string()],
                        other: false,
                    })
                    .collect(),
                Vec::new(),
                0,
            ),
            _ if spec.time_axis() => {
                let (from, to) = match w.months {
                    Some(months) if self.by_month() => months,
                    // Budget months over days (All time, Between): from the
                    // budget month of the first day to that of the last (the
                    // running one when the window reaches today), so every
                    // row, keyed by its budget month, has a column.
                    None if self.budget && self.by_month() => {
                        let mut first = self.month_of_day(w.from);
                        if spec.by == Axis::Month && self.all_time_cap().is_some() {
                            first =
                                first.max(shift_month(self.current(), -(LIMIT_MONTHS as i32 - 1)));
                        }
                        let mut last = self.month_of_day(w.to);
                        if w.to >= self.today() {
                            last = last.max(self.current());
                        }
                        (first, last)
                    }
                    _ => (w.from, w.to),
                };
                let starts = buckets(spec.by, from, to)?;
                let columns = starts
                    .iter()
                    .map(|&d| bucket_column(spec.by, d, d.year() != w.to.year()))
                    .collect();
                (columns, starts, 0)
            }
            _ => {
                let dim = Dim::from(spec.by);
                let mut kept = ranking;
                let rest = kept.split_off(kept.len().min(usize::from(spec.top)));
                let left_out = rest.len();
                let mut columns: Vec<Column> = kept
                    .into_iter()
                    .map(|k| Column::named(self.names.label(dim, &k), vec![k]))
                    .collect();
                if !rest.is_empty() {
                    columns.push(Column {
                        other: true,
                        ..Column::named(super::OTHER.into(), rest)
                    });
                }
                (columns, Vec::new(), left_out)
            }
        })
    }

    /// The start of the bucket that holds today.
    pub(super) fn today_key(&self) -> Date {
        let month = if self.budget && self.by_month() {
            self.ctx.current_month()
        } else {
            self.today()
        };
        bucket_start(self.spec.by, month)
    }

    /// The start of the bucket that holds `day`.
    fn key_of(&self, day: Date) -> Date {
        let day = if self.budget && self.by_month() {
            self.month_of_day(day)
        } else {
            day
        };
        bucket_start(self.spec.by, day)
    }

    /// Per column, whether its bucket is over; every column off a time axis is.
    pub(super) fn finished(&self, starts: &[Date], columns: usize) -> Vec<bool> {
        if starts.is_empty() {
            return vec![true; columns];
        }
        let today = self.today_key();
        starts.iter().map(|&s| s < today).collect()
    }

    /// Per column, whether it goes into an average (the insight's, the
    /// average line, the usual): it is finished and does not end before the
    /// ledger's first counted transaction. A bucket that holds that
    /// transaction counts; buckets entirely before it are still drawn (as
    /// zero) but are not averaged.
    pub(super) fn averaged(&self, starts: &[Date], columns: usize) -> Vec<bool> {
        let first = self.first_day.map(|d| self.key_of(d));
        let mut averaged = self.finished(starts, columns);
        for (a, &s) in averaged.iter_mut().zip(starts) {
            *a &= first.is_some_and(|f| s >= f);
        }
        averaged
    }

    /// The split that divides the days themselves: each year or weekday
    /// series has only its own days.
    fn partition(&self) -> Option<Dim> {
        match self.spec.split {
            Some(Split::Year) => Some(Dim::Year),
            Some(Split::Weekday) => Some(Dim::Weekday),
            _ => None,
        }
    }

    /// Per column, the days of the period up to today it covers (a bucket's
    /// own days, a weekday's or day of month's occurrences, or every day),
    /// by the split value that partitions them (`""` for none).
    pub(super) fn days(&self, w: &Window, columns: &[Column]) -> Vec<HashMap<String, i64>> {
        let mut days = vec![HashMap::new(); columns.len()];
        if !self
            .spec
            .measures
            .iter()
            .any(|m| m.measure == finance_types::plot::Measure::AveragePerDay)
        {
            return days;
        }
        let index = column_index(columns);
        let dim = Dim::from(self.spec.by);
        let named = self.spec.named_axis();
        let partition = self.partition();
        // Days before the ledger's first counted transaction have no data.
        let mut day = w.from.max(self.first_day.unwrap_or(w.from));
        while day <= w.to.min(self.today()) {
            let month = self.month_of_day(day);
            if w.months
                .is_none_or(|(first, last)| first <= month && month <= last)
            {
                let part = partition.map_or_else(String::new, |p| point_key(p, day, month));
                if named {
                    for n in &mut days {
                        *n.entry(part.clone()).or_default() += 1;
                    }
                } else if let Some(&i) = index.get(point_key(dim, day, month).as_str()) {
                    *days[i].entry(part).or_default() += 1;
                }
            }
            let Ok(next) = day.tomorrow() else { break };
            day = next;
        }
        days
    }

    /// Per def, its values over `rows` (the rows of `w`) in each column.
    pub(super) fn values(
        &self,
        w: &Window,
        rows: &[&Row],
        columns: &[Column],
        defs: &[Def],
    ) -> Vec<Vec<Option<Decimal>>> {
        let index = column_index(columns);
        let axis = Dim::from(self.spec.by);
        // As the budget counts it: a row of the window's months dated outside
        // its days counts on its first (or last) day.
        let clamp =
            self.budget && w.months.is_some() && matches!(self.spec.by, Axis::Day | Axis::Week);
        let mut cells: Vec<Vec<&Row>> = vec![Vec::new(); columns.len()];
        for r in rows {
            let key = if clamp {
                point_key(axis, r.day.clamp(w.from, w.to), r.month)
            } else {
                self.names.key(axis, r)
            };
            if let Some(&i) = index.get(key.as_str()) {
                cells[i].push(r);
            }
        }
        let days = self.days(w, columns);
        let partition = self.partition();
        defs.iter()
            .map(|def| {
                cells
                    .iter()
                    .zip(&days)
                    .map(|(cell, days)| {
                        let txs: Vec<&Tx> = cell
                            .iter()
                            .filter(|r| {
                                def.only
                                    .as_ref()
                                    .is_none_or(|(dim, set)| set.contains(&self.names.key(*dim, r)))
                            })
                            .map(|r| r.t)
                            .collect();
                        let days = match (partition, &def.only) {
                            (Some(_), Some((_, set))) => {
                                set.iter().filter_map(|k| days.get(k)).sum()
                            }
                            _ => days.values().sum(),
                        };
                        measure(def.m.measure, &txs, days)
                    })
                    .collect()
            })
            .collect()
    }
}

fn column_index(columns: &[Column]) -> HashMap<&str, usize> {
    columns
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.keys.iter().map(move |k| (k.as_str(), i)))
        .collect()
}

fn month_number(month: Date) -> i32 {
    i32::from(month.year()) * 12 + i32::from(month.month())
}

/// The key of a day under a time, weekday, day-of-month or year `dim`, when
/// it counts in `month`.
pub(super) fn point_key(dim: Dim, day: Date, month: Date) -> String {
    match dim {
        Dim::Time(axis @ (Axis::Month | Axis::Quarter | Axis::Year)) => {
            bucket_start(axis, month).to_string()
        }
        Dim::Time(axis) => bucket_start(axis, day).to_string(),
        Dim::Weekday => day.weekday().to_monday_zero_offset().to_string(),
        Dim::DayOfMonth => day.day().to_string(),
        Dim::MonthOfYear => month.month().to_string(),
        Dim::Year => month.year().to_string(),
        _ => String::new(),
    }
}

pub(super) fn bucket_start(axis: Axis, day: Date) -> Date {
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
        Axis::Day => (LIMIT_DAYS as usize, "days"),
        Axis::Week => (LIMIT_WEEKS as usize, "weeks"),
        Axis::Month => (LIMIT_MONTHS as usize, "months"),
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
        other: false,
    }
}
