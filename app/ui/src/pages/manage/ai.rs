//! Manage → Settings → "AI": the Mistral key (categories, Gisel's emails
//! and his daily brief), which model proposes categories (off, Mistral or
//! TypeSafe "Jev"), research and automatic approval. Keys go to the secret
//! store and never come back.

use finance_types::ai::{AiSettings, AiStatus, Classifier};
use leptos::html::Input as InputElement;
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{ERROR, FEEDBACK, Ops};
use crate::api::{NoArgs, load};
use crate::ds::patterns::{ACTIONS, CHECK_SLOT, OPTION_SPACED, SETTINGS_GRID, SMALL};
use crate::ds::{
    Button, CONTROL, Card, CardTitle, Checkbox, Eyebrow, FactsRow, Field, Heading, Help, Input,
    Select, Size, Variant, cn,
};
use crate::format;

#[derive(Serialize)]
struct SettingsArgs {
    settings: AiSettings,
}

#[derive(Serialize)]
struct KeyArgs {
    key: String,
}

fn classifier_value(c: Classifier) -> &'static str {
    match c {
        Classifier::Off => "off",
        Classifier::Mistral => "mistral",
        Classifier::TypeSafe => "typesafe",
    }
}

/// The form's editable copy of the settings.
#[derive(Clone, Copy)]
struct Draft {
    classifier: RwSignal<String>,
    model: RwSignal<String>,
    embeddings: RwSignal<bool>,
    typesafe_model: RwSignal<String>,
    research: RwSignal<bool>,
    auto_apply: RwSignal<bool>,
    batch: u32,
}

impl Draft {
    fn new() -> Self {
        Self {
            classifier: RwSignal::new("off".into()),
            model: RwSignal::new(String::new()),
            embeddings: RwSignal::new(false),
            typesafe_model: RwSignal::new(String::new()),
            research: RwSignal::new(false),
            auto_apply: RwSignal::new(false),
            batch: 20,
        }
    }

    fn fill(&mut self, s: &AiSettings) {
        self.classifier.set(classifier_value(s.classifier).into());
        self.model.set(s.mistral_model.clone());
        self.embeddings.set(s.embeddings);
        self.typesafe_model.set(s.typesafe_model.clone());
        self.research.set(s.research);
        self.auto_apply.set(s.auto_apply);
        self.batch = s.batch_size;
    }

    fn settings(&self) -> AiSettings {
        AiSettings {
            classifier: match self.classifier.get_untracked().as_str() {
                "mistral" => Classifier::Mistral,
                "typesafe" => Classifier::TypeSafe,
                _ => Classifier::Off,
            },
            mistral_model: self.model.get_untracked(),
            embeddings: self.embeddings.get_untracked(),
            typesafe_model: self.typesafe_model.get_untracked(),
            research: self.research.get_untracked(),
            auto_apply: self.auto_apply.get_untracked(),
            batch_size: self.batch,
        }
    }
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(v, l)| ((*v).to_owned(), (*l).to_owned()))
        .collect()
}

#[component]
pub fn AiSuggestions() -> impl IntoView {
    let changed = RwSignal::new(0_u32);
    let status = load::<AiStatus, _>("ai_status", move || {
        changed.track();
        NoArgs {}
    });
    let ops = Ops::new();
    let draft = StoredValue::new(Draft::new());
    let filled = RwSignal::new(false);
    Effect::new(move |_| {
        if let Some(Ok(s)) = status.data.get()
            && !filled.get_untracked()
        {
            draft.update_value(|d| d.fill(&s.settings));
            filled.set(true);
        }
    });
    let d = draft.get_value();
    let busy = ops.busy;
    let save = move |_| {
        let settings = draft.get_value().settings();
        ops.run(
            "save_ai_settings",
            SettingsArgs { settings },
            move |s: AiStatus| {
                draft.update_value(|d| d.fill(&s.settings));
                ops.notice.set("AI settings saved.".into());
                changed.update(|n| *n += 1);
            },
        );
    };
    let suggest = move |_| {
        ops.run(
            "suggest_categories",
            NoArgs {},
            move |p: finance_types::ai::SuggestionPass| {
                ops.notice.set(format!(
                    "Suggested categories for {} transaction{}{}.",
                    p.suggested,
                    if p.suggested == 1 { "" } else { "s" },
                    if p.applied > 0 {
                        format!("; saved {} automatically", p.applied)
                    } else {
                        String::new()
                    }
                ));
                changed.update(|n| *n += 1);
            },
        );
    };
    let key_input = NodeRef::<InputElement>::new();
    let save_key = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(el) = key_input.get() else { return };
        let key = el.value();
        el.set_value("");
        ops.run("set_typesafe_key", KeyArgs { key }, move |(): ()| {
            ops.notice.set("TypeSafe key saved privately.".into());
            changed.update(|n| *n += 1);
        });
    };
    let forget_key = move |_| {
        ops.run("forget_typesafe_key", NoArgs {}, move |(): ()| {
            changed.update(|n| *n += 1);
        });
    };
    let mistral_input = NodeRef::<InputElement>::new();
    let save_mistral = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(el) = mistral_input.get() else {
            return;
        };
        let key = el.value();
        el.set_value("");
        ops.run("set_mistral_key", KeyArgs { key }, move |(): ()| {
            ops.notice.set("Mistral key saved privately.".into());
            changed.update(|n| *n += 1);
        });
    };
    let forget_mistral = move |_| {
        ops.run("forget_mistral_key", NoArgs {}, move |(): ()| {
            changed.update(|n| *n += 1);
        });
    };
    let mistral_set = move || {
        status
            .data
            .get()
            .and_then(Result::ok)
            .is_some_and(|s| s.mistral_key_set)
    };
    let is = move |value: &'static str| move || d.classifier.get() == value;

    let last_run = move || {
        status.data.get().and_then(Result::ok).map(|s| {
            let rows = if s.running {
                vec![("Suggestion pass".to_owned(), "Running…".to_owned())]
            } else if let Some(r) = s.last_run {
                let when = format::date(r.finished_at.get(..10).unwrap_or(""));
                match r.error {
                    Some(e) => vec![("Last pass".to_owned(), when), ("Error".to_owned(), e)],
                    None => vec![
                        ("Last pass".to_owned(), when),
                        ("Suggested".to_owned(), r.suggested.to_string()),
                        ("Saved automatically".to_owned(), r.applied.to_string()),
                        (
                            "Took".to_owned(),
                            format!("{:.1} s", r.millis as f64 / 1000.0),
                        ),
                    ],
                }
            } else {
                vec![("Last pass".to_owned(), "None this session".to_owned())]
            };
            view! { <FactsRow rows=rows /> }
        })
    };
    let key_set = move || {
        status
            .data
            .get()
            .and_then(Result::ok)
            .is_some_and(|s| s.typesafe_key_set)
    };

    view! {
        <Card>
            <Eyebrow level=Heading::P>"CATEGORIES"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"AI"</CardTitle>
            <Help text="Mistral AI (Paris, EU-hosted) suggests categories, writes Gisel's email lines and her daily brief, and speaks it. Suggestions only fill the review inbox; nothing changes a category until you approve it, unless automatic approval is on." />
            {move || if mistral_set() {
                view! {
                    <div class="mb-5 flex flex-wrap items-center gap-2">
                        <Help text="Mistral key is saved privately on the server." />
                        <Button variant=Variant::Ghost size=Size::Sm disabled=Signal::derive(move || busy.get()) on_click=Callback::new(forget_mistral)>
                            "Remove key"
                        </Button>
                    </div>
                }.into_any()
            } else {
                view! {
                    <form class="mb-5 flex flex-col gap-3 sm:flex-row sm:items-end" on:submit=save_mistral>
                        <input
                            node_ref=mistral_input
                            class=cn([CONTROL, "sm:max-w-sm"])
                            type="password"
                            autocomplete="off"
                            placeholder="Mistral API key"
                            aria-label="Mistral API key"
                        />
                        <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || busy.get())>
                            "Save Mistral key"
                        </Button>
                    </form>
                }.into_any()
            }}
            <div class=SETTINGS_GRID>
                <Field label="Classifier" id="ai-classifier">
                    <Select
                        id="ai-classifier"
                        value=d.classifier
                        options=Signal::derive(|| options(&[("off", "Off"), ("typesafe", "TypeSafe (Jev)"), ("mistral", "Mistral")]))
                        disabled=Signal::derive(move || busy.get())
                    />
                </Field>
                <Field label="Mistral model" id="ai-model" hint="mistral-small-latest is cheap and good at this; mistral-medium-latest is sharper.">
                    <Input id="ai-model" value=d.model disabled=Signal::derive(move || busy.get()) />
                </Field>
                <Show when=is("typesafe")>
                    <Field label="TypeSafe model" id="ai-ts-model">
                        <Input id="ai-ts-model" value=d.typesafe_model disabled=Signal::derive(move || busy.get()) />
                    </Field>
                </Show>
            </div>
            <Show when=move || d.classifier.get() != "off">
                <label class="mb-3 flex items-start gap-2.5 text-sm leading-relaxed">
                    <span class=CHECK_SLOT>
                        <Checkbox
                            checked=Signal::derive(move || d.embeddings.get())
                            disabled=Signal::derive(move || busy.get())
                            on_change=Callback::new(move |v| d.embeddings.set(v))
                        />
                    </span>
                    <span>
                        "Find similar past transactions"
                        <span class=SMALL>"Uses mistral-embed to put your closest past transactions in the prompt. Off: text matches only."</span>
                    </span>
                </label>
            </Show>
            <Show when=is("typesafe")>
                {move || if key_set() {
                    view! {
                        <div class="mb-5 flex flex-wrap items-center gap-2">
                            <Help text="TypeSafe key is saved privately." />
                            <Button variant=Variant::Ghost size=Size::Sm disabled=Signal::derive(move || busy.get()) on_click=Callback::new(forget_key)>
                                "Remove key"
                            </Button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <form class="mb-5 flex flex-col gap-3 sm:flex-row sm:items-end" on:submit=save_key>
                            <input
                                node_ref=key_input
                                class=cn([CONTROL, "sm:max-w-sm"])
                                type="password"
                                autocomplete="off"
                                placeholder="TypeSafe API key"
                                aria-label="TypeSafe API key"
                            />
                            <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || busy.get())>
                                "Save key"
                            </Button>
                        </form>
                    }.into_any()
                }}
            </Show>
            <label class="mb-3 flex items-start gap-2.5 text-sm leading-relaxed">
                <span class=CHECK_SLOT>
                    <Checkbox
                        checked=Signal::derive(move || d.research.get())
                        disabled=Signal::derive(move || busy.get())
                        on_change=Callback::new(move |v| d.research.set(v))
                    />
                </span>
                <span>
                    "Public merchant research"
                    <span class=SMALL>"Looks up a public business name on Wikipedia when you ask, or for Jev group suggestions. Only the name is looked up."</span>
                </span>
            </label>
            <label class=OPTION_SPACED>
                <span class=CHECK_SLOT>
                    <Checkbox
                        checked=Signal::derive(move || d.auto_apply.get())
                        disabled=Signal::derive(move || busy.get())
                        on_change=Callback::new(move |v| d.auto_apply.set(v))
                    />
                </span>
                <span>
                    "Save confident suggestions automatically"
                    <span class=SMALL>"Jev suggestions at 90%+ category probability, with no conflicting merchant history, are saved without asking. Every one stays visible in the daily review."</span>
                </span>
            </label>
            <div class=ACTIONS>
                <Button disabled=Signal::derive(move || busy.get()) on_click=Callback::new(save)>
                    "Save AI settings"
                </Button>
                <Button
                    variant=Variant::Secondary
                    disabled=Signal::derive(move || busy.get() || d.classifier.get() == "off")
                    on_click=Callback::new(suggest)
                >
                    {move || if busy.get() { "Working…" } else { "Suggest categories now" }}
                </Button>
            </div>
            <div class="mt-3">{last_run}</div>
            {move || {
                let n = ops.notice.get();
                (!n.is_empty()).then(|| view! { <p role="status" class=cn([FEEDBACK, "mt-4"])>{n}</p> })
            }}
            {move || {
                let e = ops.error.get();
                (!e.is_empty()).then(|| view! { <p role="alert" class=cn([ERROR, "mt-4"])>{e}</p> })
            }}
        </Card>
    }
}
