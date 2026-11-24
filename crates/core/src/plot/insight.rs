//! The one plain sentence under a plot, from the plot's own numbers. The
//! unfinished bucket is never the last, the highest or the lowest.

use finance_types::plot::{Axis, Compare, Insight, InsightKind, Measure, PlotSpec, Unit, Value};
use finance_types::report::Currency;
use rust_decimal::{Decimal, RoundingStrategy};

use super::options::mean;
use super::{Column, measure_name};
use crate::closure::short_money;

/// `totals`: per column, the plain (not running) value; `reference`: the
/// plain comparison series.
pub(super) fn insight(
    spec: &PlotSpec,
    columns: &[Column],
    totals: &[Option<Decimal>],
    finished: &[bool],
    reference: Option<&[Option<Decimal>]>,
    unit: Unit,
    currency: &Currency,
) -> Option<Insight> {
    if spec.value == Value::Change || totals.iter().flatten().all(Decimal::is_zero) {
        return None;
    }
    let money = |v: Decimal| match unit {
        Unit::Money => short_money(v, currency),
        Unit::Count => v.normalize().to_string(),
    };
    // With several measures the sentence names the one it is about.
    let measure = (spec.measures.len() > 1).then(|| measure_name(spec.measures[0].measure).1);
    let insight = |kind, label: &str, value, against, text: String| {
        Some(Insight {
            kind,
            label: label.into(),
            value,
            against: Some(against),
            text: match measure {
                Some(m) => format!("{m}, {text}"),
                None => text,
            },
        })
    };
    // Sums and shares only mean something for a measure that adds up.
    let additive = matches!(
        spec.measures[0].measure,
        Measure::Spent | Measure::Income | Measure::Net | Measure::Count
    );
    let peak_shape = matches!(
        spec.by,
        Axis::Weekday | Axis::DayOfMonth | Axis::MonthOfYear
    );
    if !additive && (reference.is_some() || !peak_shape) {
        return None;
    }
    if let Some(reference) = reference {
        // Finished positions with a counterpart, on both sides.
        let pairs: Vec<(Decimal, Decimal)> = totals
            .iter()
            .zip(reference)
            .zip(finished)
            .filter(|(_, done)| **done)
            .filter_map(|((v, r), _)| v.zip(*r))
            .collect();
        if pairs.is_empty() {
            return None;
        }
        let value: Decimal = pairs.iter().map(|(v, _)| *v).sum();
        let against: Decimal = pairs.iter().map(|(_, r)| *r).sum();
        let what = if spec.compare == Compare::AverageOf3 {
            "the usual"
        } else {
            "the period before"
        };
        let text = format!(
            "This period: {}, {} {what} ({})",
            money(value),
            compare(value, against),
            money(against)
        );
        return insight(
            InsightKind::AgainstPrevious,
            "This period",
            value,
            against,
            text,
        );
    }
    let done: Vec<(usize, Decimal)> = totals
        .iter()
        .zip(finished)
        .enumerate()
        .filter(|(_, (_, finished))| **finished)
        .filter_map(|(i, (v, _))| v.map(|v| (i, v)))
        .collect();
    // First of equals wins.
    let highest = |a: (usize, Decimal), b: (usize, Decimal)| if b.1 > a.1 { b } else { a };
    let lowest = |a: (usize, Decimal), b: (usize, Decimal)| if b.1 < a.1 { b } else { a };
    let positive = || done.iter().copied().filter(|(_, v)| *v > Decimal::ZERO);
    match spec.by {
        Axis::Weekday | Axis::DayOfMonth | Axis::MonthOfYear => {
            let (peak, high) = positive().reduce(highest)?;
            let (low, least) = positive().reduce(lowest)?;
            if low == peak || least == high {
                return None;
            }
            let title = &columns[peak].title;
            let text = format!(
                "{title}: {}, {} the lowest, {} ({})",
                money(high),
                compare(high, least),
                columns[low].title,
                money(least)
            );
            insight(InsightKind::PeakAgainstLowest, title, high, least, text)
        }
        _ if spec.named_axis() => {
            let (top, value) = done
                .iter()
                .copied()
                // Neither "Other" nor the merchant with no name ("Unknown").
                .filter(|(i, _)| {
                    !columns[*i].other && !(spec.by == Axis::Merchant && columns[*i].keys == [""])
                })
                .reduce(highest)
                .filter(|(_, v)| *v > Decimal::ZERO)?;
            let total: Decimal = positive().map(|(_, v)| v).sum();
            let share = whole(value * Decimal::ONE_HUNDRED / total);
            let title = &columns[top].title;
            let text = format!("{title}: {}, {share}% of {}", money(value), money(total));
            insight(InsightKind::LargestShare, title, value, total, text)
        }
        _ => {
            let (&(last, value), others) = done.split_last()?;
            if others.len() < 2 {
                return None;
            }
            let others: Vec<Decimal> = others.iter().map(|(_, v)| *v).collect();
            let average = mean(&others);
            let title = &columns[last].title;
            let text = format!(
                "{title}: {}, {} your average of {}",
                money(value),
                compare(value, average),
                money(average)
            );
            insight(InsightKind::LastAgainstAverage, title, value, average, text)
        }
    }
}

/// "19% above", "4% below", "in line with", or "against" when `against` is zero.
fn compare(value: Decimal, against: Decimal) -> String {
    if against.is_zero() {
        return "against".into();
    }
    let percent = whole((value - against) * Decimal::ONE_HUNDRED / against.abs());
    if percent.is_zero() {
        "in line with".into()
    } else if percent > Decimal::ZERO {
        format!("{percent}% above")
    } else {
        format!("{}% below", -percent)
    }
}

/// A percentage as a whole number, halves away from zero.
fn whole(percent: Decimal) -> Decimal {
    percent.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}
