//! "Category library": merge a less-used category spelling into another
//! after a preview (personal-site `category-consolidation.tsx`).

use finance_types::report::catalog::CategoryCatalog;
use finance_types::report::review::{Consolidation, ConsolidationApplied, ConsolidationPreview};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use super::{Choice, clean_error, options};
use crate::api::invoke;
use crate::ds::patterns::{Disclosure, DisclosureKind, GROW_48, HINT, INLINE_FORM, MUTED};
use crate::ds::{Field, Help, Size, Variant, button_class, cn};
use crate::state;

#[derive(Serialize)]
struct Args {
    request: Consolidation,
}

#[component]
pub fn CategoryConsolidation(catalog: CategoryCatalog) -> impl IntoView {
    let app = state();
    let source = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<ConsolidationPreview>);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let submit = move |apply: bool| {
        let token = preview.get_untracked().map(|p| p.token);
        if busy.get_untracked() || (apply && token.is_none()) {
            return;
        }
        busy.set(true);
        message.set(String::new());
        let request = Consolidation {
            source: source.get_untracked(),
            target: target.get_untracked(),
            token: if apply { token } else { None },
        };
        let target_name = request.target.clone();
        spawn_local(async move {
            if apply {
                match invoke::<ConsolidationApplied>("apply_consolidation", &Args { request }).await
                {
                    Ok(body) => {
                        preview.set(None);
                        message.set(if body.complete {
                            format!(
                                "Consolidated {} transactions into “{target_name}”.{}",
                                body.applied,
                                if body.refreshed {
                                    ""
                                } else {
                                    " Balances and category lists will update after source refresh."
                                }
                            )
                        } else {
                            format!(
                                "{} of {} updated. {}",
                                body.applied,
                                body.total,
                                body.error.unwrap_or_default()
                            )
                        });
                        app.refresh();
                    }
                    Err(e) => {
                        preview.set(None);
                        message.set(clean_error(e));
                    }
                }
            } else {
                match invoke::<ConsolidationPreview>("preview_consolidation", &Args { request })
                    .await
                {
                    Ok(body) => preview.set(Some(body)),
                    Err(e) => {
                        preview.set(None);
                        message.set(clean_error(e));
                    }
                }
            }
            busy.set(false);
        });
    };
    let summary = format!(
        "Category library · {} names · {} similar pairs",
        catalog.entries.len(),
        catalog.similar.len()
    );
    let choices = {
        let mut list = options([("", "Choose category")]);
        list.extend(
            catalog
                .entries
                .iter()
                .map(|e| (e.name.clone(), format!("{} ({})", e.name, e.count), false)),
        );
        list
    };
    view! {
        <Disclosure kind=DisclosureKind::Panel summary=summary>
            <div class="mt-3 grid gap-4">
                <Help text="Checked whenever reports refresh. Review meaning before merging similar names. Extra tags stay attached to their transactions." />
                {catalog
                    .similar
                    .into_iter()
                    .map(|pair| {
                        let (s, t) = (pair.source.clone(), pair.target.clone());
                        view! {
                            <div class="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-3">
                                <span class="min-w-0 break-words text-sm">
                                    {format!("{} → {} · {} transactions", pair.source, pair.target, pair.count)}
                                    <br />
                                    <small class=HINT>{pair.reason}</small>
                                </span>
                                <button
                                    type="button"
                                    class=button_class(Variant::Secondary, Size::Sm)
                                    disabled=move || busy.get()
                                    on:click=move |_| {
                                        source.set(s.clone());
                                        target.set(t.clone());
                                        preview.set(None);
                                    }
                                >
                                    "Review pair"
                                </button>
                            </div>
                        }
                    })
                    .collect_view()}
                <div class=INLINE_FORM>
                    {[("From", source, "consolidate-from"), ("Keep", target, "consolidate-keep")]
                        .into_iter()
                        .map(|(label, value, id)| {
                            let choices = choices.clone();
                            view! {
                                <Field label=label id=id class=GROW_48>
                                    <Choice
                                        id=id
                                        value=value
                                        options=choices
                                        disabled=busy
                                        on_change=Callback::new(move |v| {
                                            value.set(v);
                                            preview.set(None);
                                        })
                                    />
                                </Field>
                            }
                        })
                        .collect_view()}
                    <button
                        type="button"
                        class=button_class(Variant::Secondary, Size::Md)
                        disabled=move || {
                            let (s, t) = (source.get(), target.get());
                            busy.get() || s.is_empty() || t.is_empty() || s == t
                        }
                        on:click=move |_| submit(false)
                    >
                        "Preview consolidation"
                    </button>
                </div>
                {move || preview.get().map(|p| view! {
                    <div class="grid gap-3 rounded-lg border border-border bg-card-strong p-3 text-sm">
                        <p class="break-words">
                            {format!(
                                "Change {} transactions from “{}” to “{}”? Amounts and saved details stay unchanged.",
                                p.count, p.source, p.target
                            )}
                        </p>
                        <button
                            type="button"
                            class=cn([button_class(Variant::Primary, Size::Md).as_str(), "justify-self-start"])
                            disabled=move || busy.get()
                            on:click=move |_| submit(true)
                        >
                            {move || if busy.get() { "Consolidating…" } else { "Confirm consolidation" }}
                        </button>
                    </div>
                })}
                {move || {
                    let m = message.get();
                    (!m.is_empty()).then(|| view! { <p role="status" class=MUTED>{m}</p> })
                }}
            </div>
        </Disclosure>
    }
}
