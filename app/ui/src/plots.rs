//! A saved plot on screen: `plot_query` for its spec, drawn with the chart
//! the spec names, or as a table of its numbers.

use std::sync::atomic::{AtomicU32, Ordering};

use finance_types::plot::{
    Axis, Chart, Draw, Measure, PlotData, PlotSeries, PlotSpec, Split, Unit, Value,
};
use finance_types::report::Currency;
use leptos::prelude::*;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api;
use crate::ds::charts::{
    BarChart, DataTable, Donut, Fmt, Heatmap, Line, LineChart, Ranked, Scatter, Series, Slice, fmt,
    series_color,
};
use crate::ds::patterns::{ERROR_LINE, MUTED, PLOT_FOOT, PLOT_INSIGHT, SKELETON_CARD};
use crate::ds::{EmptyState, Skeleton, Switch, money};
use crate::pages::{f, plural};

/// The query's key for the series that folds the rest together.
const OTHER_KEY: &str = "other";
/// The most plain series a chart draws, "Other" included.
const MAX_SERIES: usize = 6;
/// A Weekday split keeps all seven days (the query never folds them).
const WEEKDAYS: usize = 7;
/// "Other": muted, after the named series.
const OTHER_COLOR: &str = "var(--ds-chart-limit)";
/// The seventh day of a Weekday split, past the six-colour palette.
const SEVENTH_COLOR: &str = "var(--ds-chart-fixed)";
/// Reference series (period before, usual, average): dashed, in the axis colour.
const REFERENCE_COLOR: &str = "var(--ds-chart-axis)";
/// Money axes need room for "€1,200.00".
const Y_AXIS_WIDTH: u32 = 76;

fn chart_series(s: &PlotSeries, color: String, line: Line) -> Series {
    Series {
        key: s.key.clone(),
        label: s.label.clone(),
        color,
        values: s.values.iter().map(|v| v.map(f)).collect(),
        line,
    }
}

/// Chart input for `data`: colours by position, "Other" muted and last,
/// reference series dashed after them. At most six plain series (seven for a
/// Weekday split). A measure drawn as a line is `Line::Straight`; bars are
/// `Line::Smooth`, the default.
pub fn series_for(data: &PlotData) -> (Vec<String>, Vec<Series>) {
    let other = data
        .series
        .iter()
        .find(|s| !s.reference && s.key == OTHER_KEY);
    let named: Vec<&PlotSeries> = data
        .series
        .iter()
        .filter(|s| !s.reference && s.key != OTHER_KEY)
        .collect();
    let room = if other.is_none() && named.len() == WEEKDAYS {
        WEEKDAYS
    } else {
        MAX_SERIES - usize::from(other.is_some())
    };
    let mut out: Vec<Series> = named
        .iter()
        .take(room)
        .enumerate()
        .map(|(i, s)| {
            let color = if i < MAX_SERIES {
                series_color(i, None)
            } else {
                SEVENTH_COLOR.into()
            };
            let line = if s.draw == Draw::Line {
                Line::Straight
            } else {
                Line::Smooth
            };
            chart_series(s, color, line)
        })
        .collect();
    out.extend(other.map(|s| chart_series(s, OTHER_COLOR.into(), Line::Smooth)));
    out.extend(
        data.series
            .iter()
            .filter(|s| s.reference)
            .map(|s| chart_series(s, REFERENCE_COLOR.into(), Line::Dashed)),
    );
    (data.categories.clone(), out)
}

/// What a donut or ranked chart draws: the positive items of the first
/// series, with how many were left out (zero, below zero or no value).
pub fn positive_items(data: &PlotData) -> (Vec<(String, f64)>, usize) {
    let Some(first) = data.series.iter().find(|s| !s.reference) else {
        return (Vec::new(), 0);
    };
    let mut left = 0;
    let items = data
        .categories
        .iter()
        .zip(&first.values)
        .filter_map(|(name, v)| match v {
            Some(v) if *v > Decimal::ZERO => Some((name.clone(), f(*v))),
            _ => {
                left += 1;
                None
            }
        })
        .collect();
    (items, left)
}

/// The subtitle, saying how many items a donut or ranked chart left out.
pub fn with_left_out(subtitle: &str, left: usize) -> String {
    match left {
        0 => subtitle.to_owned(),
        n => format!(
            "{subtitle} · {} at zero or below not shown",
            plural(n, "item")
        ),
    }
}

/// The totals row of the numbers table: a sum per series that adds up
/// (spent, income, net, count); none for averages, largest, references or a
/// running total. None when no column has one.
pub fn totals(spec: &PlotSpec, data: &PlotData) -> Option<Vec<Option<f64>>> {
    if spec.cumulative {
        return None;
    }
    let adds = |m: &Measure| {
        matches!(
            m,
            Measure::Spent | Measure::Income | Measure::Net | Measure::Count
        )
    };
    let mut plain = 0;
    let row: Vec<Option<f64>> = data
        .series
        .iter()
        .map(|s| {
            if s.reference {
                return None;
            }
            // Split series all show the first measure; otherwise one series per measure.
            let measure = spec
                .measures
                .get(if spec.split.is_some() { 0 } else { plain });
            plain += 1;
            measure
                .is_some_and(|m| adds(&m.measure))
                .then(|| f(s.values.iter().flatten().copied().sum::<Decimal>()))
        })
        .collect();
    row.iter().any(Option::is_some).then_some(row)
}

/// Nothing to draw: no buckets, or no plain value other than zero.
fn is_empty(data: &PlotData) -> bool {
    data.categories.is_empty()
        || data
            .series
            .iter()
            .filter(|s| !s.reference)
            .all(|s| s.values.iter().all(|v| v.is_none_or(|v| v.is_zero())))
}

fn unit_fmt(unit: Unit, currency: &Currency) -> Fmt {
    match unit {
        Unit::Money => {
            let currency = currency.clone();
            fmt(move |n| money(Decimal::try_from(n).unwrap_or_default(), &currency))
        }
        Unit::Count => fmt(|n| format!("{:.0}", n)),
    }
}

fn axis_name(by: Axis) -> &'static str {
    match by {
        Axis::Day => "Day",
        Axis::Week => "Week",
        Axis::Month | Axis::MonthOfYear => "Month",
        Axis::Quarter => "Quarter",
        Axis::Year => "Year",
        Axis::Weekday => "Weekday",
        Axis::DayOfMonth => "Day of month",
        Axis::Category => "Category",
        Axis::Group => "Group",
        Axis::Merchant => "Merchant",
        Axis::Account => "Account",
        Axis::FixedOrVariable => "Fixed or variable",
    }
}

/// The numbers: one row per bucket, one column per series.
fn data_table(spec: &PlotSpec, data: &PlotData) -> AnyView {
    let columns = std::iter::once(axis_name(spec.by).to_owned())
        .chain(data.series.iter().map(|s| s.label.clone()))
        .collect();
    let rows = data
        .titles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let values = data
                .series
                .iter()
                .map(|s| s.values.get(i).copied().flatten().map(f))
                .collect();
            (t.clone(), values)
        })
        .collect();
    let formats = data
        .series
        .iter()
        .map(|s| unit_fmt(s.unit, &data.currency))
        .collect();
    view! {
        <DataTable
            columns=columns
            rows=rows
            totals=totals(spec, data)
            format=unit_fmt(data.unit, &data.currency)
            formats=formats
            label=data.subtitle.clone()
        />
    }
    .into_any()
}

/// The chart `spec.chart` names, over `data`.
fn drawing(spec: &PlotSpec, data: &PlotData) -> AnyView {
    let format = unit_fmt(data.unit, &data.currency);
    let label = data.subtitle.clone();
    let titles = data.titles.clone();
    let (categories, series) = series_for(data);
    match spec.chart {
        Chart::Bars | Chart::StackedBars => {
            // Measures drawn as lines, and reference series, go over the bars on the same axis.
            let (lines, bars): (Vec<Series>, Vec<Series>) =
                series.into_iter().partition(|s| s.line != Line::Smooth);
            view! {
                <BarChart
                    categories=categories
                    series=bars
                    lines=lines
                    format=format
                    stacked=spec.chart == Chart::StackedBars
                    titles=titles
                    partial_last=data.partial_last
                    y_axis_width=Y_AXIS_WIDTH
                    label=label
                    table=true
                />
            }
            .into_any()
        }
        Chart::Line | Chart::Area => {
            let series = series
                .into_iter()
                .map(|s| match s.line {
                    Line::Straight => s.line(Line::Smooth),
                    _ => s,
                })
                .collect();
            // Day labels ("23 Nov") need more room than month labels ("Nov").
            let long = categories.iter().any(|c| c.chars().count() > 4);
            let label_every = categories.len().div_ceil(if long { 6 } else { 12 }).max(1);
            view! {
                <LineChart
                    categories=categories
                    series=series
                    format=format
                    area=spec.chart == Chart::Area
                    titles=titles
                    label_every=label_every
                    partial_last=data.partial_last
                    y_axis_width=Y_AXIS_WIDTH
                    label=label
                    table=true
                />
            }
            .into_any()
        }
        Chart::Donut => {
            let (items, _) = positive_items(data);
            let last = items.len().saturating_sub(1);
            let slices = items
                .into_iter()
                .enumerate()
                .map(|(i, (name, value))| Slice {
                    key: format!("slice-{i}"),
                    color: if i == last && name == "Other" {
                        OTHER_COLOR.into()
                    } else {
                        series_color(i, None)
                    },
                    label: name,
                    value,
                })
                .collect();
            view! { <Donut data=slices format=format label=label /> }.into_any()
        }
        Chart::Ranked if spec.value == Value::Change => {
            let items = series.first().map_or_else(Vec::new, |s| {
                categories
                    .iter()
                    .zip(&s.values)
                    .filter_map(|(c, v)| Some((c.clone(), (*v)?)))
                    .collect()
            });
            view! { <Ranked items=items format=format two_sided=true label=label /> }.into_any()
        }
        Chart::Ranked => {
            let (items, _) = positive_items(data);
            view! { <Ranked items=items format=format label=label /> }.into_any()
        }
        Chart::Heatmap => {
            let plain: Vec<&Series> = series.iter().filter(|s| s.line != Line::Dashed).collect();
            let values = (0..categories.len())
                .map(|r| {
                    plain
                        .iter()
                        .map(|s| s.values.get(r).copied().flatten())
                        .collect()
                })
                .collect();
            // Weekday names don't fit a 40px cell: "Mon", "Tue", …
            let short = spec.split == Some(Split::Weekday);
            let columns = plain
                .iter()
                .map(|s| {
                    if short {
                        s.label.chars().take(3).collect()
                    } else {
                        s.label.clone()
                    }
                })
                .collect();
            view! { <Heatmap rows=categories columns=columns values=values format=format label=label /> }
                .into_any()
        }
        Chart::Scatter => {
            let plain: Vec<&PlotSeries> = data.series.iter().filter(|s| !s.reference).collect();
            let (Some(x), Some(y)) = (plain.first(), plain.get(1)) else {
                return view! { <EmptyState title="Nothing in this period" /> }.into_any();
            };
            let points = data
                .titles
                .iter()
                .enumerate()
                .filter_map(|(i, name)| {
                    let at = |s: &PlotSeries| s.values.get(i).copied().flatten().map(f);
                    Some((name.clone(), at(x)?, at(y)?))
                })
                .collect();
            view! {
                <Scatter
                    points=points
                    x_format=unit_fmt(x.unit, &data.currency)
                    y_format=unit_fmt(y.unit, &data.currency)
                    x_label=x.label.clone()
                    y_label=y.label.clone()
                />
            }
            .into_any()
        }
        Chart::Table => data_table(spec, data),
    }
}

#[derive(Serialize)]
struct QueryArgs {
    spec: PlotSpec,
}

/// A saved plot: loads `plot_query` for `spec` (again after every write and
/// bank sync, keeping the drawing meanwhile) and draws it, with its period,
/// its insight and a "Show numbers" switch under it.
#[component]
pub fn PlotView(
    spec: Signal<PlotSpec>,
    /// Set to the insight sentence each time data loads.
    #[prop(optional)]
    insight: Option<RwSignal<Option<String>>>,
) -> impl IntoView {
    let revision = crate::state().revision;
    let loaded = api::load::<PlotData, _>("plot_query", move || {
        revision.track();
        QueryArgs { spec: spec.get() }
    });
    if let Some(insight) = insight {
        Effect::new(move |_| {
            loaded.data.with(|d| {
                if let Some(Ok(d)) = d {
                    insight.set(d.insight.as_ref().map(|i| i.text.clone()));
                }
            });
        });
    }
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let switch_id = format!("plot-numbers-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let numbers = RwSignal::new(false);
    move || {
        loaded.data.with(|d| match d {
            None => view! { <Skeleton class=SKELETON_CARD /> }.into_any(),
            Some(Err(e)) => view! { <p role="alert" class=ERROR_LINE>{e.clone()}</p> }.into_any(),
            Some(Ok(d)) if is_empty(d) => view! { <EmptyState title="Nothing in this period" /> }.into_any(),
            Some(Ok(d)) => {
                let spec = spec.get_untracked();
                let table_only = spec.chart == Chart::Table;
                let left = match spec.chart {
                    Chart::Donut => positive_items(d).1,
                    Chart::Ranked if spec.value != Value::Change => positive_items(d).1,
                    _ => 0,
                };
                let subtitle = with_left_out(&d.subtitle, left);
                let insight = d.insight.as_ref().map(|i| view! { <p class=PLOT_INSIGHT>{i.text.clone()}</p> });
                let data = d.clone();
                let body = move || {
                    if table_only || numbers.get() {
                        data_table(&spec, &data)
                    } else {
                        drawing(&spec, &data)
                    }
                };
                let id = switch_id.clone();
                let switch = (!table_only).then(|| {
                    view! {
                        <div class="flex items-center gap-2">
                            <Switch
                                id=id.clone()
                                checked=numbers.into()
                                on_change=Callback::new(move |v| numbers.set(v))
                            />
                            <label for=id class="cursor-pointer text-sm text-fg">"Show numbers"</label>
                        </div>
                    }
                });
                view! {
                    <div>
                        {body}
                        <div class=PLOT_FOOT>
                            <div class="grid gap-1">
                                <p class=MUTED>{subtitle}</p>
                                {insight}
                            </div>
                            {switch}
                        </div>
                    </div>
                }
                .into_any()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use finance_types::plot::MeasureSpec;
    use finance_types::report::Currency;

    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn series(key: &str, values: &[Option<&str>]) -> PlotSeries {
        PlotSeries {
            key: key.into(),
            label: key.into(),
            values: values.iter().map(|v| v.map(d)).collect(),
            ..PlotSeries::default()
        }
    }

    fn reference(key: &str, values: &[Option<&str>]) -> PlotSeries {
        PlotSeries {
            reference: true,
            ..series(key, values)
        }
    }

    fn data(categories: &[&str], series: Vec<PlotSeries>) -> PlotData {
        PlotData {
            categories: categories.iter().map(|c| c.to_string()).collect(),
            titles: categories.iter().map(|c| c.to_string()).collect(),
            series,
            partial_last: false,
            insight: None,
            currency: Currency {
                code: "EUR".into(),
                minor_unit: 2,
            },
            unit: Unit::Money,
            subtitle: String::new(),
            left_out: 0,
        }
    }

    fn spec(measures: &[Measure], split: Option<Split>) -> PlotSpec {
        PlotSpec {
            measures: measures
                .iter()
                .map(|&measure| MeasureSpec {
                    measure,
                    draw: Draw::Bars,
                })
                .collect(),
            split,
            ..PlotSpec::default()
        }
    }

    #[test]
    fn series_for_colours_by_position_and_dashes_references() {
        let data = data(
            &["Jan", "Feb"],
            vec![
                series("split:a", &[Some("1"), None]),
                series("split:b", &[Some("2"), Some("3")]),
                series(OTHER_KEY, &[Some("4"), Some("5")]),
                reference("previous", &[Some("6"), Some("7")]),
            ],
        );
        let (categories, out) = series_for(&data);
        assert_eq!(categories, ["Jan", "Feb"]);
        let got: Vec<(&str, &str, Line)> = out
            .iter()
            .map(|s| (s.key.as_str(), s.color.as_str(), s.line))
            .collect();
        assert_eq!(
            got,
            [
                ("split:a", "var(--ds-chart-1)", Line::Smooth),
                ("split:b", "var(--ds-chart-2)", Line::Smooth),
                (OTHER_KEY, OTHER_COLOR, Line::Smooth),
                ("previous", REFERENCE_COLOR, Line::Dashed),
            ]
        );
        assert_eq!(
            out[0].values,
            [Some(1.0), None],
            "a missing value stays a gap"
        );
        let mut net = series("net", &[Some("1"), Some("2")]);
        net.draw = Draw::Line;
        assert_eq!(
            series_for(&super::tests::data(&["Jan", "Feb"], vec![net])).1[0].line,
            Line::Straight
        );
    }

    #[test]
    fn series_for_never_passes_more_than_six_plain_series() {
        let many = |n: usize, other: bool| {
            let mut s: Vec<PlotSeries> = (0..n)
                .map(|i| series(&format!("split:{i}"), &[Some("1")]))
                .collect();
            if other {
                s.push(series(OTHER_KEY, &[Some("1")]));
            }
            s.push(reference("usual", &[Some("1")]));
            data(&["Jan"], s)
        };
        let plain = |d: &PlotData| {
            series_for(d)
                .1
                .iter()
                .filter(|s| s.line != Line::Dashed)
                .count()
        };
        assert_eq!(plain(&many(9, false)), 6);
        assert_eq!(plain(&many(8, true)), 6);
        let (_, out) = series_for(&many(8, true));
        assert_eq!(out[5].key, OTHER_KEY, "Other stays, last");
        assert_eq!(out.last().unwrap().key, "usual", "references are kept");
        assert_eq!(
            plain(&many(7, false)),
            7,
            "a Weekday split keeps its seven days"
        );
    }

    #[test]
    fn donut_and_ranked_leave_out_non_positive_values() {
        let data = data(
            &["Rent", "Refunds", "Food", "Gifts", "Other"],
            vec![series(
                "spent",
                &[Some("800"), Some("-20"), Some("120.50"), Some("0"), None],
            )],
        );
        assert_eq!(
            positive_items(&data),
            (
                vec![("Rent".to_owned(), 800.0), ("Food".to_owned(), 120.5)],
                3
            )
        );
        assert_eq!(with_left_out("Last 3 months", 0), "Last 3 months");
        assert_eq!(
            with_left_out("Last 3 months", 3),
            "Last 3 months · 3 items at zero or below not shown"
        );
    }

    #[test]
    fn totals_sum_money_and_counts_but_not_averages() {
        let two = data(
            &["Jan", "Feb"],
            vec![
                series("spent", &[Some("10.10"), Some("20.20")]),
                series("count", &[Some("3"), None]),
                series("average", &[Some("5"), Some("6")]),
                reference("usual", &[Some("9"), Some("9")]),
            ],
        );
        let s = spec(&[Measure::Spent, Measure::Count, Measure::Average], None);
        assert_eq!(
            totals(&s, &two),
            Some(vec![Some(30.3), Some(3.0), None, None])
        );
        let averages = data(&["Jan"], vec![series("largest", &[Some("5")])]);
        assert_eq!(totals(&spec(&[Measure::Largest], None), &averages), None);
        let split = data(
            &["Jan"],
            vec![
                series("split:a", &[Some("1")]),
                series(OTHER_KEY, &[Some("2")]),
            ],
        );
        assert_eq!(
            totals(&spec(&[Measure::Spent], Some(Split::Category)), &split),
            Some(vec![Some(1.0), Some(2.0)])
        );
        let running = PlotSpec {
            cumulative: true,
            ..spec(&[Measure::Spent], None)
        };
        assert_eq!(
            totals(&running, &averages),
            None,
            "a running total has no sum"
        );
    }
}
