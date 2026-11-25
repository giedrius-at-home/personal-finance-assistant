//! Overview → Today's brief: the latest voice brief from Gisel or Rafael,
//! with a big Play button, the player and the words. The phone's push opens
//! `/?brief=play`: the page scrolls here and starts playing when the browser
//! allows it (an installed iPhone app needs the tap on Play).

use finance_types::notify::Brief;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::api::{NoArgs, load};
use crate::ds::patterns::{HINT, MUTED, STATUS_NOTE};
use crate::ds::{Card, CardTitle, Eyebrow, Heading, cn};

#[wasm_bindgen(inline_js = "
// The MP3 as a blob URL: Safari's player wants byte ranges, which the
// server does not serve; a fetched blob plays everywhere.
export function brief_audio_url(date) {
  return (async () => {
    const r = await fetch('/brief/audio/' + date + '.mp3', { credentials: 'same-origin' });
    if (!r.ok) throw 'no audio';
    return URL.createObjectURL(await r.blob());
  })();
}
export function brief_wants_play() {
  return new URLSearchParams(location.search).get('brief') === 'play';
}
export function brief_play(id) {
  const el = document.getElementById(id);
  if (el) el.play().catch(() => {});
}
export function brief_scroll(id) {
  const el = document.getElementById(id);
  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
}
")]
extern "C" {
    fn brief_audio_url(date: &str) -> js_sys::Promise;
    fn brief_wants_play() -> bool;
    fn brief_play(id: &str);
    fn brief_scroll(id: &str);
}

const PLAYER: &str = "brief-player";
const CARD: &str = "brief-card";

#[component]
pub fn OverviewBrief() -> impl IntoView {
    let brief = load::<Option<Brief>, _>("latest_brief", || NoArgs {});
    let src = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let Some(Ok(Some(b))) = brief.data.get() else {
            return;
        };
        if brief_wants_play() {
            brief_scroll(CARD);
        }
        if !b.has_audio || crate::api::in_app() {
            return;
        }
        spawn_local(async move {
            if let Ok(url) = JsFuture::from(brief_audio_url(&b.date)).await
                && let Some(url) = url.as_string()
            {
                src.set(Some(url));
                if brief_wants_play() {
                    brief_play(PLAYER);
                }
            }
        });
    });
    let body = move |b: Brief| {
        let habits = b
            .facts
            .habits
            .iter()
            .chain(b.facts.avoidable.iter())
            .map(|h| {
                let arrow = match h.direction.as_str() {
                    "up" => "↑",
                    "down" => "↓",
                    _ => "→",
                };
                format!(
                    "{arrow} {}: {} vs {} usual",
                    h.group, h.last_week, h.usual_week
                )
            })
            .collect::<Vec<_>>();
        let who = if b.persona.is_empty() {
            "your coach".to_owned()
        } else {
            b.persona.clone()
        };
        let no_audio = (!b.has_audio)
            .then(|| view! { <p class=format!("mt-3 {HINT}")>"No audio today; here is what was said."</p> });
        view! {
            <p class="mt-2 text-lg font-semibold">{b.title.clone()}</p>
            <p class=cn([HINT, "flex gap-x-3"])>
                <span>{b.facts.weekday.clone()}</span>
                <span>{b.date.clone()}</span>
            </p>
            {move || {
                let who = who.clone();
                src.get().map(|url| view! {
                    <button
                        type="button"
                        class="mt-4 flex w-full items-center justify-center gap-2 rounded-xl bg-contrast px-5 py-4 text-lg font-semibold text-fg-on-contrast hover:bg-contrast/90"
                        on:click=move |_| brief_play(PLAYER)
                    >
                        {format!("▶ Play {who}")}
                    </button>
                    <audio id=PLAYER class="mt-3 w-full" controls=true preload="auto" src=url></audio>
                })
            }}
            {no_audio}
            <p class=format!("mt-3 {MUTED}")>{b.script.clone()}</p>
            {(!habits.is_empty()).then(|| view! {
                <ul class="mt-3 grid gap-1">
                    {habits.into_iter().map(|h| view! { <li class=HINT>{h}</li> }).collect_view()}
                </ul>
            })}
        }
    };
    view! {
        <div id=CARD>
        <Card>
            <Eyebrow level=Heading::P>"BRIEF"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Today's brief"</CardTitle>
            {move || match brief.data.get() {
                None => view! { <p class=STATUS_NOTE>"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class=STATUS_NOTE>{format!("The brief is not available ({e}).")}</p> }.into_any(),
                Some(Ok(None)) => view! {
                    <p class=STATUS_NOTE>"No brief yet. Turn on the daily voice brief in Manage → Settings."</p>
                }.into_any(),
                Some(Ok(Some(b))) => body(b).into_any(),
            }}
        </Card>
        </div>
    }
}
