//! What the plot builder's controls do to a spec, as pure functions: each
//! [`Choice`] is applied by [`with_choice`], which repairs whatever it made
//! invalid; the menus' labels; titles; and where focus goes on closing.

use finance_types::plot::{
    Axis, Chart, Compare, Draw, Filters, Measure, MeasureSpec, Period, PeriodBasis, PlotSpec,
    Split, Value,
};
use finance_types::ui::MAX_NAME;
use rust_decimal::Decimal;

use crate::ds::arrange::in_panel;

/// One control of the builder and the value it was set to.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Measure(usize, Measure),
    Draw(usize, Draw),
    AddMeasure,
    RemoveMeasure(usize),
    By(Axis),
    Split(Option<Split>),
    Period(Period),
    Basis(PeriodBasis),
    Chart(Chart),
    Cumulative(bool),
    Compare(Compare),
    Value(Value),
    AverageLine(bool),
    VariableOnly(bool),
    Categories(Vec<String>),
    Groups(Vec<String>),
    Accounts(Vec<String>),
    Merchant(String),
    Min(Option<Decimal>),
    Max(Option<Decimal>),
    /// A currency code, or `None` for the main currency.
    Currency(Option<String>),
    Top(u8),
    SplitTop(u8),
}

/// The most measures a plot shows.
pub(super) const MAX_MEASURES: usize = 3;

/// Measures in menu order, with their names (as the query labels its series).
pub(super) const MEASURES: [(Measure, &str, &str); 7] = [
    (Measure::Spent, "spent", "Spent"),
    (Measure::Income, "income", "Income"),
    (Measure::Net, "net", "Net"),
    (Measure::Count, "count", "Purchases"),
    (Measure::Average, "average", "Average purchase"),
    (Measure::Largest, "largest", "Largest purchase"),
    (Measure::AveragePerDay, "average_per_day", "Average per day"),
];

pub(super) const AXES: [(Axis, &str, &str); 13] = [
    (Axis::Day, "day", "day"),
    (Axis::Week, "week", "week"),
    (Axis::Month, "month", "month"),
    (Axis::Quarter, "quarter", "quarter"),
    (Axis::Year, "year", "year"),
    (Axis::Weekday, "weekday", "weekday"),
    (Axis::DayOfMonth, "day_of_month", "day of month"),
    (Axis::MonthOfYear, "month_of_year", "month of year"),
    (Axis::Category, "category", "category"),
    (Axis::Group, "group", "group"),
    (Axis::Merchant, "merchant", "merchant"),
    (Axis::Account, "account", "account"),
    (
        Axis::FixedOrVariable,
        "fixed_or_variable",
        "fixed or variable",
    ),
];

pub(super) const SPLITS: [(Split, &str, &str); 7] = [
    (Split::Category, "category", "category"),
    (Split::Group, "group", "group"),
    (Split::Merchant, "merchant", "merchant"),
    (Split::Account, "account", "account"),
    (Split::Weekday, "weekday", "weekday"),
    (Split::Year, "year", "year"),
    (
        Split::FixedOrVariable,
        "fixed_or_variable",
        "fixed or variable",
    ),
];

pub(super) const PERIODS: [(&str, &str); 8] = [
    ("this_month", "This month"),
    ("last_month", "Last month"),
    ("last_3_months", "Last 3 months"),
    ("last_6_months", "Last 6 months"),
    ("last_12_months", "Last 12 months"),
    ("this_year", "This year"),
    ("all_time", "All time"),
    ("between", "Between two dates"),
];

pub(super) const CHARTS: [(Chart, &str, &str); 9] = [
    (Chart::Bars, "bars", "Bars"),
    (Chart::StackedBars, "stacked_bars", "Stacked bars"),
    (Chart::Line, "line", "Line"),
    (Chart::Area, "area", "Area"),
    (Chart::Donut, "donut", "Donut"),
    (Chart::Ranked, "ranked", "Ranked"),
    (Chart::Heatmap, "heatmap", "Heatmap"),
    (Chart::Scatter, "scatter", "Scatter"),
    (Chart::Table, "table", "Table"),
];

pub(super) const VALUES: [(Value, &str, &str); 2] = [
    (Value::Amount, "amount", "Amounts"),
    (Value::Change, "change", "Change against the period before"),
];

pub(super) const COMPARES: [(Compare, &str, &str); 3] = [
    (Compare::None, "none", "Nothing"),
    (Compare::Previous, "previous", "The period before"),
    (Compare::AverageOf3, "usual", "Usual"),
];

pub(super) fn id_of<T: PartialEq + Copy, const N: usize>(
    table: &[(T, &'static str, &str); N],
    v: T,
) -> &'static str {
    table
        .iter()
        .find(|(t, _, _)| *t == v)
        .map_or("", |(_, id, _)| id)
}

pub(super) fn from_id<T: Copy, const N: usize>(
    table: &[(T, &'static str, &str); N],
    id: &str,
) -> Option<T> {
    table.iter().find(|(_, i, _)| *i == id).map(|(t, _, _)| *t)
}

pub(super) fn name_of<T: PartialEq + Copy, const N: usize>(
    table: &[(T, &str, &'static str); N],
    v: T,
) -> &'static str {
    table
        .iter()
        .find(|(t, _, _)| *t == v)
        .map_or("", |(_, _, n)| n)
}

pub(super) fn options<T, const N: usize>(table: &[(T, &str, &str); N]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(_, id, name)| (id.to_string(), capitalised(name)))
        .collect()
}

pub(super) fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

pub(super) fn period_id(p: &Period) -> &'static str {
    match p {
        Period::ThisMonth => "this_month",
        Period::LastMonth => "last_month",
        Period::Last3Months => "last_3_months",
        Period::Last6Months => "last_6_months",
        Period::Last12Months => "last_12_months",
        Period::ThisYear => "this_year",
        Period::AllTime => "all_time",
        Period::Between { .. } => "between",
    }
}

/// The period a menu entry names; "between" starts as this month so far.
pub(super) fn period_from(id: &str, today: &str) -> Option<Period> {
    Some(match id {
        "this_month" => Period::ThisMonth,
        "last_month" => Period::LastMonth,
        "last_3_months" => Period::Last3Months,
        "last_6_months" => Period::Last6Months,
        "last_12_months" => Period::Last12Months,
        "this_year" => Period::ThisYear,
        "all_time" => Period::AllTime,
        "between" => Period::Between {
            from: format!("{}-01", today.get(..7)?),
            to: today.to_owned(),
        },
        _ => return None,
    })
}

/// `spec` with `change` applied, and whatever it made invalid repaired, so
/// the result validates and passes the query's checks before rows. Only a
/// `Between` period can still be refused: backwards, not real days, or over
/// the bucket limit for its axis.
pub fn with_choice(mut spec: PlotSpec, change: Choice) -> PlotSpec {
    match change {
        Choice::Measure(i, m) => {
            if let Some(x) = spec.measures.get_mut(i) {
                x.measure = m;
            }
        }
        Choice::Draw(i, d) => {
            if let Some(x) = spec.measures.get_mut(i) {
                x.draw = d;
            }
        }
        Choice::AddMeasure => {
            if spec.measures.len() < MAX_MEASURES {
                let unused = MEASURES
                    .iter()
                    .map(|(m, _, _)| *m)
                    .find(|m| !spec.measures.iter().any(|x| x.measure == *m))
                    .unwrap_or_default();
                spec.measures.push(MeasureSpec {
                    measure: unused,
                    draw: Draw::Bars,
                });
                spec.split = None;
            }
        }
        Choice::RemoveMeasure(i) => {
            if spec.measures.len() > 1 && i < spec.measures.len() {
                spec.measures.remove(i);
            }
        }
        Choice::By(a) => spec.by = a,
        Choice::Split(s) => {
            spec.split = s;
            if s.is_some() {
                spec.measures.truncate(1);
                spec.compare = Compare::None;
            }
        }
        Choice::Period(p) => spec.period = p,
        Choice::Basis(b) => spec.period_basis = b,
        Choice::Chart(c) => spec.chart = c,
        Choice::Cumulative(on) => {
            spec.cumulative = on;
            if on {
                spec.average_line = false;
                spec.value = Value::Amount;
            }
        }
        Choice::Compare(c) => {
            spec.compare = c;
            if c != Compare::None {
                spec.value = Value::Amount;
            }
        }
        Choice::Value(v) => {
            spec.value = v;
            if v == Value::Change {
                spec.compare = Compare::None;
                spec.cumulative = false;
            }
        }
        Choice::AverageLine(on) => {
            spec.average_line = on;
            if on {
                spec.cumulative = false;
            }
        }
        Choice::VariableOnly(on) => spec.filters.variable_only = on,
        Choice::Categories(v) => spec.filters.categories = v,
        Choice::Groups(v) => spec.filters.groups = v,
        Choice::Accounts(v) => spec.filters.accounts = v,
        Choice::Merchant(v) => spec.filters.merchant = v,
        Choice::Min(v) => spec.filters.min = v,
        Choice::Max(v) => spec.filters.max = v,
        Choice::Currency(c) => spec.currency = c,
        Choice::Top(n) => spec.top = n,
        Choice::SplitTop(n) => spec.split_top = n,
    }
    repaired(spec)
}

/// Undo, in a fixed order, whatever breaks a rule of `validate` (and the
/// query's own refusals), keeping the rest as chosen.
fn repaired(mut s: PlotSpec) -> PlotSpec {
    if s.measures.is_empty() {
        s.measures.push(MeasureSpec::default());
    }
    s.measures.truncate(MAX_MEASURES);
    if s.split.is_some_and(|sp| sp.axis() == s.by) {
        s.split = None;
    }
    if s.split.is_some() {
        s.measures.truncate(1);
    }
    s.top = s.top.clamp(1, 20);
    s.split_top = s.split_top.clamp(1, 6);
    // Running total, comparing and the average line need a time axis.
    if !s.time_axis() {
        s.cumulative = false;
        s.compare = Compare::None;
        s.average_line = false;
    }
    if s.split.is_some() {
        s.compare = Compare::None;
    }
    // All time has no period before it.
    if s.period == Period::AllTime {
        s.compare = Compare::None;
        s.value = Value::Amount;
    }
    if s.value == Value::Change
        && (!(s.named_axis() || s.time_axis()) || s.compare != Compare::None || s.cumulative)
    {
        s.value = Value::Amount;
    }
    if s.cumulative {
        s.average_line = false;
    }
    let charts = s.charts();
    if !charts.contains(&s.chart)
        && let Some(&first) = charts.first()
    {
        s.chart = first;
    }
    // Lines over bars only exist on a bar chart with several measures.
    if s.chart != Chart::Bars || s.measures.len() == 1 {
        for m in &mut s.measures {
            m.draw = Draw::Bars;
        }
    }
    s
}

/// What a repair turned off when `change` made `before` into `after`, for
/// the builder's "Turned off: …" line; what the choice itself sets is not listed.
pub fn turned_off(before: &PlotSpec, change: &Choice, after: &PlotSpec) -> Vec<&'static str> {
    let lines = |s: &PlotSpec| s.measures.iter().filter(|m| m.draw == Draw::Line).count();
    let mut off = Vec::new();
    if !matches!(change, Choice::Value(_))
        && before.value == Value::Change
        && after.value != Value::Change
    {
        off.push("change against the period before");
    }
    if !matches!(change, Choice::Compare(_))
        && before.compare != Compare::None
        && after.compare == Compare::None
    {
        off.push("comparing");
    }
    if !matches!(change, Choice::Cumulative(_)) && before.cumulative && !after.cumulative {
        off.push("running total");
    }
    if !matches!(change, Choice::AverageLine(_)) && before.average_line && !after.average_line {
        off.push("average line");
    }
    if !matches!(change, Choice::Split(_)) && before.split.is_some() && after.split.is_none() {
        off.push("the split");
    }
    if !matches!(change, Choice::RemoveMeasure(_)) && after.measures.len() < before.measures.len() {
        off.push("extra measures");
    }
    // The measures that stayed: was one of their lines reset to bars?
    let mut kept = before.clone();
    match change {
        Choice::RemoveMeasure(i) if *i < kept.measures.len() && kept.measures.len() > 1 => {
            kept.measures.remove(*i);
        }
        _ => kept.measures.truncate(after.measures.len()),
    }
    if !matches!(change, Choice::Draw(..)) && lines(after) < lines(&kept) {
        off.push("lines over the bars");
    }
    off
}

/// How many of the "Only…" filters are set.
pub fn filter_count(f: &Filters) -> usize {
    [
        !f.categories.is_empty(),
        !f.groups.is_empty(),
        !f.accounts.is_empty(),
        !f.merchant.trim().is_empty(),
        f.min.is_some(),
        f.max.is_some(),
    ]
    .into_iter()
    .filter(|&on| on)
    .count()
}

/// "Spent by weekday", "Income and Spent by month", "Spent by category, by
/// year": the measures, the axis and the split, cut to a plot name's length.
pub fn default_title(spec: &PlotSpec) -> String {
    let names: Vec<&str> = spec
        .measures
        .iter()
        .map(|m| name_of(&MEASURES, m.measure))
        .collect();
    let measures = match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let mut title = format!("{measures} by {}", name_of(&AXES, spec.by));
    if let Some(split) = spec.split {
        title += &format!(", by {}", name_of(&SPLITS, split));
    }
    title
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// `name`, or with " 2", " 3", … added when a plot already has that title;
/// the name is shortened so the result stays a valid plot name.
pub fn copy_title(name: &str, taken: &[String]) -> String {
    let free = |t: &str| !taken.iter().any(|x| x == t);
    let cut = |n: usize| {
        name.chars()
            .take(n)
            .collect::<String>()
            .trim_end()
            .to_owned()
    };
    let name = cut(MAX_NAME);
    if free(&name) {
        return name;
    }
    (2..)
        .map(|n| {
            let suffix = format!(" {n}");
            format!("{}{suffix}", cut(MAX_NAME - suffix.len()))
        })
        .find(|t| free(t))
        .unwrap_or_default()
}

/// The "Edit plot" button of a plot titled `title`.
pub fn edit_label(title: &str) -> String {
    format!("Edit plot {title}")
}

/// The "New plot" button in "Add a panel".
pub const NEW_PLOT: &str = "New plot";

/// How the builder closed.
#[derive(Clone, Debug, PartialEq)]
pub enum Closed {
    /// Saved (over the plot, or as a new one) and shown in panel `panel`.
    Saved {
        panel: String,
        title: String,
    },
    Cancelled,
    Deleted,
}

/// The selector of the control that takes focus when the builder closes:
/// the saved plot's "Edit plot", the control that opened it after a cancel,
/// and after a delete "New plot" in Edit layout, else the tab shown.
pub fn focus_after_builder(closed: &Closed, opener: &str, editing: bool) -> String {
    match closed {
        Closed::Saved { panel, title } => in_panel(panel, &edit_label(title)),
        Closed::Cancelled => opener.to_owned(),
        Closed::Deleted if editing => format!("button[aria-label=\"{NEW_PLOT}\"]"),
        Closed::Deleted => "[role=tab][aria-selected=true]".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use finance_types::plot::refusal_before_rows;

    use super::*;

    fn m(measure: Measure) -> MeasureSpec {
        MeasureSpec {
            measure,
            draw: Draw::Bars,
        }
    }

    fn base() -> PlotSpec {
        PlotSpec::default()
    }

    /// Six specs that cover every chart family and option.
    fn samples() -> Vec<PlotSpec> {
        let mut out: Vec<PlotSpec> = finance_types::plot::ready_made()
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        out.push(PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Count)],
            by: Axis::Week,
            chart: Chart::Scatter,
            ..base()
        });
        out.push(PlotSpec {
            period: Period::Between {
                from: "2026-01-01".into(),
                to: "2026-06-30".into(),
            },
            by: Axis::Week,
            average_line: true,
            ..base()
        });
        assert!(out.len() >= 6);
        out
    }

    fn every_choice() -> Vec<Choice> {
        let mut c = vec![
            Choice::AddMeasure,
            Choice::RemoveMeasure(0),
            Choice::RemoveMeasure(1),
        ];
        for (measure, _, _) in MEASURES {
            c.push(Choice::Measure(0, measure));
            c.push(Choice::Measure(1, measure));
        }
        c.extend([Choice::Draw(0, Draw::Line), Choice::Draw(1, Draw::Bars)]);
        c.extend(AXES.iter().map(|(a, _, _)| Choice::By(*a)));
        c.push(Choice::Split(None));
        c.extend(SPLITS.iter().map(|(s, _, _)| Choice::Split(Some(*s))));
        c.extend(
            PERIODS
                .iter()
                .filter_map(|(id, _)| period_from(id, "2026-11-24"))
                .map(Choice::Period),
        );
        c.extend([
            Choice::Basis(PeriodBasis::Budget),
            Choice::Basis(PeriodBasis::Calendar),
        ]);
        c.extend(CHARTS.iter().map(|(ch, _, _)| Choice::Chart(*ch)));
        c.extend(COMPARES.iter().map(|(x, _, _)| Choice::Compare(*x)));
        c.extend(VALUES.iter().map(|(v, _, _)| Choice::Value(*v)));
        for on in [true, false] {
            c.extend([
                Choice::Cumulative(on),
                Choice::AverageLine(on),
                Choice::VariableOnly(on),
            ]);
        }
        c.extend([
            Choice::Categories(vec!["Food".into()]),
            Choice::Groups(vec!["Fun".into()]),
            Choice::Accounts(vec!["a1".into()]),
            Choice::Merchant("albert".into()),
            Choice::Min(Some(Decimal::ONE)),
            Choice::Max(Some(Decimal::TEN)),
            Choice::Currency(Some("USD".into())),
            Choice::Currency(None),
            Choice::Top(5),
            Choice::Top(0),
            Choice::Top(30),
            Choice::SplitTop(3),
            Choice::SplitTop(9),
        ]);
        c
    }

    #[test]
    fn every_result_validates() {
        for spec in samples() {
            for choice in every_choice() {
                let out = with_choice(spec.clone(), choice.clone());
                assert_eq!(out.validate(), Ok(()), "{choice:?} on {spec:?}");
                assert_eq!(refusal_before_rows(&out), None, "{choice:?} on {spec:?}");
                // And again on the result: two choices in a row stay valid.
                for second in [
                    Choice::Chart(Chart::Line),
                    Choice::Split(Some(Split::Group)),
                ] {
                    assert_eq!(
                        with_choice(out.clone(), second.clone()).validate(),
                        Ok(()),
                        "{choice:?} then {second:?} on {spec:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn default_titles() {
        let weekday = PlotSpec {
            by: Axis::Weekday,
            ..base()
        };
        assert_eq!(default_title(&weekday), "Spent by weekday");
        let two = PlotSpec {
            measures: vec![m(Measure::Income), m(Measure::Spent)],
            ..base()
        };
        assert_eq!(default_title(&two), "Income and Spent by month");
        let split = PlotSpec {
            by: Axis::Category,
            split: Some(Split::Year),
            ..base()
        };
        assert_eq!(default_title(&split), "Spent by category, by year");
        let three = PlotSpec {
            measures: vec![m(Measure::Income), m(Measure::Spent), m(Measure::Net)],
            ..base()
        };
        assert_eq!(default_title(&three), "Income, Spent and Net by month");
        let long = PlotSpec {
            measures: vec![
                m(Measure::Average),
                m(Measure::Largest),
                m(Measure::AveragePerDay),
            ],
            by: Axis::FixedOrVariable,
            ..base()
        };
        let t = default_title(&long);
        assert!(t.chars().count() <= MAX_NAME && !t.ends_with(' '), "{t}");
    }

    #[test]
    fn choosing_a_named_axis_drops_time_only_options() {
        let running = PlotSpec {
            by: Axis::Day,
            period: Period::ThisMonth,
            chart: Chart::Line,
            cumulative: true,
            compare: Compare::AverageOf3,
            ..base()
        };
        let out = with_choice(running, Choice::By(Axis::Merchant));
        assert!(!out.cumulative);
        assert_eq!(out.compare, Compare::None);
        assert_eq!(out.chart, Chart::Bars, "a line needs a time axis");
        let average = PlotSpec {
            average_line: true,
            ..base()
        };
        assert!(!with_choice(average, Choice::By(Axis::Weekday)).average_line);
    }

    #[test]
    fn choosing_a_split_drops_extra_measures_and_compare() {
        let s = PlotSpec {
            measures: vec![m(Measure::Income), m(Measure::Spent)],
            compare: Compare::Previous,
            ..base()
        };
        let out = with_choice(s, Choice::Split(Some(Split::Group)));
        assert_eq!(out.measures, vec![m(Measure::Income)]);
        assert_eq!(out.compare, Compare::None);
        assert_eq!(out.split, Some(Split::Group));
        // A split equal to the new axis goes.
        assert_eq!(with_choice(out, Choice::By(Axis::Group)).split, None);
        // Adding a measure ends the split.
        let split = PlotSpec {
            split: Some(Split::Category),
            ..base()
        };
        let added = with_choice(split, Choice::AddMeasure);
        assert_eq!((added.split, added.measures.len()), (None, 2));
        assert_eq!(
            added.measures[1].measure,
            Measure::Income,
            "the first unused"
        );
    }

    #[test]
    fn an_invalid_chart_falls_back_to_the_first_that_fits() {
        let ranked = PlotSpec {
            by: Axis::Category,
            chart: Chart::Ranked,
            ..base()
        };
        // Back on a time axis: Ranked no longer fits, Bars is first.
        assert_eq!(
            with_choice(ranked.clone(), Choice::By(Axis::Month)).chart,
            Chart::Bars
        );
        // Counts mixed with amounts: only Scatter or Table.
        let mixed = with_choice(
            PlotSpec {
                measures: vec![m(Measure::Spent), m(Measure::Income)],
                ..base()
            },
            Choice::Measure(1, Measure::Count),
        );
        assert_eq!(mixed.chart, Chart::Scatter);
        let heat = PlotSpec {
            by: Axis::Week,
            split: Some(Split::Weekday),
            chart: Chart::Heatmap,
            ..base()
        };
        assert_eq!(with_choice(heat, Choice::Split(None)).chart, Chart::Bars);
        // A chart that fits is kept.
        assert_eq!(
            with_choice(ranked, Choice::Merchant("x".into())).chart,
            Chart::Ranked
        );
    }

    #[test]
    fn choosing_scatter_is_only_offered_with_two_measures() {
        let one = base();
        assert!(!one.charts().contains(&Chart::Scatter));
        assert_eq!(
            with_choice(one.clone(), Choice::Chart(Chart::Scatter)).chart,
            Chart::Bars
        );
        let two = with_choice(one, Choice::AddMeasure);
        assert!(two.charts().contains(&Chart::Scatter));
        assert_eq!(
            with_choice(two.clone(), Choice::Chart(Chart::Scatter)).chart,
            Chart::Scatter
        );
        let three = with_choice(two, Choice::AddMeasure);
        assert!(!three.charts().contains(&Chart::Scatter));
    }

    #[test]
    fn options_that_clash_clear_each_other() {
        let s = PlotSpec {
            by: Axis::Day,
            period: Period::ThisMonth,
            ..base()
        };
        let running = with_choice(s.clone(), Choice::Cumulative(true));
        let average = with_choice(running.clone(), Choice::AverageLine(true));
        assert!(average.average_line && !average.cumulative);
        let running = with_choice(average, Choice::Cumulative(true));
        assert!(running.cumulative && !running.average_line);
        // All time has nothing before it.
        let compared = with_choice(s, Choice::Compare(Compare::Previous));
        assert_eq!(compared.compare, Compare::Previous);
        assert_eq!(
            with_choice(compared, Choice::Period(Period::AllTime)).compare,
            Compare::None
        );
        // Change goes with comparing, a running total or an axis without it.
        let change = PlotSpec {
            by: Axis::Category,
            value: Value::Change,
            chart: Chart::Ranked,
            period: Period::Last3Months,
            ..base()
        };
        assert_eq!(
            with_choice(change.clone(), Choice::By(Axis::Weekday)).value,
            Value::Amount
        );
        assert_eq!(
            with_choice(change.clone(), Choice::Period(Period::AllTime)).value,
            Value::Amount
        );
        assert_eq!(
            with_choice(change, Choice::By(Axis::Month)).value,
            Value::Change
        );
        // Filters are kept as typed.
        let f = with_choice(base(), Choice::Min(Some(Decimal::ONE)));
        assert_eq!(
            f.filters,
            Filters {
                min: Some(Decimal::ONE),
                ..Filters::default()
            }
        );
        // The last measure stays.
        assert_eq!(
            with_choice(base(), Choice::RemoveMeasure(0)).measures.len(),
            1
        );
    }

    #[test]
    fn a_ready_made_copy_gets_a_free_title() {
        let taken = vec!["Top merchants".to_owned(), "Top merchants 2".to_owned()];
        assert_eq!(copy_title("Top merchants", &taken), "Top merchants 3");
        assert_eq!(copy_title("Year on year", &taken), "Year on year");
        assert_eq!(copy_title("Top merchants", &[]), "Top merchants");
        let long = "x".repeat(MAX_NAME);
        let copy = copy_title(&long, std::slice::from_ref(&long));
        assert_eq!(copy.chars().count(), MAX_NAME);
        assert!(copy.ends_with("x 2"), "{copy}");
    }

    #[test]
    fn a_repair_names_what_it_turned_off() {
        let change = PlotSpec {
            by: Axis::Category,
            value: Value::Change,
            chart: Chart::Ranked,
            ..base()
        };
        let c = Choice::By(Axis::Weekday);
        let after = with_choice(change.clone(), c.clone());
        assert_eq!(
            turned_off(&change, &c, &after),
            ["change against the period before"]
        );
        // What the choice itself sets is not a repair.
        let c = Choice::Value(Value::Amount);
        assert!(turned_off(&change, &c, &with_choice(change.clone(), c.clone())).is_empty());
        let two = PlotSpec {
            measures: vec![m(Measure::Income), m(Measure::Spent)],
            compare: Compare::Previous,
            ..base()
        };
        let c = Choice::Split(Some(Split::Group));
        assert_eq!(
            turned_off(&two, &c, &with_choice(two.clone(), c.clone())),
            ["comparing", "extra measures"]
        );
        // A measure drawn as a line goes back to bars off a bar chart.
        let mut lined = two.clone();
        lined.compare = Compare::None;
        lined.measures[1].draw = Draw::Line;
        let c = Choice::Chart(Chart::Line);
        let after = with_choice(lined.clone(), c.clone());
        assert!(after.measures.iter().all(|m| m.draw == Draw::Bars));
        assert_eq!(turned_off(&lined, &c, &after), ["lines over the bars"]);
    }

    #[test]
    fn change_is_offered_only_where_it_survives() {
        let named = PlotSpec {
            by: Axis::Category,
            period: Period::Last3Months,
            ..base()
        };
        assert_eq!(
            with_choice(named, Choice::Value(Value::Change)).value,
            Value::Change
        );
        let running = PlotSpec {
            cumulative: true,
            compare: Compare::Previous,
            ..base()
        };
        let out = with_choice(running, Choice::Value(Value::Change));
        assert_eq!(
            (out.value, out.cumulative, out.compare),
            (Value::Change, false, Compare::None)
        );
        let weekday = PlotSpec {
            by: Axis::Weekday,
            ..base()
        };
        assert_eq!(
            with_choice(weekday, Choice::Value(Value::Change)).value,
            Value::Amount
        );
        let all = PlotSpec {
            period: Period::AllTime,
            ..base()
        };
        assert_eq!(
            with_choice(all, Choice::Value(Value::Change)).value,
            Value::Amount
        );
    }

    #[test]
    fn filters_count_what_is_set() {
        assert_eq!(filter_count(&Filters::default()), 0);
        let f = Filters {
            categories: vec!["Food".into()],
            merchant: "albert".into(),
            min: Some(Decimal::ONE),
            variable_only: true,
            ..Filters::default()
        };
        assert_eq!(filter_count(&f), 3, "variable only has its own switch");
    }

    #[test]
    fn focus_after_the_builder_closes() {
        let saved = Closed::Saved {
            panel: "plot:p2".into(),
            title: "Food".into(),
        };
        assert_eq!(
            focus_after_builder(&saved, "x", false),
            in_panel("plot:p2", "Edit plot Food")
        );
        assert_eq!(
            focus_after_builder(&Closed::Cancelled, "#opener", true),
            "#opener"
        );
        assert_eq!(
            focus_after_builder(&Closed::Deleted, "#opener", true),
            "button[aria-label=\"New plot\"]"
        );
        assert_eq!(
            focus_after_builder(&Closed::Deleted, "#opener", false),
            "[role=tab][aria-selected=true]"
        );
    }

    #[test]
    fn currency_and_how_many_are_set_and_kept_in_range() {
        let usd = with_choice(base(), Choice::Currency(Some("USD".into())));
        assert_eq!(usd.currency.as_deref(), Some("USD"));
        assert_eq!(with_choice(usd, Choice::Currency(None)).currency, None);
        assert_eq!(with_choice(base(), Choice::Top(15)).top, 15);
        assert_eq!(with_choice(base(), Choice::SplitTop(4)).split_top, 4);
    }

    #[test]
    fn a_single_measure_is_drawn_as_bars() {
        let two = PlotSpec {
            measures: vec![
                MeasureSpec {
                    measure: Measure::Spent,
                    draw: Draw::Line,
                },
                m(Measure::Income),
            ],
            ..base()
        };
        // Either measure can go; the one left is drawn as bars.
        let first_gone = with_choice(two.clone(), Choice::RemoveMeasure(0));
        assert_eq!(first_gone.measures, vec![m(Measure::Income)]);
        let c = Choice::RemoveMeasure(1);
        let second_gone = with_choice(two.clone(), c.clone());
        assert_eq!(second_gone.measures, vec![m(Measure::Spent)]);
        assert_eq!(turned_off(&two, &c, &second_gone), ["lines over the bars"]);
        // Measures cut and a line reset in one choice: both are named.
        let c = Choice::Split(Some(Split::Group));
        let split = with_choice(two.clone(), c.clone());
        assert_eq!(
            turned_off(&two, &c, &split),
            ["extra measures", "lines over the bars"]
        );
    }
}
