//! "Details & tags" on a reviewed transaction: merchant, location, payment
//! method, countries, tags, bulk payment and notes, prefilled from the
//! suggestions (personal-site `transaction-details.tsx`).

use finance_types::gateway::{OwnerDetails, OwnerTag, SavedDetails};
use finance_types::report::review::{DetailsSave, ReviewItem};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use wasm_bindgen::JsValue;

use super::{Choice, clean_error, options};
use crate::api::invoke;
use crate::ds::{Badge, CONTROL, Field, Size, Tone, Variant, button_class, cn};
use crate::state;

const OWNER_TAGS_LIMIT: usize = 20;
const TAG_LENGTH_LIMIT: usize = 80;
const DETAIL_LENGTH_LIMIT: usize = 200;
const NOTES_LENGTH_LIMIT: usize = 2000;

/// The enabled review-policy fields, in order.
const FIELDS: [(&str, &str); 5] = [
    ("merchant_name", "Merchant / payee"),
    ("merchant_location", "Merchant city / address"),
    ("payment_method", "Payment method"),
    ("from_country", "From country (ISO code)"),
    ("to_country", "To country (ISO code)"),
];

#[derive(Serialize)]
struct SaveArgs {
    details: DetailsSave,
}

fn get(values: &OwnerDetails, key: &str) -> String {
    match key {
        "merchant_name" => values.merchant_name.clone(),
        "merchant_location" => values.merchant_location.clone(),
        "payment_method" => values.payment_method.clone(),
        "from_country" => values.from_country.clone(),
        "to_country" => values.to_country.clone(),
        "notes" => values.notes.clone(),
        "bulk_payment_role" => values.bulk_payment_role.clone(),
        "bulk_payment_name" => values.bulk_payment_name.clone(),
        _ => String::new(),
    }
}

fn put(values: &mut OwnerDetails, key: &str, value: String) {
    match key {
        "merchant_name" => values.merchant_name = value,
        "merchant_location" => values.merchant_location = value,
        "payment_method" => values.payment_method = value,
        "from_country" => values.from_country = value,
        "to_country" => values.to_country = value,
        "notes" => values.notes = value,
        "bulk_payment_role" => values.bulk_payment_role = value,
        "bulk_payment_name" => values.bulk_payment_name = value,
        _ => {}
    }
}

pub fn tag_text(tag: &OwnerTag) -> String {
    if tag.parent.is_empty() {
        tag.label.clone()
    } else {
        format!("{} / {}", tag.parent, tag.label)
    }
}

fn tags_text(tags: &[OwnerTag]) -> String {
    tags.iter().map(tag_text).collect::<Vec<_>>().join(", ")
}

/// `parseTags`: "Food / Nachos, Work" → tags.
fn parse_tags(text: &str) -> Vec<OwnerTag> {
    text.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|part| match part.split_once('/') {
            None => OwnerTag {
                label: part.to_owned(),
                parent: String::new(),
            },
            Some((parent, label)) => OwnerTag {
                parent: parent.trim().to_owned(),
                label: label.trim().to_owned(),
            },
        })
        .collect()
}

/// The site's `detailValuesSchema`.
fn valid(values: &OwnerDetails) -> bool {
    let country =
        |c: &str| c.is_empty() || (c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase()));
    let len = |s: &str| s.chars().count();
    values.tags.len() <= OWNER_TAGS_LIMIT
        && values.tags.iter().all(|t| {
            (1..=TAG_LENGTH_LIMIT).contains(&len(&t.label)) && len(&t.parent) <= TAG_LENGTH_LIMIT
        })
        && [
            &values.merchant_name,
            &values.merchant_location,
            &values.payment_method,
        ]
        .iter()
        .all(|v| len(v) <= DETAIL_LENGTH_LIMIT)
        && country(&values.from_country)
        && country(&values.to_country)
        && len(&values.notes) <= NOTES_LENGTH_LIMIT
        && len(values.bulk_payment_name.trim()) <= 128
        && values.bulk_payment_role.is_empty() == values.bulk_payment_name.trim().is_empty()
}

/// The suggestion for one field: (value, evidence).
fn suggestion(item: &ReviewItem, key: &str) -> Option<String> {
    let extras = &item.extras;
    match key {
        "merchant_name" => extras.merchant_name.as_ref().map(|s| s.value.clone()),
        "merchant_location" => extras.merchant_location.as_ref().map(|s| s.value.clone()),
        "payment_method" => extras.payment_method.as_ref().map(|s| s.value.clone()),
        "from_country" => item
            .source_country
            .as_ref()
            .map(|c| c.code.clone())
            .or_else(|| {
                item.country_suggestions
                    .from_country
                    .as_ref()
                    .map(|c| c.code.clone())
            }),
        "to_country" => item
            .destination_country
            .as_ref()
            .map(|c| c.code.clone())
            .or_else(|| {
                item.country_suggestions
                    .to_country
                    .as_ref()
                    .map(|c| c.code.clone())
            }),
        _ => None,
    }
}

#[component]
pub fn TransactionDetails(
    item: ReviewItem,
    #[prop(optional)] on_editing: Option<Callback<bool>>,
) -> impl IntoView {
    let app = state();
    let editing = move |dirty: bool| {
        if let Some(f) = on_editing {
            f.run(dirty);
        }
    };
    let saved = item.owner_details.clone();
    let mut prefilled = saved.values.clone();
    for (key, _) in FIELDS {
        if get(&prefilled, key).is_empty()
            && let Some(value) = suggestion(&item, key)
        {
            put(&mut prefilled, key, value);
        }
    }
    if prefilled.tags.is_empty() {
        prefilled.tags = item
            .extras
            .tags
            .iter()
            .map(|t| OwnerTag {
                label: t.label.clone(),
                parent: t.parent.clone(),
            })
            .collect();
    }
    let prefill_count = FIELDS
        .iter()
        .filter(|(key, _)| get(&saved.values, key).is_empty() && suggestion(&item, key).is_some())
        .count()
        + if saved.values.tags.is_empty() {
            item.extras.tags.len()
        } else {
            0
        };
    let draft = RwSignal::new(None::<SavedDetails>);
    let tags = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let saved = StoredValue::new(saved);
    let item = StoredValue::new(item);
    let current = move || draft.get().unwrap_or_else(|| saved.get_value());
    let edit = move |key: &'static str, value: String| {
        if draft.with_untracked(Option::is_none) {
            tags.set(tags_text(&saved.with_value(|s| s.values.tags.clone())));
        }
        let mut next = draft.get_untracked().unwrap_or_else(|| saved.get_value());
        put(&mut next.values, key, value);
        draft.set(Some(next));
        editing(true);
    };
    let bulk_complete = move || {
        let c = current();
        c.values.bulk_payment_role.is_empty() == c.values.bulk_payment_name.trim().is_empty()
    };
    let save = move || {
        let Some(d) = draft.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return;
        }
        let mut values = d.values.clone();
        values.tags = parse_tags(&tags.get_untracked());
        if !valid(&values) {
            message.set(format!(
                "Check the bulk payment name, country codes and tags. Use ‘Food / Nachos’ for tags; up to {OWNER_TAGS_LIMIT} tags."
            ));
            return;
        }
        busy.set(true);
        message.set(String::new());
        let details = DetailsSave {
            proposal_id: item.with_value(|i| i.proposal_id.clone()),
            revision: d.revision,
            values,
        };
        spawn_local(async move {
            match invoke::<SavedDetails>("save_review_details", &SaveArgs { details }).await {
                Ok(result) => {
                    draft.set(Some(result));
                    message.set("Saved.".into());
                    editing(false);
                    app.refresh();
                    draft.set(None);
                }
                Err(e) => {
                    let e = clean_error(e);
                    message.set(if e.is_empty() {
                        "Could not save details.".into()
                    } else {
                        e
                    });
                }
            }
            busy.set(false);
        });
    };
    let prefilled = StoredValue::new(prefilled);
    let summary = {
        let s = saved.get_value();
        if !s.values.tags.is_empty() {
            format!(" · {}", tags_text(&s.values.tags))
        } else if prefill_count > 0 {
            format!(" · {prefill_count} ready")
        } else {
            String::new()
        }
    };
    let id = StoredValue::new(format!(
        "details-{}",
        item.with_value(|i| i.proposal_id.clone())
    ));
    let bulk_name_id = StoredValue::new(format!("{}-bulk-name", id.get_value()));

    view! {
        <details
            class="rounded-lg border border-border bg-card-muted px-4 py-3"
            on:toggle=move |e| {
                let open = e
                    .target()
                    .and_then(|t| js_sys::Reflect::get(&t, &JsValue::from_str("open")).ok())
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !open || draft.with_untracked(Option::is_some) || prefill_count == 0 {
                    return;
                }
                let values = prefilled.get_value();
                tags.set(tags_text(&values.tags));
                draft.set(Some(SavedDetails { revision: saved.with_value(|s| s.revision), values }));
            }
        >
            <summary class="cursor-pointer text-sm font-semibold">{format!("Details & tags{summary}")}</summary>
            {(prefill_count > 0).then(|| view! { <p class="mt-2 text-sm text-fg-muted">"Check and save."</p> })}
            <fieldset disabled=move || busy.get() class="mt-4 grid min-w-0 gap-4 border-0 p-0">
                {FIELDS
                    .into_iter()
                    .map(|(key, label)| {
                        let country = key.ends_with("country");
                        let suggested = item.with_value(|i| suggestion(i, key));
                        let field_id = format!("{}-{key}", id.get_value());
                        view! {
                            <div class="grid gap-1.5">
                                <Field label=label id=field_id.clone()>
                                    <input
                                        id=field_id
                                        class=CONTROL
                                        maxlength=if country { 2 } else { DETAIL_LENGTH_LIMIT }
                                        prop:value=move || get(&current().values, key)
                                        on:input=move |e| {
                                            let v = event_target_value(&e);
                                            edit(key, if country { v.to_uppercase() } else { v });
                                        }
                                    />
                                </Field>
                                {suggested.map(|s| {
                                    let s2 = s.clone();
                                    let s3 = s.clone();
                                    view! {
                                        <div class="flex flex-wrap items-center gap-2 text-xs text-fg-muted">
                                            <span class="break-words">
                                                {move || if get(&current().values, key) == s2 { "Prefilled".to_owned() } else { s2.clone() }}
                                            </span>
                                            {move || {
                                                let s = s3.clone();
                                                (get(&current().values, key) != s).then(|| view! {
                                                    <button
                                                        type="button"
                                                        class=button_class(Variant::Ghost, Size::Sm)
                                                        on:click=move |_| edit(key, s.clone())
                                                    >
                                                        "Use"
                                                    </button>
                                                })
                                            }}
                                        </div>
                                    }
                                })}
                            </div>
                        }
                    })
                    .collect_view()}
                <Field label="Extra tags" id=format!("{}-tags", id.get_value()) hint="Comma separated · use “parent / tag” for hierarchy">
                    <input
                        id=format!("{}-tags", id.get_value())
                        class=CONTROL
                        placeholder="Food / Nachos, Work / Lunch"
                        maxlength=OWNER_TAGS_LIMIT * (TAG_LENGTH_LIMIT * 2 + 5)
                        prop:value=move || {
                            if draft.get().is_some() { tags.get() } else { tags_text(&saved.with_value(|s| s.values.tags.clone())) }
                        }
                        on:input=move |e| {
                            if draft.with_untracked(Option::is_none) {
                                draft.set(Some(saved.get_value()));
                            }
                            tags.set(event_target_value(&e));
                            editing(true);
                        }
                    />
                </Field>
                {item.with_value(|i| i.extras.tags.clone())
                    .into_iter()
                    .map(|tag| {
                        let owner = OwnerTag { label: tag.label.clone(), parent: tag.parent.clone() };
                        let text = StoredValue::new(tag_text(&owner));
                        let owner = StoredValue::new(owner);
                        view! {
                            <div class="flex flex-wrap items-center gap-2">
                                <Badge tone=Tone::Neutral>{text.get_value()}</Badge>
                                <button
                                    type="button"
                                    class=button_class(Variant::Ghost, Size::Sm)
                                    on:click=move |_| {
                                        let mut existing = if draft.with_untracked(Option::is_some) {
                                            parse_tags(&tags.get_untracked())
                                        } else {
                                            saved.with_value(|s| s.values.tags.clone())
                                        };
                                        if !existing.iter().any(|t| tag_text(t).to_lowercase() == text.get_value().to_lowercase()) {
                                            existing.push(owner.get_value());
                                            tags.set(tags_text(&existing));
                                            if draft.with_untracked(Option::is_none) {
                                                draft.set(Some(saved.get_value()));
                                            }
                                            editing(true);
                                        }
                                    }
                                >
                                    "Add tag"
                                </button>
                            </div>
                        }
                    })
                    .collect_view()}
                <div class="grid gap-3">
                    <Field label="Bulk payment handling" id=format!("{}-bulk", id.get_value())>
                        <Choice
                            id=format!("{}-bulk", id.get_value())
                            value=Signal::derive(move || current().values.bulk_payment_role)
                            options=options([
                                ("", "Individual payment"),
                                ("component", "Included in a bulk payment"),
                                ("settlement", "The combined settlement payment"),
                            ])
                            on_change=Callback::new(move |role: String| {
                                if draft.with_untracked(Option::is_none) {
                                    tags.set(tags_text(&saved.with_value(|s| s.values.tags.clone())));
                                }
                                let mut next = draft.get_untracked().unwrap_or_else(|| saved.get_value());
                                if role.is_empty() {
                                    next.values.bulk_payment_name = String::new();
                                }
                                next.values.bulk_payment_role = role;
                                draft.set(Some(next));
                                editing(true);
                            })
                        />
                    </Field>
                    {move || (!current().values.bulk_payment_role.is_empty()).then(|| view! {
                        <Field label="Bulk payment name" id=bulk_name_id.get_value()>
                            <input
                                id=bulk_name_id.get_value()
                                class=CONTROL
                                required=true
                                maxlength=128
                                placeholder="e.g. Klarna September"
                                prop:value=move || current().values.bulk_payment_name
                                on:input=move |e| edit("bulk_payment_name", event_target_value(&e))
                            />
                        </Field>
                    })}
                    {move || (!bulk_complete()).then(|| view! {
                        <p role="status" class="text-sm text-warning-fg">"Give this bulk payment a name before saving."</p>
                    })}
                    <p class="text-sm text-fg-muted">"Use the same name for its purchases and final payment."</p>
                </div>
                <Field label="Your notes" id=format!("{}-notes", id.get_value())>
                    <textarea
                        id=format!("{}-notes", id.get_value())
                        class=cn([CONTROL, "h-auto min-h-20 py-2", "resize-y"])
                        maxlength=NOTES_LENGTH_LIMIT
                        rows=3
                        prop:value=move || current().values.notes
                        on:input=move |e| edit("notes", event_target_value(&e))
                    ></textarea>
                </Field>
                <div class="flex flex-wrap items-center gap-2">
                    <button
                        type="button"
                        class=button_class(Variant::Primary, Size::Md)
                        disabled=move || draft.get().is_none() || busy.get() || !bulk_complete()
                        on:click=move |_| save()
                    >
                        {move || if busy.get() { "Saving…" } else { "Save details" }}
                    </button>
                    <button
                        type="button"
                        class=button_class(Variant::Ghost, Size::Md)
                        disabled=move || draft.get().is_none() || busy.get()
                        on:click=move |_| {
                            draft.set(None);
                            message.set(String::new());
                            editing(false);
                        }
                    >
                        "Discard edits"
                    </button>
                </div>
            </fieldset>
            {move || {
                let m = message.get();
                (!m.is_empty()).then(|| view! { <p role="status" class="mt-3 text-sm text-fg-muted">{m}</p> })
            }}
        </details>
    }
}
