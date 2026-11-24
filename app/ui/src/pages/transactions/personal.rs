//! "Shared spending & money back": the owner marks payments for others and
//! money back from them, with a 30-second undo (personal-site
//! `personal-spending.tsx`).

use std::time::Duration;

use finance_types::report::review::{
    PersonalChoice, PersonalSaved, PersonalTransaction, PersonalTransactions,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use super::{Choice, TextInput, clean_error, options, report_categories};
use crate::api::invoke;
use crate::ds::patterns::{
    Disclosure, ERROR_LINE, FILTER_ROW, FORM_FIELD_NARROW, FORM_FIELD_WIDE, MUTED, SECTION_HEADING,
    SPLIT_ROW, SUBLINE, TRAILING, WRAP_TEXT,
};
use crate::ds::{
    CardTone, Eyebrow, FactsRow, Field, Heading, Help, Money, Size, Variant, button_class,
    card_class, cn,
};
use crate::{format, state};

const UNDO_WINDOW: Duration = Duration::from_secs(30);
const PAYMENT_LIST: &str = "max-h-[34rem] overflow-y-auto";
const EVERYDAY: [&str; 2] = ["Groceries", "Eating out"];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pick {
    role: String,
    category: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Saved {
    row: PersonalTransaction,
    previous: Pick,
    next: Pick,
    revision: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Toast {
    id: u64,
    saved: Saved,
    message: String,
}

#[derive(Serialize)]
struct QueryArgs {
    query: String,
}

#[derive(Serialize)]
struct ChoiceArgs {
    choice: PersonalChoice,
}

fn pick_label(pick: &Pick) -> String {
    match pick.role.as_str() {
        "" => "Choice cleared".into(),
        "ignore" => "Marked as not shared spending".into(),
        role => format!(
            "Marked as {} · {}",
            if role == "return" {
                "money back"
            } else {
                "payment"
            },
            pick.category
        ),
    }
}

/// `2026-09-01` → `1 September 2026`.
fn since_label(since: &str) -> String {
    match (
        since.get(..7),
        since.get(8..10).and_then(|d| d.parse::<u32>().ok()),
    ) {
        (Some(month), Some(day)) => format!("{day} {}", format::month_long(month)),
        _ => since.to_owned(),
    }
}

/// Save a pick; an empty role clears it. Returns the stored revision.
async fn save_pick(row: &PersonalTransaction, revision: u32, pick: &Pick) -> Result<u32, String> {
    let choice = PersonalChoice {
        transaction_id: row.transaction_id.clone(),
        revision,
        role: if pick.role.is_empty() {
            "clear".into()
        } else {
            pick.role.clone()
        },
        category: if pick.role.is_empty() || pick.role == "ignore" {
            String::new()
        } else {
            pick.category.clone()
        },
        expected_amount: row.amount.amount.to_string(),
        expected_currency: row.amount.currency.code.clone(),
    };
    invoke::<PersonalSaved>("save_personal_choice", &ChoiceArgs { choice })
        .await
        .map(|s| s.revision)
        .map_err(|e| {
            let e = clean_error(e);
            if e.is_empty() {
                "Could not save your choice.".into()
            } else {
                e
            }
        })
}

#[component]
pub fn PersonalSpending() -> impl IntoView {
    let app = state();
    let query = RwSignal::new(String::new());
    let search = RwSignal::new(String::new());
    let rows = RwSignal::new(Vec::<PersonalTransaction>::new());
    let total = RwSignal::new(0_usize);
    let since = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let loading = RwSignal::new(true);
    let show_reviewed = RwSignal::new(false);
    let toasts = RwSignal::new(Vec::<Toast>::new());
    let sequence = StoredValue::new(0_u64);
    let generation = StoredValue::new(0_u64);
    let categories = report_categories(&EVERYDAY);

    let load = move |quiet: bool| {
        if !quiet {
            loading.set(true);
        }
        error.set(String::new());
        generation.update_value(|g| *g += 1);
        let mine = generation.get_value();
        let query = search.get_untracked();
        spawn_local(async move {
            let found =
                invoke::<PersonalTransactions>("personal_transactions", &QueryArgs { query }).await;
            if generation.get_value() != mine {
                return;
            }
            match found {
                Ok(data) => {
                    rows.set(data.items);
                    total.set(data.total);
                    since.set(data.since);
                }
                Err(e) => {
                    let e = clean_error(e);
                    error.set(if e.is_empty() {
                        "Could not load personal payments.".into()
                    } else {
                        e
                    });
                }
            }
            loading.set(false);
        });
    };
    Effect::new(move |_| {
        search.track();
        load(false);
    });

    // Archive locally right away, then resync the list and the report.
    let apply_locally = move |row: &PersonalTransaction, pick: &Pick, revision: u32| {
        let id = row.transaction_id.clone();
        let pick = pick.clone();
        rows.update(|list| {
            for r in list.iter_mut().filter(|r| r.transaction_id == id) {
                r.role = pick.role.clone();
                r.assigned_category = if pick.role == "ignore" {
                    String::new()
                } else {
                    pick.category.clone()
                };
                r.revision = revision;
            }
        });
        load(true);
        app.refresh();
    };
    let chosen = Callback::new(move |saved: Saved| {
        apply_locally(&saved.row, &saved.next, saved.revision);
        sequence.update_value(|n| *n += 1);
        let toast = Toast {
            id: sequence.get_value(),
            message: pick_label(&saved.next),
            saved,
        };
        toasts.update(|list| list.insert(0, toast));
    });
    let dismiss = Callback::new(move |id: u64| toasts.update(|list| list.retain(|t| t.id != id)));
    let undo = Callback::new(move |toast: Toast| {
        spawn_local(async move {
            let s = &toast.saved;
            match save_pick(&s.row, s.revision, &s.previous).await {
                Ok(revision) => {
                    dismiss.run(toast.id);
                    apply_locally(&s.row, &s.previous, revision);
                }
                Err(e) => {
                    dismiss.run(toast.id);
                    error.set(format!("Undo failed: {e}"));
                }
            }
        });
    });

    let pending = Memo::new(move |_| {
        rows.get()
            .into_iter()
            .filter(|r| r.role.is_empty())
            .collect::<Vec<_>>()
    });
    let reviewed = Memo::new(move |_| {
        rows.get()
            .into_iter()
            .filter(|r| !r.role.is_empty())
            .collect::<Vec<_>>()
    });
    let since_text = move || {
        let s = since.get();
        if s.is_empty() {
            String::new()
        } else {
            since_label(&s)
        }
    };
    let row_view = move |row: PersonalTransaction| {
        view! { <PersonalPayment row=row categories=categories on_chosen=chosen /> }
    };

    view! {
        <section class=cn([card_class(CardTone::Default), "my-5 grid min-w-0 gap-4"]) aria-label="Personal payments">
            <div>
                <Eyebrow level=Heading::P>
                    {move || {
                        let s = since_text();
                        if s.is_empty() { "YOUR CHOICE".to_owned() } else { format!("YOUR CHOICE · SINCE {}", s.to_uppercase()) }
                    }}
                </Eyebrow>
                <h2 class=SECTION_HEADING>"Shared spending & money back"</h2>
            </div>
            <Help text="Money back reduces the category you choose. Money sent adds to it. No purchase link needed; no AI decisions. Bank memo cues only; nothing is assigned automatically." />
            <form
                class=FILTER_ROW
                on:submit=move |e| {
                    e.prevent_default();
                    search.set(query.get_untracked().trim().to_owned());
                }
            >
                <Field label="Find a person" id="personal-query" class=FORM_FIELD_WIDE>
                    <TextInput id="personal-query" kind="search" value=query maxlength=128 placeholder="e.g. Yuri or Nicolas" />
                </Field>
                <button type="submit" class=button_class(Variant::Primary, Size::Md)>"Search"</button>
                <button
                    type="button"
                    class=button_class(Variant::Secondary, Size::Md)
                    on:click=move |_| {
                        query.set(String::new());
                        search.set(String::new());
                    }
                >
                    "Show personal transfers"
                </button>
            </form>
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! { <p role="alert" class=ERROR_LINE>{e}</p> })
            }}
            {move || {
                if loading.get() {
                    return view! { <p role="status" class=MUTED>"Loading payments…"</p> }.into_any();
                }
                let (p, r, all) = (pending.get().len(), reviewed.get().len(), rows.with(Vec::len));
                let shown_total = total.get();
                let empty = if all > 0 {
                    "Nothing left to review.".to_owned()
                } else {
                    let s = since_text();
                    format!(
                        "No matching transactions{}. Own-account movements are excluded.",
                        if s.is_empty() { String::new() } else { format!(" since {s}") }
                    )
                };
                view! {
                    <FactsRow rows={
                        let mut rows = vec![
                            ("To review".to_owned(), p.to_string()),
                            ("Reviewed".to_owned(), r.to_string()),
                        ];
                        if shown_total > all {
                            rows.push(("Showing latest".to_owned(), format!("{all} of {shown_total}")));
                        }
                        rows
                    } />
                    <div class=PAYMENT_LIST>
                        <For
                            each=move || pending.get()
                            key=|row| format!("{}:{}", row.transaction_id, row.revision)
                            children=row_view
                        />
                    </div>
                    {(p == 0).then(|| view! { <p class=MUTED>{empty}</p> })}
                    {(r > 0).then(|| view! {
                        <button
                            type="button"
                            class=cn([button_class(Variant::Ghost, Size::Md).as_str(), "justify-self-start"])
                            aria-expanded=move || show_reviewed.get().to_string()
                            on:click=move |_| show_reviewed.update(|o| *o = !*o)
                        >
                            {move || if show_reviewed.get() { "Hide reviewed".to_owned() } else { format!("Show reviewed ({})", reviewed.with(Vec::len)) }}
                        </button>
                        <Show when=move || show_reviewed.get()>
                            <div class=PAYMENT_LIST aria-label="Reviewed transactions">
                                <For
                                    each=move || reviewed.get()
                                    key=|row| format!("{}:{}", row.transaction_id, row.revision)
                                    children=row_view
                                />
                            </div>
                        </Show>
                    })}
                }
                .into_any()
            }}
            <div class="pointer-events-none fixed bottom-4 left-1/2 z-50 flex w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 flex-col gap-2">
                <For
                    each=move || toasts.get()
                    key=|t| t.id
                    children=move |toast| view! { <UndoToast toast=toast on_undo=undo on_expire=dismiss /> }
                />
            </div>
        </section>
    }
}

#[component]
fn UndoToast(toast: Toast, on_undo: Callback<Toast>, on_expire: Callback<u64>) -> impl IntoView {
    let busy = RwSignal::new(false);
    let id = toast.id;
    let timer = set_timeout_with_handle(
        move || {
            if !busy.get_untracked() {
                on_expire.run(id);
            }
        },
        UNDO_WINDOW,
    )
    .ok();
    on_cleanup(move || {
        if let Some(t) = timer {
            t.clear();
        }
    });
    let description = toast.saved.row.description.clone();
    let message = toast.message.clone();
    let toast = StoredValue::new(toast);
    view! {
        <div class="pointer-events-auto flex items-center justify-between gap-4 rounded-lg bg-contrast px-4 py-3 text-fg-on-contrast shadow-pop" role="status">
            <span class="min-w-0 text-sm">
                {message}" "
                <small class="block break-words text-xs text-fg-on-contrast/75">{description}</small>
            </span>
            <button
                type="button"
                class=cn([button_class(Variant::Accent, Size::Sm).as_str(), "shrink-0"])
                disabled=move || busy.get()
                on:click=move |_| {
                    busy.set(true);
                    on_undo.run(toast.get_value());
                }
            >
                {move || if busy.get() { "Undoing…" } else { "Undo" }}
            </button>
        </div>
    }
}

#[component]
fn PersonalPayment(
    row: PersonalTransaction,
    categories: Memo<Vec<String>>,
    on_chosen: Callback<Saved>,
) -> impl IntoView {
    let role = RwSignal::new(row.role.clone());
    let category = RwSignal::new(row.assigned_category.clone());
    let other = RwSignal::new(
        !row.assigned_category.is_empty() && !EVERYDAY.contains(&row.assigned_category.as_str()),
    );
    let busy = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let row = StoredValue::new(row);
    let save = move |clear: bool| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(String::new());
        let next = if clear {
            Pick {
                role: String::new(),
                category: String::new(),
            }
        } else {
            let r = role.get_untracked();
            Pick {
                category: if r == "ignore" {
                    String::new()
                } else {
                    category.get_untracked()
                },
                role: r,
            }
        };
        let row = row.get_value();
        spawn_local(async move {
            match save_pick(&row, row.revision, &next).await {
                Ok(revision) => on_chosen.run(Saved {
                    previous: Pick {
                        role: row.role.clone(),
                        category: row.assigned_category.clone(),
                    },
                    row,
                    next,
                    revision,
                }),
                Err(e) => {
                    error.set(e);
                    busy.set(false);
                }
            }
        });
    };
    let r = row.get_value();
    let incoming = r.incoming;
    let status = match r.role.as_str() {
        "ignore" => "Not shared spending".to_owned(),
        "" => "Choose how to count this".to_owned(),
        role => format!(
            "{} · {}",
            if role == "return" {
                "Money back"
            } else {
                "Payment"
            },
            r.assigned_category
        ),
    };
    let had_role = !r.role.is_empty();
    let unchanged = move || {
        let (saved_role, saved_category) =
            row.with_value(|r| (r.role.clone(), r.assigned_category.clone()));
        let current = role.get();
        current == saved_role && (current == "ignore" || category.get() == saved_category)
    };
    let id = StoredValue::new(format!("personal-{}", r.transaction_id));
    let category_id = StoredValue::new(format!("{}-category", id.get_value()));
    view! {
        <Disclosure
            class="border-t border-line py-3"
            summary_class=SPLIT_ROW
            summary=view! {
                <span class=WRAP_TEXT>
                    <strong class="font-semibold">{r.description.clone()}</strong>
                    <small class=SUBLINE>
                        {format!("{} · {}", r.occurred_on, if incoming { "Received" } else { "Sent" })}
                    </small>
                </span>
                <span class=TRAILING>
                    {if incoming { "+" } else { "−" }}
                    <Money value=r.amount.amount unit=r.amount.currency.clone() neutral=true />
                    <small class=SUBLINE>{status}</small>
                </span>
            }
        >
            <fieldset disabled=move || busy.get() class="flex min-w-0 flex-wrap items-end gap-3 border-0 px-0 pb-2 pt-4">
                <Field label="This was" id=format!("{}-role", id.get_value()) class=FORM_FIELD_WIDE>
                    <Choice
                        id=format!("{}-role", id.get_value())
                        value=role
                        options=vec![
                            (String::new(), "Choose…".to_owned(), false),
                            ("return".to_owned(), "Money back to me · subtract spending".to_owned(), !incoming),
                            ("payment".to_owned(), "I paid someone · add spending".to_owned(), incoming),
                            ("ignore".to_owned(), "Not a shared expense · keep bank treatment".to_owned(), false),
                        ]
                        on_change=Callback::new(move |v| role.set(v))
                    />
                </Field>
                <Field label="For" id=format!("{}-for", id.get_value()) class=FORM_FIELD_NARROW>
                    <Choice
                        id=format!("{}-for", id.get_value())
                        disabled=Signal::derive(move || role.get() == "ignore")
                        value=Signal::derive(move || if other.get() { "__other".to_owned() } else { category.get() })
                        options=options([("", "Choose…"), ("Groceries", "Groceries"), ("Eating out", "Going out"), ("__other", "Something else")])
                        on_change=Callback::new(move |v: String| {
                            other.set(v == "__other");
                            category.set(if v == "__other" { String::new() } else { v });
                        })
                    />
                </Field>
                <Show when=move || other.get()>
                    <Field label="Category" id=category_id.get_value() class=FORM_FIELD_NARROW>
                        <Choice
                            id=category_id.get_value()
                            value=category
                            options=Signal::derive(move || {
                                let mut list = options([("", "Choose a category…")]);
                                list.extend(categories.get().into_iter().map(|c| (c.clone(), c, false)));
                                list
                            })
                            on_change=Callback::new(move |v| category.set(v))
                        />
                    </Field>
                </Show>
                <button
                    type="button"
                    class=button_class(Variant::Primary, Size::Md)
                    disabled=move || {
                        let r = role.get();
                        r.is_empty() || (r != "ignore" && category.get().is_empty()) || busy.get() || unchanged()
                    }
                    on:click=move |_| save(false)
                >
                    {move || if busy.get() { "Saving…" } else if had_role { "Change my choice" } else { "Save my choice" }}
                </button>
                {had_role.then(|| view! {
                    <button type="button" class=button_class(Variant::Secondary, Size::Md) on:click=move |_| save(true)>
                        "Clear my choice"
                    </button>
                })}
            </fieldset>
            <Help text="Applies to this transaction across spending, budgets and forecasts. Your bank amount stays unchanged." />
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! { <p role="alert" class=ERROR_LINE>{e}</p> })
            }}
        </Disclosure>
    }
}
