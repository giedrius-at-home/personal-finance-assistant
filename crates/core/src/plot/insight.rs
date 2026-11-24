//! The one plain sentence under a plot, from the plot's own numbers. The
//! unfinished bucket is never the last, the highest or the lowest.

use finance_types::plot::{Axis, Compare, Insight, InsightKind, PlotSpec, Unit, Value};
use finance_types::report::Currency;
use rust_decimal::Decimal;

use super::Column;
use super::options::mean;
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
    let insight = |kind, label: &str, value, against, text| {
        Some(Insight {
            kind,
            label: label.into(),
            value,
            against: Some(against),
            text,
        })
    };
    if let Some(reference) = reference {
        let value: Decimal = totals.iter().flatten().sum();
        let against: Decimal = reference.iter().flatten().sum();
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
        Axis::Weekday | Axis::DayOfMonth => {
            let (peak, high) = positive().reduce(highest)?;
            let (low, least) = positive().reduce(lowest)?;
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
                .filter(|(i, _)| !columns[*i].other)
                .reduce(highest)
                .filter(|(_, v)| *v > Decimal::ZERO)?;
            let total: Decimal = positive().map(|(_, v)| v).sum();
            let share = (value * Decimal::ONE_HUNDRED / total).round();
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
    let percent = ((value - against) * Decimal::ONE_HUNDRED / against.abs()).round();
    if percent.is_zero() {
        "in line with".into()
    } else if percent > Decimal::ZERO {
        format!("{percent}% above")
    } else {
        format!("{}% below", -percent)
    }
}
