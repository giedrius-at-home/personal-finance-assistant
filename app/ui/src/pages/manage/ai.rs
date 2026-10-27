//! Manage → Settings → "AI suggestions": which model proposes categories
//! (off, the local Ollama or TypeSafe "Jev"), research and automatic
//! approval. The TypeSafe key goes to the secret store and never comes back.

use finance_types::ai::{AiSettings, AiStatus, Classifier};
use leptos::html::Input as InputElement;
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{ERROR, FEEDBACK, Ops};
use crate::api::{NoArgs, load};
use crate::ds::{
    Button, CONTROL, Card, CardTitle, Checkbox, Eyebrow, Field, Heading, Input, Select, Size,
    Variant, cn,
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
        Classifier::Ollama => "ollama",
        Classifier::TypeSafe => "typesafe",
    }
}

/// The form's editable copy of the settings.
#[derive(Clone, Copy)]
struct Draft {
    classifier: RwSignal<String>,
    url: RwSignal<String>,
    model: RwSignal<String>,
    embedding: RwSignal<String>,
    typesafe_model: RwSignal<String>,
    research: RwSignal<bool>,
    auto_apply: RwSignal<bool>,
    batch: u32,
}

impl Draft {
    fn new() -> Self {
        Self {
            classifier: RwSignal::new("off".into()),
            url: RwSignal::new(String::new()),
            model: RwSignal::new(String::new()),
            embedding: RwSignal::new(String::new()),
            typesafe_model: RwSignal::new(String::new()),
            research: RwSignal::new(false),
            auto_apply: RwSignal::new(false),
            batch: 20,
        }
    }

    fn fill(&mut self, s: &AiSettings) {
        self.classifier.set(classifier_value(s.classifier).into());
        self.url.set(s.ollama_url.clone());
        self.model.set(s.ollama_model.clone());
        self.embedding.set(s.embedding_model.clone());
        self.typesafe_model.set(s.typesafe_model.clone());
        self.research.set(s.research);
        self.auto_apply.set(s.auto_apply);
        self.batch = s.batch_size;
    }

    fn settings(&self) -> AiSettings {
        AiSettings {
            classifier: match self.classifier.get_untracked().as_str() {
                "ollama" => Classifier::Ollama,
                "typesafe" => Classifier::TypeSafe,
                _ => Classifier::Off,
            },
            ollama_url: self.url.get_untracked(),
            ollama_model: self.model.get_untracked(),
            embedding_model: self.embedding.get_untracked(),
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

/// The installed models, with the current choice kept even when missing.
fn model_options(models: &[String], current: &str, none: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = none
        .map(|label| (String::new(), label.to_owned()))
        .into_iter()
        .collect();
    if !current.is_empty() && !models.iter().any(|m| m == current) {
        out.push((current.to_owned(), format!("{current} (not installed)")));
    }
    out.extend(models.iter().map(|m| (m.clone(), m.clone())));
    out
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
            ops.notice
                .set("TypeSafe key saved privately on this Mac.".into());
            changed.update(|n| *n += 1);
        });
    };
    let forget_key = move |_| {
        ops.run("forget_typesafe_key", NoArgs {}, move |(): ()| {
            changed.update(|n| *n += 1);
        });
    };
    let is = move |value: &'static str| move || d.classifier.get() == value;

    let ollama_line = move || {
        status.data.get().and_then(Result::ok).map(|s| {
            let (text, tone) = if !s.ollama_detected {
                ("Ollama is not answering at this address.".to_owned(), "text-fg-muted")
            } else if !s.model_available {
                (
                    format!(
                        "Ollama detected · {} is not available on this Ollama. Choose an installed model or pull it with ollama pull.",
                        s.settings.ollama_model
                    ),
                    "text-warning-fg",
                )
            } else {
                (
                    format!("Ollama detected · {} models installed", s.ollama_models.len()),
                    "text-positive-fg",
                )
            };
            view! { <p class=cn(["text-xs", tone]) role="status">{text}</p> }
        })
    };
    let models = Memo::new(move |_| {
        status
            .data
            .get()
            .and_then(Result::ok)
            .map(|s| s.ollama_models)
            .unwrap_or_default()
    });
    let model_choices = Signal::derive(move || model_options(&models.get(), &d.model.get(), None));
    let embedding_choices = Signal::derive(move || {
        model_options(
            &models.get(),
            &d.embedding.get(),
            Some("None (text matches only)"),
        )
    });
    let last_run = move || {
        status.data.get().and_then(Result::ok).map(|s| {
            let line = if s.running {
                "A suggestion pass is running…".to_owned()
            } else if let Some(r) = s.last_run {
                let when = format::date(r.finished_at.get(..10).unwrap_or(""));
                match r.error {
                    Some(e) => format!("Last pass {when}: {e}"),
                    None => format!(
                        "Last pass {when}: {} suggested, {} saved automatically, {:.1} s",
                        r.suggested,
                        r.applied,
                        r.millis as f64 / 1000.0
                    ),
                }
            } else {
                "No pass yet in this session.".to_owned()
            };
            view! { <p class="text-xs text-fg-muted">{line}</p> }
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
            <CardTitle level=Heading::H2 class="mt-1">"AI suggestions"</CardTitle>
            <p class="mt-2 text-sm text-fg-muted">
                "Suggestions only fill the review inbox; nothing changes a category until you approve it, unless automatic approval is on."
            </p>
            <div class="my-5 grid gap-5 md:grid-cols-3">
                <Field label="Classifier" id="ai-classifier">
                    <Select
                        id="ai-classifier"
                        value=d.classifier
                        options=Signal::derive(|| options(&[("off", "Off"), ("typesafe", "TypeSafe (Jev)"), ("ollama", "Ollama (on this Mac)")]))
                        disabled=Signal::derive(move || busy.get())
                    />
                </Field>
                <Show when=is("ollama")>
                    <Field label="Ollama address" id="ai-url" hint="Only a local address is accepted.">
                        <Input id="ai-url" value=d.url disabled=Signal::derive(move || busy.get()) />
                    </Field>
                    <Field label="Model" id="ai-model">
                        <Select id="ai-model" value=d.model options=model_choices disabled=Signal::derive(move || busy.get()) />
                    </Field>
                </Show>
                <Show when=is("typesafe")>
                    <Field label="TypeSafe model" id="ai-ts-model">
                        <Input id="ai-ts-model" value=d.typesafe_model disabled=Signal::derive(move || busy.get()) />
                    </Field>
                </Show>
                <Show when=move || d.classifier.get() != "off">
                    <Field label="Embedding model" id="ai-embedding" hint="Finds similar past transactions for the prompt. Served by the Ollama address above.">
                        <Select id="ai-embedding" value=d.embedding options=embedding_choices disabled=Signal::derive(move || busy.get()) />
                    </Field>
                </Show>
            </div>
            <Show when=is("ollama")>
                <div class="mb-5">{ollama_line}</div>
            </Show>
            <Show when=is("typesafe")>
                {move || if key_set() {
                    view! {
                        <div class="mb-5 flex flex-wrap items-center gap-2">
                            <p class="text-sm text-fg-muted">"TypeSafe key is saved privately on this Mac."</p>
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
                <span class="mt-1 flex">
                    <Checkbox
                        checked=Signal::derive(move || d.research.get())
                        disabled=Signal::derive(move || busy.get())
                        on_change=Callback::new(move |v| d.research.set(v))
                    />
                </span>
                <span>
                    "Public merchant research"
                    <span class="block text-xs text-fg-muted">"Looks up a public business name on Wikipedia when you ask, or for Jev group suggestions. Only the name leaves this Mac."</span>
                </span>
            </label>
            <label class="mb-5 flex items-start gap-2.5 text-sm leading-relaxed">
                <span class="mt-1 flex">
                    <Checkbox
                        checked=Signal::derive(move || d.auto_apply.get())
                        disabled=Signal::derive(move || busy.get())
                        on_change=Callback::new(move |v| d.auto_apply.set(v))
                    />
                </span>
                <span>
                    "Save confident suggestions automatically"
                    <span class="block text-xs text-fg-muted">"Jev suggestions at 90%+ category probability, with no conflicting merchant history, are saved without asking. Every one stays visible in the daily review."</span>
                </span>
            </label>
            <div class="flex flex-wrap items-center gap-2">
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
