//! The Transactions section: search every transaction, shared spending &
//! money back, and the category review inbox (personal-site
//! `integrated-finance.tsx`, owner Transactions tab).

mod consolidation;
mod details;
mod personal;
mod research;
mod review;
mod search;

use std::collections::BTreeSet;
use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;

use crate::ds::patterns::{SELECT_CHEVRON, SELECT_WRAP};
use crate::ds::{CONTROL, Icon, Lucide, cn};
use crate::layout::{Initial, PageView, PanelDef};
use crate::state;

/// Transactions' panels, in their first-run order.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "transactions.search",
        title: "All transactions",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || search::TransactionSearch().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "transactions.personal",
        title: "Shared spending and money back",
        page: PAGE,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || personal::PersonalSpending().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "transactions.inbox",
        title: "Transaction inbox",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || review::OwnerReview().into_any(),
        summary: None,
        shown: None,
    },
];

const PAGE: &str = "transactions";

#[component]
pub fn Transactions() -> impl IntoView {
    view! { <PageView tab="transactions" tab_name="Transactions" page=PAGE /> }
}

#[wasm_bindgen(inline_js = "
export function storage_get(key) { try { return window.localStorage.getItem(key); } catch { return null; } }
export function storage_set(key, value) { try { window.localStorage.setItem(key, value); } catch { } }
export function region_name(code) { try { return new Intl.DisplayNames(['en'], { type: 'region' }).of(code) || code; } catch { return code; } }
export function focus_id(id) { const e = document.getElementById(id); if (e) e.focus(); }
")]
extern "C" {
    /// `Intl.DisplayNames` English region name of an ISO code.
    fn region_name(code: &str) -> String;
    fn focus_id(id: &str);
    /// `localStorage.getItem`, `None` when storage is unavailable.
    fn storage_get(key: &str) -> Option<String>;
    fn storage_set(key: &str, value: &str);
}

/// A sorted, de-duplicated list of the category names the dashboard knows,
/// plus `extra`.
fn report_categories(extra: &'static [&'static str]) -> Memo<Vec<String>> {
    let app = state();
    Memo::new(move |_| {
        app.report.data.with(|data| {
            let mut names: BTreeSet<String> = extra.iter().map(|c| (*c).to_owned()).collect();
            if let Some(Ok(report)) = data {
                names.extend(
                    report
                        .monthly_spending
                        .mappings
                        .iter()
                        .map(|m| m.category.clone()),
                );
                names.extend(
                    report
                        .category_catalog
                        .entries
                        .iter()
                        .map(|e| e.name.clone()),
                );
            }
            names.into_iter().collect()
        })
    })
}

/// Errors from the backend, without the core's internal prefix.
fn clean_error(error: String) -> String {
    error
        .strip_prefix("stored value is not valid: ")
        .map(str::to_owned)
        .unwrap_or(error)
}

/// The site's `Input` as a plain control bound to `value`.
#[component]
fn TextInput(
    value: RwSignal<String>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] kind: Option<String>,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional)] maxlength: Option<usize>,
    #[prop(optional, into)] inputmode: Option<String>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional)] upper: bool,
    #[prop(optional, into)] on_input: Option<Callback<String>>,
) -> impl IntoView {
    view! {
        <input
            id=id
            type=kind.unwrap_or_else(|| "text".into())
            class=cn([CONTROL, class.as_str()])
            placeholder=placeholder
            aria-label=label
            maxlength=maxlength
            inputmode=inputmode
            disabled=move || disabled.get()
            prop:value=move || value.get()
            on:input=move |e| {
                let mut text = event_target_value(&e);
                if upper {
                    text = text.to_uppercase();
                }
                value.set(text.clone());
                if let Some(f) = on_input {
                    f.run(text);
                }
            }
        />
    }
}

/// The site's `Select` with a change handler; `options` are (value, label,
/// disabled) triples.
#[component]
fn Choice(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] options: Signal<Vec<(String, String, bool)>>,
    on_change: Callback<String>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    view! {
        <div class=SELECT_WRAP>
            <select
                id=id
                aria-label=label
                class=cn([CONTROL, "appearance-none pr-9", class.as_str()])
                disabled=move || disabled.get()
                on:change=move |e| on_change.run(event_target_value(&e))
                prop:value=move || value.get()
            >
                {move || {
                    let current = value.get();
                    options
                        .get()
                        .into_iter()
                        .map(|(v, l, off)| {
                            let selected = v == current;
                            view! { <option value=v selected=selected disabled=off>{l}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <span class=SELECT_CHEVRON>
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

/// `(value, label)` pairs as enabled options.
fn options<I, A, B>(items: I) -> Vec<(String, String, bool)>
where
    I: IntoIterator<Item = (A, B)>,
    A: Into<String>,
    B: Into<String>,
{
    items
        .into_iter()
        .map(|(v, l)| (v.into(), l.into(), false))
        .collect()
}

/// Run `f` once `source` has not changed for `delay`.
fn debounce<T: Clone + PartialEq + Send + Sync + 'static>(
    source: RwSignal<T>,
    delay: Duration,
    f: impl Fn(T) + Clone + 'static,
) {
    let timer = StoredValue::new(None::<TimeoutHandle>);
    Effect::new(move |previous: Option<()>| {
        let value = source.get();
        if previous.is_none() {
            return;
        }
        if let Some(handle) = timer.get_value() {
            handle.clear();
        }
        let f = f.clone();
        timer.set_value(set_timeout_with_handle(move || f(value), delay).ok());
    });
}
