//! Parts of the plot builder: the chart chips (a radio group) and one
//! measure's row.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::patterns::{ACTIONS, BUILDER_FIELD, CHIPS};
use super::{Icon, Lucide, SegmentedControl, Select, Size, Variant, button_class};

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
                    if let Some(g) = group.get_untracked()
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
        <div node_ref=group role="radiogroup" aria-label=label class=CHIPS on:keydown=on_key>
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

/// One measure of a plot: what it is, and when there are several on bars,
/// bars or a line; with a remove button when it is not the only one.
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
        <span class=ACTIONS>
            <span class=BUILDER_FIELD>
                <Select
                    id=format!("plot-measure-{index}")
                    label=format!("Measure {n}")
                    value=value
                    options=options
                    on_change=on_pick
                />
            </span>
            {draw.map(|d| view! {
                <SegmentedControl
                    label=format!("Draw measure {n} as")
                    small=true
                    value=d
                    on_change=on_draw
                    items=vec![("bars".to_owned(), "Bars".to_owned()), ("line".to_owned(), "Line".to_owned())]
                />
            })}
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
