//! Manage → Settings → Daily voice brief: once a day at the chosen hour,
//! Gisel or Rafael (Mistral) talks through what is left, what is close to
//! its limit, the bills coming and how your habits moved; Voxtral speaks it
//! and the phone gets a push that opens the player on Overview. Every dial
//! is here, with "Preview" to hear the form as it stands before saving.

use finance_types::notify::{
    BriefLength, BriefPreview, BriefSettings, BriefStatus, Persona, VoiceOption, mood_versions,
};
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{ERROR, FEEDBACK, Ops};
use crate::api::{NoArgs, load};
use crate::ds::patterns::{
    ACTIONS, CHECK_SLOT, HINT, MUTED, OPTION_SPACED, SETTINGS_GRID, SMALL, STATUS_NOTE,
};
use crate::ds::{
    Button, Card, CardTitle, Checkbox, Eyebrow, Field, Heading, Help, Input, Select, Variant, cn,
};

#[derive(Serialize)]
struct SettingsArgs {
    settings: BriefSettings,
}

#[derive(Clone, Copy)]
struct Draft {
    enabled: RwSignal<bool>,
    hour: RwSignal<String>,
    voice: RwSignal<bool>,
    persona: RwSignal<String>,
    voice_id: RwSignal<String>,
    expressiveness: RwSignal<String>,
    length: RwSignal<String>,
    match_mood: RwSignal<bool>,
    style: RwSignal<String>,
}

fn persona_value(p: Persona) -> &'static str {
    match p {
        Persona::Gisel => "gisel",
        Persona::Rafael => "rafael",
    }
}

fn persona_of(value: &str) -> Persona {
    if value == "rafael" {
        Persona::Rafael
    } else {
        Persona::Gisel
    }
}

fn length_value(l: BriefLength) -> &'static str {
    match l {
        BriefLength::Short => "short",
        BriefLength::Normal => "normal",
        BriefLength::Long => "long",
    }
}

fn length_of(value: &str) -> BriefLength {
    match value {
        "short" => BriefLength::Short,
        "long" => BriefLength::Long,
        _ => BriefLength::Normal,
    }
}

/// "1.4" → 14 (tenths), when it is between 1.0 and 2.0.
fn tenths(value: &str) -> Option<u8> {
    let e: f64 = value.trim().parse().ok()?;
    (1.0..=2.0).contains(&e).then(|| (e * 10.0).round() as u8)
}

impl Draft {
    fn new() -> Self {
        Self {
            enabled: RwSignal::new(false),
            hour: RwSignal::new("8".into()),
            voice: RwSignal::new(true),
            persona: RwSignal::new("gisel".into()),
            voice_id: RwSignal::new(String::new()),
            expressiveness: RwSignal::new("1.4".into()),
            length: RwSignal::new("normal".into()),
            match_mood: RwSignal::new(true),
            style: RwSignal::new(String::new()),
        }
    }

    fn fill(&self, s: &BriefSettings) {
        self.enabled.set(s.enabled);
        self.hour.set(s.hour.to_string());
        self.voice.set(s.voice);
        self.persona.set(persona_value(s.persona).into());
        self.voice_id.set(s.voice_id.clone());
        self.expressiveness
            .set(format!("{:.1}", s.expressiveness()));
        self.length.set(length_value(s.length).into());
        self.match_mood.set(s.match_mood);
        self.style.set(s.style.clone());
    }

    fn settings(&self) -> Result<BriefSettings, String> {
        let hour = self
            .hour
            .get_untracked()
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|h| *h <= 23)
            .ok_or("The hour must be between 0 and 23.")?;
        let expressiveness = tenths(&self.expressiveness.get_untracked())
            .ok_or("Expressiveness goes from 1.0 to 2.0.")?;
        Ok(BriefSettings {
            enabled: self.enabled.get_untracked(),
            hour,
            voice: self.voice.get_untracked(),
            persona: persona_of(&self.persona.get_untracked()),
            voice_id: self.voice_id.get_untracked(),
            expressiveness,
            length: length_of(&self.length.get_untracked()),
            match_mood: self.match_mood.get_untracked(),
            style: self.style.get_untracked(),
        })
    }
}

fn check(
    label: &'static str,
    hint: &'static str,
    value: RwSignal<bool>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <label class=OPTION_SPACED>
            <span class=CHECK_SLOT>
                <Checkbox
                    checked=Signal::derive(move || value.get())
                    disabled=Signal::derive(move || busy.get())
                    on_change=Callback::new(move |v| value.set(v))
                />
            </span>
            <span>
                {label}
                <span class=SMALL>{hint}</span>
            </span>
        </label>
    }
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(v, l)| ((*v).to_owned(), (*l).to_owned()))
        .collect()
}

/// "Elena (es, female)", "My voice · your voice".
fn voice_label(v: &VoiceOption) -> String {
    let mut details: Vec<String> = v.languages.clone();
    if let Some(g) = &v.gender {
        details.push(g.clone());
    }
    let details = if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    };
    let mine = if v.custom { " · your voice" } else { "" };
    format!("{}{details}{mine}", v.name)
}

#[component]
pub fn BriefCard() -> impl IntoView {
    let status = load::<BriefStatus, _>("brief_status", || NoArgs {});
    let ops = Ops::new();
    let busy = ops.busy;
    let draft = Draft::new();
    let filled = RwSignal::new(false);
    let latest = RwSignal::new(None::<finance_types::notify::Brief>);
    let preview = RwSignal::new(None::<BriefPreview>);
    Effect::new(move |_| {
        if let Some(Ok(s)) = status.data.get()
            && !filled.get_untracked()
        {
            draft.fill(&s.settings);
            latest.set(s.latest.clone());
            filled.set(true);
        }
    });
    let list = Memo::new(move |_| {
        status
            .data
            .get()
            .and_then(Result::ok)
            .map(|s| s.voices)
            .unwrap_or_default()
    });
    let persona = move || persona_of(&draft.persona.get());
    let voices = Signal::derive(move || {
        let p = persona();
        let mut out = vec![(
            String::new(),
            format!("{} ({}'s own voice)", p.default_voice_name(), p.name()),
        )];
        let current = draft.voice_id.get();
        let list = list.get();
        if !current.is_empty() && !list.iter().any(|v| v.id == current || v.slug == current) {
            out.push((current.clone(), current));
        }
        out.extend(list.iter().map(|v| (v.id.clone(), voice_label(v))));
        out
    });
    // The persona picker: switching brings that persona's own voice back.
    let persona_choice = RwSignal::new("gisel".to_owned());
    Effect::new(move |_| persona_choice.set(draft.persona.get()));
    Effect::new(move |_| {
        let v = persona_choice.get();
        if v != draft.persona.get_untracked() {
            draft.persona.set(v);
            draft.voice_id.set(String::new());
        }
    });
    let mood_note = move || {
        if !draft.match_mood.get() || !draft.voice.get() {
            return None;
        }
        let list = list.get();
        if list.is_empty() {
            return None;
        }
        let settings = BriefSettings {
            persona: persona(),
            voice_id: draft.voice_id.get(),
            ..BriefSettings::default()
        };
        let chosen = settings.chosen_voice().to_owned();
        let versions = mood_versions(&chosen, &list);
        let name = list
            .iter()
            .find(|v| v.id == chosen || v.slug == chosen)
            .map_or_else(|| "This voice".to_owned(), |v| v.name.clone());
        Some(if versions.is_empty() {
            format!("{name} has no mood versions, so the mood comes through in the words only.")
        } else {
            let moods: Vec<String> = versions.into_iter().map(|(_, m)| m).collect();
            format!("Moods for this voice: {}.", moods.join(", "))
        })
    };
    let save = move |_| match draft.settings() {
        Err(e) => ops.error.set(e),
        Ok(settings) => ops.run(
            "save_brief_settings",
            SettingsArgs { settings },
            move |s: BriefStatus| {
                draft.fill(&s.settings);
                ops.notice.set(if s.settings.enabled {
                    format!(
                        "Saved. {}'s brief goes out daily after {}:00.",
                        s.settings.persona.name(),
                        s.settings.hour
                    )
                } else {
                    "Saved. The daily brief is off.".into()
                });
            },
        ),
    };
    let try_it = move |_| match draft.settings() {
        Err(e) => ops.error.set(e),
        Ok(settings) => {
            preview.set(None);
            ops.run(
                "brief_preview",
                SettingsArgs { settings },
                move |p: BriefPreview| {
                    ops.notice.set(if p.audio.is_some() {
                        "This is today's brief with these settings (not saved, not sent).".into()
                    } else {
                        "These are today's words with these settings (no audio).".into()
                    });
                    preview.set(Some(p));
                },
            );
        }
    };
    let now = move |_| {
        ops.run("brief_now", NoArgs {}, move |s: BriefStatus| {
            let spoken = s.latest.as_ref().is_some_and(|b| b.has_audio);
            latest.set(s.latest);
            ops.notice.set(if spoken {
                "Today's brief is ready; it is on Overview and on your phone.".into()
            } else {
                "Today's brief is ready as text (no audio this time).".into()
            });
        });
    };
    let expressive_label = move || {
        let e = tenths(&draft.expressiveness.get()).unwrap_or(14);
        let word = match e {
            ..=12 => "calm",
            13..=16 => "lively",
            _ => "very lively",
        };
        format!("{}.{} ({word})", e / 10, e % 10)
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>{move || persona().name().to_uppercase()}</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Daily voice brief"</CardTitle>
            <Help text="Once a day your coach talks you through what is left, what is close to its limit, the bills coming and whether your habits are changing, with one thing to do differently. Written and spoken by Mistral (needs the Mistral key under AI). Only totals, plan groups and bill names are sent; never accounts or merchants. Try the dials with Preview: nothing is saved or sent until you save." />
            {check(
                "Send the brief every day",
                "A push to every phone with notifications on; tap it to listen.",
                draft.enabled,
                busy,
            )}
            <div class=SETTINGS_GRID>
                <Field label="Who talks" id="brief-persona" hint="Both are laid-back Spaniards; the emails are signed by the same one.">
                    <Select
                        id="brief-persona"
                        value=persona_choice
                        options=Signal::derive(|| options(&[("gisel", "Gisel (woman from Spain)"), ("rafael", "Rafael (guy from Spain)")]))
                        disabled=Signal::derive(move || busy.get())
                    />
                </Field>
                <Field label="Hour" id="brief-hour" hint="Local time; a missed day is caught up once.">
                    <Input id="brief-hour" value=draft.hour kind="number" min="0" max="23" />
                </Field>
                <Field label="Voice" id="brief-voice" hint="Clone a voice in Mistral Studio → Voices, then pick it here.">
                    <Select
                        id="brief-voice"
                        value=draft.voice_id
                        options=voices
                        disabled=Signal::derive(move || busy.get() || !draft.voice.get())
                    />
                </Field>
                <Field label="Length" id="brief-length">
                    <Select
                        id="brief-length"
                        value=draft.length
                        options=Signal::derive(|| options(&[("short", "Short (about 45 seconds)"), ("normal", "Normal (about a minute)"), ("long", "Long (about a minute and a half)")]))
                        disabled=Signal::derive(move || busy.get())
                    />
                </Field>
                <Field label="Expressiveness" id="brief-expressive" hint="1.0 calm · 1.4 lively · 2.0 very lively">
                    <Input id="brief-expressive" value=draft.expressiveness kind="range" min="1.0" max="2.0" step="0.1" />
                    <span class=HINT>{expressive_label}</span>
                </Field>
                <Field label="Style note" id="brief-style" hint="Your own words on how they talk, up to 300 characters, no numbers. For example: more Spanish words, drier humour, shorter sentences.">
                    <Input id="brief-style" value=draft.style />
                </Field>
            </div>
            {check(
                "Speak it",
                "Off sends the text only.",
                draft.voice,
                busy,
            )}
            {check(
                "Match the voice's mood to the day",
                "Voices with mood versions (Paul, Oliver, Marie) switch: brighter when habits improve, lower when something is over its limit.",
                draft.match_mood,
                busy,
            )}
            {move || mood_note().map(|n| view! { <p class=cn([HINT, "mb-4"])>{n}</p> })}
            <div class=ACTIONS>
                <Button variant=Variant::Secondary disabled=Signal::derive(move || busy.get()) on_click=Callback::new(try_it)>
                    {move || if busy.get() { "Working…" } else { "Preview" }}
                </Button>
                <Button disabled=Signal::derive(move || busy.get()) on_click=Callback::new(save)>
                    "Save brief settings"
                </Button>
                <Button variant=Variant::Ghost disabled=Signal::derive(move || busy.get()) on_click=Callback::new(now)>
                    "Send today's brief now"
                </Button>
            </div>
            {move || preview.get().map(|p| {
                let audio = p.audio.map(|a| format!("data:audio/mpeg;base64,{a}"));
                let voice = if p.brief.voice_id.is_empty() {
                    "No audio".to_owned()
                } else {
                    format!("Voice: {}", p.brief.voice_id)
                };
                let fixed = if p.brief.source == "fixed" {
                    " · fixed words: Mistral was not available"
                } else {
                    ""
                };
                view! {
                    <div class="mt-4 grid gap-2 rounded-lg border border-border p-3">
                        <p class="text-sm font-semibold">{p.brief.title.clone()}</p>
                        {audio.map(|src| view! {
                            <audio class="w-full" controls=true autoplay=true src=src></audio>
                        })}
                        <p class=MUTED>{p.brief.script.clone()}</p>
                        <p class=HINT>{format!("{voice}{fixed}")}</p>
                    </div>
                }
            })}
            {move || latest.get().map(|b| view! {
                <p class=STATUS_NOTE>
                    {format!(
                        "Last brief: {} · {}{}",
                        b.date,
                        b.title,
                        if b.source == "fixed" { " (fixed words: Mistral was not available)" } else { "" }
                    )}
                </p>
            })}
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
