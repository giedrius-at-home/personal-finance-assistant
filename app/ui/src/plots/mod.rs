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
    label_step, series_color,
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
/// The seventh day of a Weekday split, past the six-colour palette: the
/// neutral axis colour (not a semantic one; references are told apart by their dashes).
const SEVENTH_COLOR: &str = "var(--ds-chart-axis)";
/// The most named slices a donut draws; the rest are added to "Other".
const DONUT_NAMED: usize = 5;
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

/// What a donut or ranked chart draws from the first series.
#[derive(Debug, Default, PartialEq)]
pub struct Items {
    /// The positive named items, in the query's order.
    pub named: Vec<(String, Decimal)>,
    /// "Other": the query's own plus named items folded in here, summed exactly.
    pub other: Option<Decimal>,
    /// How many items "Other" holds.
    pub in_other: usize,
    /// Items at zero or below, not shown.
    pub not_positive: usize,
    /// Items with no value, not shown.
    pub missing: usize,
}

/// The positive items of the first series; past `keep` named ones, the rest
/// are added to "Other". The query's "Other" is its last column, there exactly
/// when it left items out (`left_out` > 0; the axis carries no keys).
pub fn positive_items(data: &PlotData, keep: Option<usize>) -> Items {
    let mut out = Items::default();
    let Some(first) = data.series.iter().find(|s| !s.reference) else {
        return out;
    };
    let other_at = (data.left_out > 0).then(|| data.categories.len().saturating_sub(1));
    for (i, (name, v)) in data.categories.iter().zip(&first.values).enumerate() {
        match *v {
            None => out.missing += 1,
            Some(v) if v <= Decimal::ZERO => out.not_positive += 1,
            Some(v) if Some(i) == other_at => {
                out.other = Some(out.other.unwrap_or_default() + v);
                out.in_other += data.left_out;
            }
            Some(v) if keep.is_some_and(|k| out.named.len() >= k) => {
                out.other = Some(out.other.unwrap_or_default() + v);
                out.in_other += 1;
            }
            Some(v) => out.named.push((name.clone(), v)),
        }
    }
    out
}

/// The query's subtitle, plus what the chart leaves out: items in "Other",
/// and (donut, ranked) items at zero or below or with no value.
pub fn subtitle_for(spec: &PlotSpec, data: &PlotData) -> String {
    let items = match spec.chart {
        Chart::Donut => positive_items(data, Some(DONUT_NAMED)),
        Chart::Ranked if spec.value != Value::Change => positive_items(data, None),
        _ => Items {
            in_other: data.left_out,
            ..Items::default()
        },
    };
    let mut parts = vec![data.subtitle.clone()];
    parts.retain(|p| !p.is_empty());
    if items.in_other > 0 {
        parts.push(format!("{} more in Other", items.in_other));
    }
    if items.not_positive > 0 {
        parts.push(format!(
            "{} at zero or below not shown",
            plural(items.not_positive, "item")
        ));
    }
    if items.missing > 0 {
        parts.push(format!(
            "{} with no value not shown",
            plural(items.missing, "item")
        ));
    }
    parts.join(" · ")
}

/// The totals row of the numbers table: a sum per series that adds up
/// (spent, income, net, count); none for averages, largest, references or a
/// running total or a change. None when no column has one.
pub fn totals(spec: &PlotSpec, data: &PlotData) -> Option<Vec<Option<f64>>> {
    // A running total or a change has no meaningful sum.
    if spec.cumulative || spec.value == Value::Change {
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

/// Nothing to draw: no buckets, or no plain value at all (zeros are drawn).
fn is_empty(data: &PlotData) -> bool {
    data.categories.is_empty()
        || data
            .series
            .iter()
            .filter(|s| !s.reference)
            .all(|s| s.values.iter().all(Option::is_none))
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
fn data_table(spec: &PlotSpec, data: &PlotData, label: String) -> AnyView {
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
            label=label
        />
    }
    .into_any()
}

/// The chart `spec.chart` names, over `data`.
fn drawing(spec: &PlotSpec, data: &PlotData, label: String) -> AnyView {
    let format = unit_fmt(data.unit, &data.currency);
    let titles = data.titles.clone();
    let (categories, series) = series_for(data);
    match spec.chart {
        Chart::Bars | Chart::StackedBars => {
            // Measures drawn as lines, and reference series, go over the bars on the same axis.
            let (lines, bars): (Vec<Series>, Vec<Series>) =
                series.into_iter().partition(|s| s.line != Line::Smooth);
            let label_every = label_step(&categories);
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
                    label_every=label_every
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
            let label_every = label_step(&categories);
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
            let items = positive_items(data, Some(DONUT_NAMED));
            let mut slices: Vec<Slice> = items
                .named
                .into_iter()
                .enumerate()
                .map(|(i, (name, value))| Slice {
                    key: format!("slice-{i}"),
                    color: series_color(i, None),
                    label: name,
                    value: f(value),
                })
                .collect();
            slices.extend(items.other.map(|value| Slice {
                key: OTHER_KEY.into(),
                label: "Other".into(),
                value: f(value),
                color: OTHER_COLOR.into(),
            }));
            view! { <Donut data=slices format=format label=label /> }.into_any()
        }
        Chart::Ranked if spec.value == Value::Change => {
            let values = series.first().map_or(&[][..], |s| &s.values[..]);
            let items: Vec<(String, f64)> = categories
                .iter()
                .zip(values)
                .filter_map(|(c, v)| Some((c.clone(), (*v)?)))
                .collect();
            // The query's "Other" is its last column when it left items out.
            let other_last = data.left_out > 0
                && values.len() == categories.len()
                && values.last().is_some_and(Option::is_some);
            view! { <Ranked items=items format=format two_sided=true label=label other_last=other_last /> }.into_any()
        }
        Chart::Ranked => {
            let items = positive_items(data, None);
            let other_last = items.other.is_some();
            let rows = items
                .named
                .into_iter()
                .map(|(name, v)| (name, f(v)))
                .chain(items.other.map(|v| ("Other".to_owned(), f(v))))
                .collect();
            view! { <Ranked items=rows format=format label=label other_last=other_last /> }
                .into_any()
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
            view! {
                <Heatmap
                    rows=categories
                    columns=columns
                    values=values
                    format=format
                    label=label
                    partial_last=data.partial_last
                />
            }
            .into_any()
        }
        Chart::Scatter => {
            let plain: Vec<&PlotSeries> = data.series.iter().filter(|s| !s.reference).collect();
            let (Some(x), Some(y)) = (plain.first(), plain.get(1)) else {
                return view! { <EmptyState title="Nothing in this period" /> }.into_any();
            };
            let at = |s: &PlotSeries, i: usize| s.values.get(i).copied().flatten().map(f);
            let points: Vec<_> = data
                .titles
                .iter()
                .enumerate()
                .filter_map(|(i, name)| Some((name.clone(), at(x, i)?, at(y, i)?)))
                .collect();
            // The unfinished bucket is the last one, when it has a point.
            let last = data.titles.len().checked_sub(1);
            let partial_last =
                data.partial_last && last.is_some_and(|i| at(x, i).is_some() && at(y, i).is_some());
            view! {
                <Scatter
                    points=points
                    x_format=unit_fmt(x.unit, &data.currency)
                    y_format=unit_fmt(y.unit, &data.currency)
                    x_label=x.label.clone()
                    y_label=y.label.clone()
                    partial_last=partial_last
                />
            }
            .into_any()
        }
        Chart::Table => data_table(spec, data, label),
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
    /// The plot's name, for the chart's and table's accessible label.
    #[prop(optional, into)]
    title: Signal<String>,
) -> impl IntoView {
    let revision = crate::state().revision;
    let loaded = api::load::<PlotData, _>("plot_query", move || {
        revision.track();
        QueryArgs { spec: spec.get() }
    });
    if let Some(insight) = insight {
        Effect::new(move |_| {
            loaded.data.with(|d| match d {
                Some(Ok(d)) => insight.set(d.insight.as_ref().map(|i| i.text.clone())),
                // A refused reload: no stale insight in the folded row.
                Some(Err(_)) => insight.set(None),
                None => {}
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
                let subtitle = subtitle_for(&spec, d);
                let insight = d.insight.as_ref().map(|i| view! { <p class=PLOT_INSIGHT>{i.text.clone()}</p> });
                let data = d.clone();
                let fallback = subtitle.clone();
                let body = move || {
                    // Without a title the period and filters name the chart.
                    let label = Some(title.get()).filter(|t| !t.is_empty()).unwrap_or_else(|| fallback.clone());
                    if table_only || numbers.get() {
                        data_table(&spec, &data, label)
                    } else {
                        drawing(&spec, &data, label)
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
            &["Rent", "Refunds", "Food", "Gifts", "Lost"],
            vec![series(
                "spent",
                &[Some("800"), Some("-20"), Some("120.50"), Some("0"), None],
            )],
        );
        let items = positive_items(&data, None);
        assert_eq!(
            items.named,
            [
                ("Rent".to_owned(), d("800")),
                ("Food".to_owned(), d("120.50"))
            ]
        );
        assert_eq!(
            (items.other, items.not_positive, items.missing),
            (None, 2, 1)
        );
        let ranked = PlotSpec {
            chart: Chart::Ranked,
            ..spec(&[Measure::Spent], None)
        };
        assert_eq!(
            subtitle_for(&ranked, &data),
            "2 items at zero or below not shown · 1 item with no value not shown"
        );
    }

    #[test]
    fn a_donut_folds_past_five_into_other_with_the_exact_sum() {
        let mut names: Vec<String> = (1..=10).map(|i| format!("c{i}")).collect();
        names.push("Other".into());
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let values = [
            "50", "40", "30", "20", "10", "0.10", "0.20", "0.30", "0.40", "0.50", "7.01",
        ];
        let mut data = data(&names, vec![series("spent", &values.map(Some))]);
        data.left_out = 12;
        let items = positive_items(&data, Some(DONUT_NAMED));
        assert_eq!(items.named.len(), 5);
        assert_eq!(items.named[4], ("c5".to_owned(), d("10")));
        assert_eq!(
            items.other,
            Some(d("8.51")),
            "0.1+0.2+0.3+0.4+0.5+7.01, exactly"
        );
        assert_eq!(items.in_other, 17, "five folded here, twelve by the query");
        let donut = PlotSpec {
            chart: Chart::Donut,
            ..spec(&[Measure::Spent], None)
        };
        assert_eq!(subtitle_for(&donut, &data), "17 more in Other");
        let bars = spec(&[Measure::Spent], None);
        assert_eq!(subtitle_for(&bars, &data), "12 more in Other");
        data.subtitle = "Last 6 months".into();
        assert_eq!(
            subtitle_for(&bars, &data),
            "Last 6 months · 12 more in Other"
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
        let change = PlotSpec {
            value: Value::Change,
            ..spec(&[Measure::Spent], None)
        };
        assert_eq!(totals(&change, &averages), None, "a change has no sum");
    }
}
