//! The plot builder: a popup where the owner picks what a plot shows, sees it
//! drawn as he goes, and saves it as a panel. Every control goes through
//! [`with_choice`], which keeps the spec one that validates.

use std::time::Duration;

use finance_types::plot::{
    Axis, Chart, Compare, Draw, Measure, MeasureSpec, Period, PeriodBasis, PlotSpec, Split, Value,
};
use finance_types::ui::MAX_NAME;
use leptos::html;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::PlotView;
use crate::ds::arrange::in_panel;
use crate::ds::builder::{ChartChips, MeasureRow};
use crate::ds::patterns::{
    ACTIONS, BUILDER, BUILDER_FIELD, BUILDER_ROW, ERROR_LINE, FIELD, FIELD_LABEL, FILTER_ROW,
    SUBPANEL,
};
use crate::ds::{
    Button, Confirm, Field, Icon, Input, Lucide, MultiSelect, Popup, SegmentedControl, Select,
    Size, Switch, Variant, list_names,
};

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
    AverageLine(bool),
    VariableOnly(bool),
    Categories(Vec<String>),
    Groups(Vec<String>),
    Accounts(Vec<String>),
    Merchant(String),
    Min(Option<Decimal>),
    Max(Option<Decimal>),
}

/// The most measures a plot shows.
const MAX_MEASURES: usize = 3;

/// Measures in menu order, with their names (as the query labels its series).
const MEASURES: [(Measure, &str, &str); 7] = [
    (Measure::Spent, "spent", "Spent"),
    (Measure::Income, "income", "Income"),
    (Measure::Net, "net", "Net"),
    (Measure::Count, "count", "Purchases"),
    (Measure::Average, "average", "Average purchase"),
    (Measure::Largest, "largest", "Largest purchase"),
    (Measure::AveragePerDay, "average_per_day", "Average per day"),
];

const AXES: [(Axis, &str, &str); 13] = [
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

const SPLITS: [(Split, &str, &str); 7] = [
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

const PERIODS: [(&str, &str); 8] = [
    ("this_month", "This month"),
    ("last_month", "Last month"),
    ("last_3_months", "Last 3 months"),
    ("last_6_months", "Last 6 months"),
    ("last_12_months", "Last 12 months"),
    ("this_year", "This year"),
    ("all_time", "All time"),
    ("between", "Between two dates"),
];

const CHARTS: [(Chart, &str, &str); 9] = [
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

const COMPARES: [(Compare, &str, &str); 3] = [
    (Compare::None, "none", "Nothing"),
    (Compare::Previous, "previous", "The period before"),
    (Compare::AverageOf3, "usual", "Usual"),
];

fn id_of<T: PartialEq + Copy, const N: usize>(
    table: &[(T, &'static str, &str); N],
    v: T,
) -> &'static str {
    table
        .iter()
        .find(|(t, _, _)| *t == v)
        .map_or("", |(_, id, _)| id)
}

fn from_id<T: Copy, const N: usize>(table: &[(T, &'static str, &str); N], id: &str) -> Option<T> {
    table.iter().find(|(_, i, _)| *i == id).map(|(t, _, _)| *t)
}

fn name_of<T: PartialEq + Copy, const N: usize>(
    table: &[(T, &str, &'static str); N],
    v: T,
) -> &'static str {
    table
        .iter()
        .find(|(t, _, _)| *t == v)
        .map_or("", |(_, _, n)| n)
}

fn options<T, const N: usize>(table: &[(T, &str, &str); N]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(_, id, name)| (id.to_string(), capitalised(name)))
        .collect()
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn period_id(p: &Period) -> &'static str {
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
fn period_from(id: &str, today: &str) -> Option<Period> {
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
/// the result validates. The owner's own dates in `Between` are the only
/// thing left to `validate` to refuse (backwards, or too many buckets).
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
    s
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

/// `name`, or with " 2", " 3", … added when a plot already has that title.
pub fn copy_title(name: &str, taken: &[String]) -> String {
    let free = |t: &str| !taken.iter().any(|x| x == t);
    if free(name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
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

/// Over the plot being edited, or as a new plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saving {
    Over,
    New,
}

/// What the filters may pick from: category and group names, accounts as (id, name).
#[derive(Clone, Debug, Default)]
pub struct FilterOptions {
    pub categories: Vec<String>,
    pub groups: Vec<String>,
    pub accounts: Vec<(String, String)>,
}

/// The wait after the last change before the preview reloads.
const PREVIEW_DELAY: Duration = Duration::from_millis(250);

/// A select kept equal to `value(spec)`; a pick runs `on_pick`.
fn bound(
    spec: RwSignal<PlotSpec>,
    value: impl Fn(&PlotSpec) -> String + Send + Sync + 'static,
) -> RwSignal<String> {
    let v = RwSignal::new(spec.with_untracked(&value));
    Effect::new(move |_| {
        let now = spec.with(&value);
        if v.get_untracked() != now {
            v.set(now);
        }
    });
    v
}

/// A text field whose text, once changed, is applied as `choice(text)`.
fn applied(
    initial: String,
    choose: impl Fn(Choice) + Copy + Send + Sync + 'static,
    choice: impl Fn(String) -> Option<Choice> + Send + Sync + 'static,
) -> RwSignal<String> {
    let v = RwSignal::new(initial.clone());
    Effect::new(move |was: Option<String>| {
        let now = v.get();
        if was.unwrap_or_else(|| initial.clone()) != now
            && let Some(c) = choice(now.clone())
        {
            choose(c);
        }
        now
    });
    v
}

fn amount(text: &str) -> Option<Decimal> {
    text.trim().parse().ok()
}

fn amount_text(v: Option<Decimal>) -> String {
    v.map(|d| d.to_string()).unwrap_or_default()
}

/// The builder in a popup. `name` is the saved title (empty for a new plot);
/// `on_save` returns why it refused; `on_close` runs on Cancel, Esc and the
/// close button.
#[component]
pub fn PlotBuilder(
    #[prop(into)] heading: String,
    name: String,
    spec: PlotSpec,
    /// Editing a saved plot: also "Save as new" and "Delete".
    existing: bool,
    filter_options: FilterOptions,
    on_save: Callback<(Saving, String, PlotSpec), Result<(), String>>,
    on_delete: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let first = spec.clone();
    let spec = RwSignal::new(spec);
    let choose = move |c: Choice| {
        let now = spec.get_untracked();
        let next = with_choice(now.clone(), c);
        if next != now {
            spec.set(next);
        }
    };
    // The preview follows the spec once it has been still for a moment.
    let shown = RwSignal::new(first.clone());
    let generation = StoredValue::new(0_u32);
    Effect::new(move |_| {
        let s = spec.get();
        generation.update_value(|g| *g += 1);
        let mine = generation.get_value();
        set_timeout(
            move || {
                if generation.try_get_value() == Some(mine) && shown.with_untracked(|v| *v != s) {
                    shown.try_set(s);
                }
            },
            PREVIEW_DELAY,
        );
    });
    // The title follows the spec until the owner types his own.
    let follows = StoredValue::new(name.is_empty() || name == default_title(&first));
    let title = RwSignal::new(if name.is_empty() {
        default_title(&first)
    } else {
        name
    });
    Effect::new(move |_| {
        let t = default_title(&spec.get());
        if follows.get_value() && title.get_untracked() != t {
            title.set(t);
        }
    });
    let refused = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        spec.track();
        title.track();
        refused.set(None);
    });
    let save = move |how: Saving| {
        let s = spec.get_untracked();
        // The reason is already under the preview; say it is why Save did nothing.
        let out = s
            .validate()
            .map_err(|why| format!("Can't save yet. {why}"))
            .and_then(|()| on_save.run((how, title.get_untracked(), s)));
        if let Err(why) = out {
            refused.set(Some(why));
        }
    };

    // Row one: Show … by … split by … over ….
    let today = crate::pages::wishes::today();
    let shape = Memo::new(move |_| {
        spec.with(|s| (s.measures.len(), s.split.is_none(), s.chart == Chart::Bars))
    });
    let measures = move || {
        let (n, unsplit, bars) = shape.get();
        let rows = (0..n)
            .map(|i| {
                let value = bound(spec, move |s| {
                    s.measures
                        .get(i)
                        .map_or("", |m| id_of(&MEASURES, m.measure))
                        .to_owned()
                });
                let draw = Signal::derive(move || {
                    spec.with(|s| match s.measures.get(i).map(|m| m.draw) {
                        Some(Draw::Line) => "line".to_owned(),
                        _ => "bars".to_owned(),
                    })
                });
                let (choices, draw, removable) = (options(&MEASURES), (bars && n > 1).then_some(draw), n > 1);
                view! {
                    <MeasureRow
                        index=i
                        value=value
                        options=choices
                        draw=draw
                        removable=removable
                        on_pick=Callback::new(move |v: String| {
                            if let Some(m) = from_id(&MEASURES, &v) {
                                choose(Choice::Measure(i, m));
                            }
                        })
                        on_draw=Callback::new(move |v: String| {
                            choose(Choice::Draw(i, if v == "line" { Draw::Line } else { Draw::Bars }));
                        })
                        on_remove=Callback::new(move |()| choose(Choice::RemoveMeasure(i)))
                    />
                }
            })
            .collect_view();
        let add = (unsplit && n < MAX_MEASURES).then(|| {
            view! {
                <Button variant=Variant::Ghost size=Size::Sm on_click=Callback::new(move |_| choose(Choice::AddMeasure))>
                    <Lucide icon=Icon::Plus size=14 />
                    "Add a measure"
                </Button>
            }
        });
        view! { {rows} {add} }
    };
    let by = bound(spec, |s| id_of(&AXES, s.by).to_owned());
    let split = bound(spec, |s| {
        s.split.map_or("none", |sp| id_of(&SPLITS, sp)).to_owned()
    });
    let split_options = Memo::new(move |_| {
        let axis = spec.with(|s| s.by);
        std::iter::once(("none".to_owned(), "Nothing".to_owned()))
            .chain(
                SPLITS
                    .iter()
                    .filter(|(sp, _, _)| sp.axis() != axis)
                    .map(|(_, id, name)| (id.to_string(), capitalised(name))),
            )
            .collect::<Vec<_>>()
    });
    let split_options = Signal::from(split_options);
    let period = bound(spec, |s| period_id(&s.period).to_owned());
    let between = Memo::new(move |_| spec.with(|s| matches!(s.period, Period::Between { .. })));
    let dates = move || {
        spec.with_untracked(|s| match &s.period {
            Period::Between { from, to } => (from.clone(), to.clone()),
            _ => Default::default(),
        })
    };
    let date_fields = move || {
        between.get().then(|| {
            let (from, to) = dates();
            let set = move |from: Option<String>, to: Option<String>| {
                let (f, t) = dates();
                Some(Choice::Period(Period::Between {
                    from: from.unwrap_or(f),
                    to: to.unwrap_or(t),
                }))
            };
            let from = applied(from, choose, move |v| set(Some(v), None));
            let to = applied(to, choose, move |v| set(None, Some(v)));
            view! {
                <Field label="From" id="plot-from" class=BUILDER_FIELD>
                    <Input id="plot-from" kind="date" value=from />
                </Field>
                <Field label="To" id="plot-to" class=BUILDER_FIELD>
                    <Input id="plot-to" kind="date" value=to />
                </Field>
            }
        })
    };

    // Row two: as [chart], only [filters].
    let charts = Memo::new(move |_| {
        // While the dates are refused nothing validates: keep the chart chosen.
        spec.with(|s| {
            Some(s.charts())
                .filter(|c| !c.is_empty())
                .unwrap_or(vec![s.chart])
        })
        .into_iter()
        .map(|c| (id_of(&CHARTS, c).to_owned(), name_of(&CHARTS, c).to_owned()))
        .collect::<Vec<_>>()
    });
    let charts = Signal::from(charts);
    let chart = Signal::derive(move || spec.with(|s| id_of(&CHARTS, s.chart).to_owned()));
    let f0 = first.filters.clone();
    let filtered = f0 != Default::default();
    let filters_open = RwSignal::new(filtered);
    let account_names = StoredValue::new(filter_options.accounts.clone());
    let names_of_accounts = |ids: &[String]| {
        account_names.with_value(|a| {
            ids.iter()
                .map(|id| {
                    a.iter()
                        .find(|(i, _)| i == id)
                        .map_or(id.clone(), |(_, n)| n.clone())
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
    };
    let categories = applied(f0.categories.join(", "), choose, |v| {
        Some(Choice::Categories(list_names(&v)))
    });
    let groups = applied(f0.groups.join(", "), choose, |v| {
        Some(Choice::Groups(list_names(&v)))
    });
    let accounts = applied(names_of_accounts(&f0.accounts), choose, move |v| {
        let ids = list_names(&v)
            .into_iter()
            .map(|n| {
                account_names.with_value(|a| {
                    a.iter()
                        .find(|(_, name)| *name == n)
                        .map_or(n.clone(), |(id, _)| id.clone())
                })
            })
            .collect();
        Some(Choice::Accounts(ids))
    });
    let merchant = applied(f0.merchant.clone(), choose, |v| Some(Choice::Merchant(v)));
    let min = applied(amount_text(f0.min), choose, |v| {
        Some(Choice::Min(amount(&v)))
    });
    let max = applied(amount_text(f0.max), choose, |v| {
        Some(Choice::Max(amount(&v)))
    });
    let category_options = StoredValue::new(filter_options.categories);
    let group_options = StoredValue::new(filter_options.groups);
    let account_options = StoredValue::new(
        filter_options
            .accounts
            .into_iter()
            .map(|(_, n)| n)
            .collect::<Vec<_>>(),
    );
    let filter_fields = move || {
        filters_open.get().then(|| {
            view! {
                <div class=SUBPANEL>
                    <Field label="Categories" id="plot-categories">
                        <MultiSelect id="plot-categories" label="Categories" value=categories options=category_options.get_value() />
                    </Field>
                    <Field label="Groups" id="plot-groups">
                        <MultiSelect id="plot-groups" label="Groups" value=groups options=group_options.get_value() />
                    </Field>
                    <Field label="Accounts" id="plot-accounts">
                        <MultiSelect id="plot-accounts" label="Accounts" value=accounts options=account_options.get_value() />
                    </Field>
                    <div class=FILTER_ROW>
                        <Field label="Merchant contains" id="plot-merchant" class=BUILDER_FIELD>
                            <Input id="plot-merchant" value=merchant />
                        </Field>
                        <Field label="Smallest amount" id="plot-min" class=BUILDER_FIELD>
                            <Input id="plot-min" kind="number" step="0.01" value=min />
                        </Field>
                        <Field label="Largest amount" id="plot-max" class=BUILDER_FIELD>
                            <Input id="plot-max" kind="number" step="0.01" value=max />
                        </Field>
                    </div>
                </div>
            }
        })
    };

    // Row three: what only a time axis offers, then the basis and "Variable only".
    let time = Memo::new(move |_| spec.with(PlotSpec::time_axis));
    let compare = bound(spec, |s| id_of(&COMPARES, s.compare).to_owned());
    let compare_options: Vec<(String, String)> = COMPARES
        .iter()
        .map(|(_, id, name)| (id.to_string(), name.to_string()))
        .collect();
    // All time has nothing before it to compare with.
    let comparable =
        Memo::new(move |_| spec.with(|s| s.split.is_none() && s.period != Period::AllTime));
    let time_options = move || {
        time.get().then(|| {
            let compare_options = compare_options.clone();
            view! {
                <label class=FILTER_ROW>
                    <Switch
                        checked=Signal::derive(move || spec.with(|s| s.cumulative))
                        on_change=Callback::new(move |on| choose(Choice::Cumulative(on)))
                    />
                    <span class=FIELD_LABEL>"Running total"</span>
                </label>
                <label class=FILTER_ROW>
                    <Switch
                        checked=Signal::derive(move || spec.with(|s| s.average_line))
                        on_change=Callback::new(move |on| choose(Choice::AverageLine(on)))
                    />
                    <span class=FIELD_LABEL>"Average line"</span>
                </label>
                {move || comparable.get().then(|| {
                    let compare_options = compare_options.clone();
                    view! {
                        <Field label="Compare with" id="plot-compare" class=BUILDER_FIELD>
                            <Select
                                id="plot-compare"
                                value=compare
                                options=compare_options
                                on_change=Callback::new(move |v: String| {
                                    if let Some(c) = from_id(&COMPARES, &v) {
                                        choose(Choice::Compare(c));
                                    }
                                })
                            />
                        </Field>
                    }
                })}
            }
        })
    };

    let period_options: Vec<(String, String)> = PERIODS
        .iter()
        .map(|(id, l)| (id.to_string(), l.to_string()))
        .collect();
    let heading_id = heading.clone();
    let first_control = NodeRef::<html::Div>::new();
    // Focus starts on the first control, inside the box.
    Effect::new(move |_| {
        if let Some(row) = first_control.get()
            && let Ok(Some(el)) = row.query_selector("select")
            && let Ok(el) = wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el)
        {
            let _ = el.focus();
        }
    });
    view! {
        <Popup title=heading_id class="w-[min(94vw,60rem)]" on_close=on_close>
            <div class=BUILDER>
                <div class=BUILDER_ROW node_ref=first_control>
                    <div class=crate::ds::cn([FIELD, BUILDER_FIELD])>
                        <span class=FIELD_LABEL>"Show"</span>
                        {measures}
                    </div>
                    <Field label="By" id="plot-by" class=BUILDER_FIELD>
                        <Select
                            id="plot-by"
                            value=by
                            options=options(&AXES)
                            on_change=Callback::new(move |v: String| {
                                if let Some(a) = from_id(&AXES, &v) {
                                    choose(Choice::By(a));
                                }
                            })
                        />
                    </Field>
                    <Field label="Split by" id="plot-split" class=BUILDER_FIELD>
                        <Select
                            id="plot-split"
                            value=split
                            options=split_options
                            on_change=Callback::new(move |v: String| choose(Choice::Split(from_id(&SPLITS, &v))))
                        />
                    </Field>
                    <Field label="Over" id="plot-period" class=BUILDER_FIELD>
                        <Select
                            id="plot-period"
                            value=period
                            options=period_options
                            on_change=Callback::new(move |v: String| {
                                if let Some(p) = period_from(&v, &today) {
                                    choose(Choice::Period(p));
                                }
                            })
                        />
                    </Field>
                    {date_fields}
                </div>
                <div class=FILTER_ROW>
                    <ChartChips
                        label="Chart"
                        value=chart
                        items=charts
                        on_change=Callback::new(move |v: String| {
                            if let Some(c) = from_id(&CHARTS, &v) {
                                choose(Choice::Chart(c));
                            }
                        })
                    />
                    <button
                        type="button"
                        class=crate::ds::button_class(Variant::Secondary, Size::Sm)
                        aria-expanded=move || filters_open.get().to_string()
                        on:click=move |_| filters_open.update(|o| *o = !*o)
                    >
                        <Lucide icon=Icon::SlidersHorizontal size=14 />
                        "Only…"
                    </button>
                </div>
                {filter_fields}
                <div class=FILTER_ROW>
                    {time_options}
                    <SegmentedControl
                        label="Months"
                        small=true
                        value=Signal::derive(move || spec.with(|s| match s.period_basis {
                            PeriodBasis::Calendar => "calendar".to_owned(),
                            PeriodBasis::Budget => "budget".to_owned(),
                        }))
                        on_change=Callback::new(move |v: String| choose(Choice::Basis(
                            if v == "budget" { PeriodBasis::Budget } else { PeriodBasis::Calendar },
                        )))
                        items=vec![
                            ("calendar".to_owned(), "Calendar months".to_owned()),
                            ("budget".to_owned(), "Budget months".to_owned()),
                        ]
                    />
                    <label class=FILTER_ROW>
                        <Switch
                            checked=Signal::derive(move || spec.with(|s| s.filters.variable_only))
                            on_change=Callback::new(move |on| choose(Choice::VariableOnly(on)))
                        />
                        <span class=FIELD_LABEL>"Variable only"</span>
                    </label>
                </div>
                <div class=SUBPANEL>
                    <PlotView spec=shown.into() title=title />
                </div>
                <div class=SUBPANEL>
                    <Field label="Name" id="plot-title">
                        <span class="contents" on:input=move |_| follows.set_value(false)>
                            <Input id="plot-title" value=title />
                        </span>
                    </Field>
                    {move || refused.get().map(|why| view! { <p role="alert" class=ERROR_LINE>{why}</p> })}
                    <div class=ACTIONS>
                        <Button variant=Variant::Primary size=Size::Sm on_click=Callback::new(move |_| save(Saving::Over))>
                            "Save"
                        </Button>
                        {existing.then(|| view! {
                            <Button variant=Variant::Secondary size=Size::Sm on_click=Callback::new(move |_| save(Saving::New))>
                                "Save as new"
                            </Button>
                        })}
                        <Button variant=Variant::Ghost size=Size::Sm on_click=Callback::new(move |_| on_close.run(()))>
                            "Cancel"
                        </Button>
                        {existing.then(|| view! {
                            <Confirm
                                label="Delete"
                                aria="Delete this plot"
                                question="Delete this plot? It leaves every tab."
                                on_yes=on_delete
                            />
                        })}
                    </div>
                </div>
            </div>
        </Popup>
    }
}

#[cfg(test)]
mod tests {
    use finance_types::plot::Filters;

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
        ]);
        c
    }

    #[test]
    fn every_result_validates() {
        for spec in samples() {
            for choice in every_choice() {
                let out = with_choice(spec.clone(), choice.clone());
                assert_eq!(out.validate(), Ok(()), "{choice:?} on {spec:?}");
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
}
