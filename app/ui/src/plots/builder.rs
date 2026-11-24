//! The plot builder: a popup where the owner picks what a plot shows, sees it
//! drawn as he goes, and saves it as a panel. Every control goes through
//! [`with_choice`], which keeps the spec one that validates.

use std::time::Duration;

use finance_types::plot::{Chart, Draw, Period, PeriodBasis, PlotSpec, Value};
use leptos::html;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::PlotView;
use super::choice::{
    AXES, CHARTS, COMPARES, Choice, MAX_MEASURES, MEASURES, PERIODS, SPLITS, VALUES, capitalised,
    default_title, filter_count, from_id, id_of, name_of, options, period_from, period_id,
    turned_off, with_choice,
};
use crate::ds::builder::{ChartChips, ChipPick, DrawChoice, MeasureRow, SelectField, SwitchField};
use crate::ds::patterns::{
    BUILDER, BUILDER_FIELD, CONTROL_ROW, ERROR_LINE, FIELD_ROW, MUTED, SUBPANEL,
};
use crate::ds::{
    Button, Confirm, Field, Icon, Input, Lucide, Popup, SegmentedControl, Size, Variant,
};

/// Over the plot being edited, or as a new plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saving {
    Over,
    New,
}

/// What the filters pick from, as (value, label): categories and groups by
/// name, accounts by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterOptions {
    pub categories: Vec<(String, String)>,
    pub groups: Vec<(String, String)>,
    pub accounts: Vec<(String, String)>,
}

/// The wait after the last change before the preview reloads.
const PREVIEW_DELAY: Duration = Duration::from_millis(250);

/// A select kept equal to `value(spec)`.
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
    choice: impl Fn(String) -> Choice + Send + Sync + 'static,
) -> RwSignal<String> {
    let v = RwSignal::new(initial.clone());
    Effect::new(move |was: Option<String>| {
        let now = v.get();
        if was.unwrap_or_else(|| initial.clone()) != now {
            choose(choice(now.clone()));
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

/// A `SegmentedControl` over a table of (value, id, label).
fn segments<T: Copy + PartialEq + Send + Sync + 'static, const N: usize>(
    label: &'static str,
    table: &'static [(T, &'static str, &'static str); N],
    value: Signal<T>,
    on_change: impl Fn(T) + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <SegmentedControl
            label=label
            small=true
            value=Signal::derive(move || id_of(table, value.get()).to_owned())
            on_change=Callback::new(move |v: String| {
                if let Some(t) = from_id(table, &v) {
                    on_change(t);
                }
            })
            items=table.iter().map(|(_, id, l)| (id.to_string(), l.to_string())).collect()
        />
    }
}

const BASES: [(PeriodBasis, &str, &str); 2] = [
    (PeriodBasis::Calendar, "calendar", "Calendar months"),
    (PeriodBasis::Budget, "budget", "Budget months"),
];

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
    /// Follows the report, which may still be loading when the builder opens.
    filter_options: Signal<FilterOptions>,
    on_save: Callback<(Saving, String, PlotSpec), Result<(), String>>,
    on_delete: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let first = spec.clone();
    let spec = RwSignal::new(spec);
    // What the last choice's repair turned off, until the next choice.
    let off = RwSignal::new(Vec::<&'static str>::new());
    let choose = move |c: Choice| {
        let now = spec.get_untracked();
        let next = with_choice(now.clone(), c.clone());
        off.set(turned_off(&now, &c, &next));
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

    // Row one: Show … by … split by … over …, one line of selects.
    let today = crate::pages::wishes::today();
    let first_measure = bound(spec, |s| {
        s.measures
            .first()
            .map_or("", |m| id_of(&MEASURES, m.measure))
            .to_owned()
    });
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
    let period = bound(spec, |s| period_id(&s.period).to_owned());
    let period_options: Vec<(String, String)> = PERIODS
        .iter()
        .map(|(id, l)| (id.to_string(), l.to_string()))
        .collect();
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
                Choice::Period(Period::Between {
                    from: from.unwrap_or(f),
                    to: to.unwrap_or(t),
                })
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

    // Under it: the first measure's drawing, the other measures, "Add a measure".
    let shape = Memo::new(move |_| {
        spec.with(|s| (s.measures.len(), s.split.is_none(), s.chart == Chart::Bars))
    });
    let draw_of = move |i: usize| {
        Signal::derive(move || {
            spec.with(|s| match s.measures.get(i).map(|m| m.draw) {
                Some(Draw::Line) => "line".to_owned(),
                _ => "bars".to_owned(),
            })
        })
    };
    let on_draw = move |i: usize| {
        Callback::new(move |v: String| {
            choose(Choice::Draw(
                i,
                if v == "line" { Draw::Line } else { Draw::Bars },
            ));
        })
    };
    let first_name = Signal::derive(move || {
        spec.with(|s| {
            s.measures
                .first()
                .map_or("", |m| name_of(&MEASURES, m.measure))
                .to_owned()
        })
    });
    let measures = move || {
        let (n, unsplit, bars) = shape.get();
        let lines = bars && n > 1;
        let rest = (1..n)
            .map(|i| {
                let value = bound(spec, move |s| {
                    s.measures
                        .get(i)
                        .map_or("", |m| id_of(&MEASURES, m.measure))
                        .to_owned()
                });
                let (choices, draw) = (options(&MEASURES), lines.then(|| draw_of(i)));
                view! {
                    <MeasureRow
                        index=i
                        value=value
                        options=choices
                        draw=draw
                        removable=true
                        on_pick=Callback::new(move |v: String| {
                            if let Some(m) = from_id(&MEASURES, &v) {
                                choose(Choice::Measure(i, m));
                            }
                        })
                        on_draw=on_draw(i)
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
        (n > 1 || add.is_some()).then(|| {
            view! {
                <div class=CONTROL_ROW>
                    {lines.then(|| view! { <DrawChoice index=0 value=draw_of(0) on_change=on_draw(0) named=first_name /> })}
                    {rest}
                    {add}
                </div>
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
    let chart = Signal::derive(move || spec.with(|s| id_of(&CHARTS, s.chart).to_owned()));
    let filters_open = RwSignal::new(filter_count(&first.filters) > 0);
    let only_text = move || {
        let n = spec.with(|s| filter_count(&s.filters));
        if filters_open.get() || n == 0 {
            "Only…".to_owned()
        } else {
            format!("Only… ({n})")
        }
    };
    let f0 = first.filters.clone();
    let merchant = applied(f0.merchant.clone(), choose, Choice::Merchant);
    let min = applied(amount_text(f0.min), choose, |v| Choice::Min(amount(&v)));
    let max = applied(amount_text(f0.max), choose, |v| Choice::Max(amount(&v)));
    let pick = move |options: fn(&FilterOptions) -> Vec<(String, String)>,
                     chosen: fn(&PlotSpec) -> Vec<String>,
                     to: fn(Vec<String>) -> Choice| {
        (
            Signal::derive(move || filter_options.with(options)),
            Signal::derive(move || spec.with(chosen)),
            Callback::new(move |v: Vec<String>| choose(to(v))),
        )
    };
    let filter_fields = move || {
        filters_open.get().then(|| {
            let (c_opts, c_on, c_set) =
                pick(|o| o.categories.clone(), |s| s.filters.categories.clone(), Choice::Categories);
            let (g_opts, g_on, g_set) =
                pick(|o| o.groups.clone(), |s| s.filters.groups.clone(), Choice::Groups);
            let (a_opts, a_on, a_set) =
                pick(|o| o.accounts.clone(), |s| s.filters.accounts.clone(), Choice::Accounts);
            view! {
                <div class=SUBPANEL>
                    <ChipPick label="Categories" id="plot-categories" options=c_opts chosen=c_on on_change=c_set />
                    <ChipPick label="Groups" id="plot-groups" options=g_opts chosen=g_on on_change=g_set />
                    <ChipPick label="Accounts" id="plot-accounts" options=a_opts chosen=a_on on_change=a_set />
                    <div class=FIELD_ROW>
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

    // Row three: amounts or change where change fits; what only a time axis
    // offers; then the basis and "Variable only"; all on one middle line.
    let changeable = Memo::new(move |_| {
        spec.with(|s| with_choice(s.clone(), Choice::Value(Value::Change)).value == Value::Change)
    });
    let time = Memo::new(move |_| spec.with(PlotSpec::time_axis));
    let compare = bound(spec, |s| id_of(&COMPARES, s.compare).to_owned());
    let compare_options: Vec<(String, String)> = COMPARES
        .iter()
        .map(|(_, id, name)| (id.to_string(), name.to_string()))
        .collect();
    // All time has nothing before it to compare with.
    let comparable = Memo::new(move |_| {
        spec.with(|s| s.split.is_none() && s.period != Period::AllTime && s.value == Value::Amount)
    });
    let value_choice = move || {
        changeable.get().then(|| {
            segments(
                "Values",
                &VALUES,
                Signal::derive(move || spec.with(|s| s.value)),
                move |v| choose(Choice::Value(v)),
            )
        })
    };
    let time_options = move || {
        time.get().then(|| {
            let compare_options = compare_options.clone();
            view! {
                <SwitchField
                    label="Running total"
                    id="plot-running"
                    checked=Signal::derive(move || spec.with(|s| s.cumulative))
                    on_change=Callback::new(move |on| choose(Choice::Cumulative(on)))
                />
                <SwitchField
                    label="Average line"
                    id="plot-average"
                    checked=Signal::derive(move || spec.with(|s| s.average_line))
                    on_change=Callback::new(move |on| choose(Choice::AverageLine(on)))
                />
                {move || comparable.get().then(|| {
                    view! {
                        <span class=CONTROL_ROW>
                            <label for="plot-compare" class=MUTED>"Compare with"</label>
                            <span class="w-48">
                                <crate::ds::Select
                                    id="plot-compare"
                                    value=compare
                                    options=compare_options.clone()
                                    on_change=Callback::new(move |v: String| {
                                        if let Some(c) = from_id(&COMPARES, &v) {
                                            choose(Choice::Compare(c));
                                        }
                                    })
                                />
                            </span>
                        </span>
                    }
                })}
            }
        })
    };

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
                <div class=FIELD_ROW node_ref=first_control>
                    <SelectField
                        label="Show"
                        id="plot-measure-0"
                        value=first_measure
                        options=options(&MEASURES)
                        on_change=Callback::new(move |v: String| {
                            if let Some(m) = from_id(&MEASURES, &v) {
                                choose(Choice::Measure(0, m));
                            }
                        })
                    />
                    <SelectField
                        label="By"
                        id="plot-by"
                        value=by
                        options=options(&AXES)
                        on_change=Callback::new(move |v: String| {
                            if let Some(a) = from_id(&AXES, &v) {
                                choose(Choice::By(a));
                            }
                        })
                    />
                    <SelectField
                        label="Split by"
                        id="plot-split"
                        value=split
                        options=split_options
                        on_change=Callback::new(move |v: String| choose(Choice::Split(from_id(&SPLITS, &v))))
                    />
                    <SelectField
                        label="Over"
                        id="plot-period"
                        value=period
                        options=period_options
                        on_change=Callback::new(move |v: String| {
                            if let Some(p) = period_from(&v, &today) {
                                choose(Choice::Period(p));
                            }
                        })
                    />
                    {date_fields}
                </div>
                {measures}
                <div class=CONTROL_ROW>
                    <ChartChips
                        label="Chart"
                        value=chart
                        items=charts.into()
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
                        {only_text}
                    </button>
                </div>
                {filter_fields}
                <div class=CONTROL_ROW>
                    {value_choice}
                    {time_options}
                    {segments(
                        "Months",
                        &BASES,
                        Signal::derive(move || spec.with(|s| s.period_basis)),
                        move |b| choose(Choice::Basis(b)),
                    )}
                    <SwitchField
                        label="Variable only"
                    id="plot-variable"
                        checked=Signal::derive(move || spec.with(|s| s.filters.variable_only))
                        on_change=Callback::new(move |on| choose(Choice::VariableOnly(on)))
                    />
                </div>
                {move || {
                    let off = off.get();
                    (!off.is_empty()).then(|| view! {
                        <p role="status" class=MUTED>{format!("Turned off: {}", off.join(", "))}</p>
                    })
                }}
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
                    <div class=CONTROL_ROW>
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
