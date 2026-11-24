//! The options on top of a plain plot: change against the period before,
//! the comparison series, the running total and the average line.

use std::collections::{BTreeMap, BTreeSet};

use finance_types::plot::{Compare, Measure};
use rust_decimal::Decimal;

use super::buckets::{Window, bucket_start};
use super::{Column, Def, Dim, Frame, Row, measure};
use crate::Result;
use crate::model::Tx;
use crate::trends::cents;

/// A named axis's keys, by the size of their change against the period
/// before, largest first (ties in key order).
pub(super) fn change_ranking(frame: &Frame, window: &Window, rows: &[&Row]) -> Result<Vec<String>> {
    let before = frame.window(1)?;
    let before_rows = frame.rows_in(&before);
    let dim = Dim::from(frame.spec.by);
    let (now, then) = (by_key(frame, dim, rows), by_key(frame, dim, &before_rows));
    let all = [Column::named(String::new(), Vec::new())];
    let day_count = |w: &Window| frame.days(w, &all)[0].values().sum::<i64>();
    let (days_now, days_then) = (day_count(window), day_count(&before));
    let m = frame.spec.measures[0].measure;
    let value = |map: &BTreeMap<String, Vec<&Tx>>, key: &str, days| {
        measure(m, map.get(key).map(Vec::as_slice).unwrap_or_default(), days)
    };
    let keys: BTreeSet<&String> = now.keys().chain(then.keys()).collect();
    let mut ranked: Vec<(String, Option<Decimal>)> = keys
        .into_iter()
        .map(|k| {
            let change = difference(value(&now, k, days_now), value(&then, k, days_then));
            (k.clone(), change)
        })
        .collect();
    // `None` ranks last.
    ranked.sort_by_key(|(_, change)| std::cmp::Reverse(change.map(|c| c.abs())));
    Ok(ranked.into_iter().map(|(k, _)| k).collect())
}

/// `values` become their change: on a named axis against the same item over
/// the period before, on a time axis against the bucket before.
pub(super) fn change(
    frame: &Frame,
    window: &Window,
    columns: &[Column],
    defs: &[Def],
    values: &mut [Vec<Option<Decimal>>],
) -> Result<()> {
    let before = frame.window(1)?;
    let before_rows = frame.rows_in(&before);
    if frame.spec.named_axis() {
        let then = frame.values(&before, &before_rows, columns, defs);
        for (now, then) in values.iter_mut().zip(then) {
            for (v, t) in now.iter_mut().zip(then) {
                *v = difference(*v, t);
            }
        }
    } else {
        let (before_columns, _, _) = frame.columns(&before, Vec::new())?;
        let then = frame.values(&before, &before_rows, &before_columns, defs);
        let first_of = |w: &Window| match w.months {
            Some((first, _)) if frame.by_month() => first,
            _ => w.from,
        };
        let first = first_of(window);
        let start = bucket_start(frame.spec.by, first);
        // A first bucket cut by the period start: its whole is the cut part
        // plus the period before's last bucket (the same calendar bucket),
        // when that period holds all of it, the ledger reaches back to it and
        // the measure adds up.
        let cut = start < first;
        let whole = first_of(&before) <= start
            && frame.first_day.is_some_and(|d| d <= start)
            && matches!(
                frame.spec.measures[0].measure,
                Measure::Spent | Measure::Income | Measure::Net | Measure::Count
            );
        for (now, then) in values.iter_mut().zip(then) {
            let last_before = then.last().copied().flatten();
            let mut previous = if cut {
                whole
                    .then(|| now.first().copied().flatten().zip(last_before))
                    .flatten()
                    .map(|(a, b)| a + b)
            } else {
                last_before
            };
            for (i, v) in now.iter_mut().enumerate() {
                let value = *v;
                *v = if cut && i == 0 {
                    None
                } else {
                    difference(value, previous)
                };
                if !(cut && i == 0) {
                    previous = value;
                }
            }
        }
    }
    Ok(())
}

fn by_key<'a>(frame: &Frame, dim: Dim, rows: &[&Row<'a>]) -> BTreeMap<String, Vec<&'a Tx>> {
    let mut map: BTreeMap<String, Vec<&Tx>> = BTreeMap::new();
    for r in rows {
        map.entry(frame.names.key(dim, r)).or_default().push(r.t);
    }
    map
}

fn difference(a: Option<Decimal>, b: Option<Decimal>) -> Option<Decimal> {
    a.zip(b).map(|(a, b)| a - b)
}

/// A comparison series: its key, label and values.
pub(super) type Reference = (&'static str, &'static str, Vec<Option<Decimal>>);

/// The comparison series (key, label, values over `n` positions) for
/// `spec.compare`: the first measure over the period before, or the mean of
/// the three before, bucket for bucket. A position without a finished
/// counterpart is `None`.
pub(super) fn reference(frame: &Frame, n: usize) -> Result<Option<Reference>> {
    let (key, label, periods) = match frame.spec.compare {
        Compare::None => return Ok(None),
        Compare::Previous => ("previous", "Period before", 1),
        Compare::AverageOf3 => ("usual", "Usual", 3),
    };
    let def = [Def::whole(frame.spec.measures[0].clone())];
    let mut counterparts = Vec::new();
    for back in 1..=periods {
        let w = frame.window(back)?;
        // A period from before the ledger starts has no counterpart.
        if frame.first_day.is_none_or(|first| w.to < first) {
            counterparts.push(vec![None; n]);
            continue;
        }
        let rows = frame.rows_in(&w);
        let (columns, starts, _) = frame.columns(&w, Vec::new())?;
        // The period before is shown as it is; the usual averages only
        // buckets the ledger reaches.
        let finished = if periods == 1 {
            frame.finished(&starts, columns.len())
        } else {
            frame.averaged(&starts, columns.len())
        };
        let values = frame.values(&w, &rows, &columns, &def).remove(0);
        counterparts.push(
            (0..n)
                .map(|i| {
                    values
                        .get(i)
                        .copied()
                        .flatten()
                        .filter(|_| finished.get(i) == Some(&true))
                })
                .collect::<Vec<_>>(),
        );
    }
    let values = (0..n)
        .map(|i| {
            let found: Vec<Decimal> = counterparts.iter().filter_map(|c| c[i]).collect();
            match found.as_slice() {
                [] => None,
                [one] if periods == 1 => Some(*one),
                _ => Some(mean(&found)),
            }
        })
        .collect();
    Ok(Some((key, label, values)))
}

/// Each value becomes the sum of those up to it.
pub(super) fn running(values: &mut [Option<Decimal>]) {
    let mut total = Decimal::ZERO;
    for v in values.iter_mut().flatten() {
        total += *v;
        *v = total;
    }
}

/// The mean of the finished positions' values.
pub(super) fn average(values: &[Option<Decimal>], finished: &[bool]) -> Option<Decimal> {
    let found: Vec<Decimal> = values
        .iter()
        .zip(finished)
        .filter(|(_, done)| **done)
        .filter_map(|(v, _)| *v)
        .collect();
    (!found.is_empty()).then(|| mean(&found))
}

pub(super) fn mean(values: &[Decimal]) -> Decimal {
    cents(values.iter().sum::<Decimal>() / Decimal::from(values.len()))
}

/// Positions marked in `future` get no value.
pub(super) fn blank(values: &mut [Option<Decimal>], future: &[bool]) {
    for (v, _) in values.iter_mut().zip(future).filter(|(_, f)| **f) {
        *v = None;
    }
}
