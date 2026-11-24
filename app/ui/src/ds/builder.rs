//! Parts of the plot builder: the chart chips (a radio group), one measure's
//! row, a labelled select, a switch with its label, and chips that pick
//! several values of a list.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::patterns::{
    BUILDER_FIELD, CHIP, CHIP_OFF, CHIP_ON, CHIPS, CONTROL_GROUP, CONTROL_ROW, GROUP_SELECT, MUTED,
    SWITCH_FIELD,
};
use super::{
    CONTROL, Field, Icon, LONG_LIST, Lucide, SegmentedControl, Select, Size, Switch, Variant,
    button_class, cn,
};

/// Pick one of `items` (value, label) as chips; a radio group: one tab stop,
/// the arrow keys move the choice.
#[component]
pub fn ChartChips(
    #[prop(into)] label: String,
    value: Signal<String>,
    items: Signal<Vec<(String, String)>>,
    on_change: Callback<String>,
) -> impl IntoView {
    let group = NodeRef::<html::Div>::new();
    let pick = move |v: String, focus: bool| {
        on_change.run(v);
        if focus {
            // After the redraw, on the chip now checked.
            set_timeout(
                move || {
                    if let Some(Some(g)) = group.try_get_untracked()
                        && let Ok(Some(el)) = g.query_selector("[aria-checked=true]")
                        && let Ok(el) = el.dyn_into::<web_sys::HtmlElement>()
                    {
                        let _ = el.focus();
                    }
                },
                std::time::Duration::ZERO,
            );
        }
    };
    let on_key = move |e: leptos::ev::KeyboardEvent| {
        let step: isize = match e.key().as_str() {
            "ArrowRight" | "ArrowDown" => 1,
            "ArrowLeft" | "ArrowUp" => -1,
            _ => return,
        };
        e.prevent_default();
        let all = items.get_untracked();
        if all.is_empty() {
            return;
        }
        let at = all
            .iter()
            .position(|(v, _)| *v == value.get_untracked())
            .unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(all.len() as isize) as usize;
        pick(all[next].0.clone(), true);
    };
    view! {
        <div node_ref=group role="radiogroup" aria-label=label class=CONTROL_ROW on:keydown=on_key>
            {move || {
                items
                    .get()
                    .into_iter()
                    .map(|(v, l)| {
                        let on = {
                            let v = v.clone();
                            move || value.get() == v
                        };
                        let (on2, on3) = (on.clone(), on.clone());
                        view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || on().to_string()
                                tabindex=move || if on2() { "0" } else { "-1" }
                                class=move || button_class(if on3() { Variant::Primary } else { Variant::Secondary }, Size::Sm)
                                on:click=move |_| pick(v.clone(), false)
                            >
                                {l}
                            </button>
                        }
                    })
                    .collect_view()
            }}
        </div>
    }
}

/// One measure of a plot: what it is, bars or a line (when `draw` is given),
/// and a remove button when `removable`; on one middle line.
#[component]
pub fn MeasureRow(
    index: usize,
    value: RwSignal<String>,
    options: Vec<(String, String)>,
    /// Bars or a line ("bars" / "line"); None hides the choice.
    draw: Option<Signal<String>>,
    removable: bool,
    on_pick: Callback<String>,
    on_draw: Callback<String>,
    on_remove: Callback<()>,
) -> impl IntoView {
    let n = index + 1;
    view! {
        <span class=CONTROL_GROUP>
            <span class=GROUP_SELECT>
                <Select
                    id=format!("plot-measure-{index}")
                    label=format!("Measure {n}")
                    value=value
                    options=options
                    on_change=on_pick
                />
            </span>
            {draw.map(|d| view! { <DrawChoice index=index value=d on_change=on_draw /> })}
            {removable.then(|| view! {
                <button
                    type="button"
                    class=button_class(Variant::Ghost, Size::IconSm)
                    aria-label=format!("Remove measure {n}")
                    on:click=move |_| on_remove.run(())
                >
                    <Lucide icon=Icon::X size=14 />
                </button>
            })}
        </span>
    }
}

/// Bars or a line for measure `index` (0-based); with `named`, after a
/// muted "<named> as".
#[component]
pub fn DrawChoice(
    index: usize,
    value: Signal<String>,
    on_change: Callback<String>,
    #[prop(optional)] named: Option<Signal<String>>,
) -> impl IntoView {
    view! {
        <span class=CONTROL_GROUP>
            {named.map(|n| view! { <span class=MUTED>{move || format!("{} as", n.get())}</span> })}
            <SegmentedControl
                label=format!("Draw measure {} as", index + 1)
                small=true
                value=value
                on_change=on_change
                items=vec![("bars".to_owned(), "Bars".to_owned()), ("line".to_owned(), "Line".to_owned())]
            />
        </span>
    }
}

/// A labelled select in a row of fields; a pick runs `on_change`.
#[component]
pub fn SelectField(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<(String, String)>>,
    on_change: Callback<String>,
) -> impl IntoView {
    view! {
        <Field label=label id=id.clone() class=BUILDER_FIELD>
            <Select id=id value=value options=options on_change=on_change />
        </Field>
    }
}

/// A switch with its label after it, on one middle line.
#[component]
pub fn SwitchField(
    #[prop(into)] label: String,
    checked: Signal<bool>,
    on_change: Callback<bool>,
    #[prop(into)] id: String,
) -> impl IntoView {
    view! {
        <label class=SWITCH_FIELD>
            <Switch id=id checked=checked on_change=on_change />
            <span>{label}</span>
        </label>
    }
}

/// Pick several of `options` (value, label) as chips. `chosen` are values:
/// labels may repeat or hold commas. A chosen value that is no longer an
/// option keeps its chip; a long list shows the chosen and a search box.
#[component]
pub fn ChipPick(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    options: Signal<Vec<(String, String)>>,
    chosen: Signal<Vec<String>>,
    on_change: Callback<Vec<String>>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let long = move || options.with(|o| o.len() > LONG_LIST);
    let chips = move || {
        let on_now = chosen.get();
        let mut all = options.get();
        for v in &on_now {
            if !all.iter().any(|(o, _)| o == v) {
                all.push((v.clone(), v.clone()));
            }
        }
        let needle = query.get().trim().to_lowercase();
        all.into_iter()
            .filter(|(v, l)| {
                !long()
                    || on_now.contains(v)
                    || (!needle.is_empty() && l.to_lowercase().contains(&needle))
            })
            .map(|(v, l)| {
                let on = on_now.contains(&v);
                view! {
                    <button
                        type="button"
                        aria-pressed=on.to_string()
                        class=cn([CHIP, if on { CHIP_ON } else { CHIP_OFF }])
                        on:click=move |_| {
                            let mut next = chosen.get_untracked();
                            if let Some(at) = next.iter().position(|c| *c == v) {
                                next.remove(at);
                            } else {
                                next.push(v.clone());
                            }
                            on_change.run(next);
                        }
                    >
                        {on.then(|| view! { <Lucide icon=Icon::Check size=12 stroke=3.0 /> })}
                        {l}
                    </button>
                }
            })
            .collect_view()
    };
    let search_label = format!("Search {label}");
    view! {
        <Field label=label.clone() id=id.clone()>
            <div id=id role="group" aria-label=label class=CHIPS>
                {move || long().then(|| view! {
                    <input
                        type="search"
                        class=cn([CONTROL, "h-8 max-w-xs basis-full"])
                        placeholder=format!("Search {} to add", options.with(Vec::len))
                        aria-label=search_label.clone()
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev))
                    />
                })}
                {chips}
                {move || {
                    (options.with(Vec::is_empty) && chosen.with(Vec::is_empty))
                        .then(|| view! { <p class=MUTED>"Nothing to choose from yet."</p> })
                }}
            </div>
        </Field>
    }
}
