//! Today's voice brief as a one-time pop-up over whatever page is open.
//!
//! It opens by itself the first time the app is opened on the brief's day,
//! and always when the phone's push is tapped (`/?brief=play`). Once it has
//! been played to the end or closed, this device does not show it again.
//! A browser plays sound only after a tap, so the pop-up tries to start on
//! its own and otherwise waits on its big Play button.

use finance_types::notify::{Brief, plain_script};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::api::{NoArgs, load};
use crate::ds::Popup;
use crate::ds::patterns::{HINT, MUTED};

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
// Opened from the push: true once, then the address is cleaned so a reload
// does not open it again.
export function brief_from_push() {
  const params = new URLSearchParams(location.search);
  if (params.get('brief') !== 'play') return false;
  params.delete('brief');
  const rest = params.toString();
  history.replaceState(null, '', location.pathname + (rest ? '?' + rest : '') + location.hash);
  return true;
}
export function brief_today() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate());
}
export function brief_heard() {
  try { return localStorage.getItem('brief-heard') || ''; } catch (_) { return ''; }
}
export function brief_mark_heard(date) {
  try { localStorage.setItem('brief-heard', date); } catch (_) {}
}
// Start the player; false when the browser wants a tap first.
export function brief_start(id) {
  return new Promise((r) => setTimeout(r, 50)).then(() => {
    const el = document.getElementById(id);
    if (!el) return false;
    return el.play().then(() => true, () => false);
  });
}
export function brief_toggle(id) {
  const el = document.getElementById(id);
  if (!el) return;
  if (el.paused) el.play().catch(() => {}); else el.pause();
}
")]
extern "C" {
    fn brief_audio_url(date: &str) -> js_sys::Promise;
    fn brief_from_push() -> bool;
    fn brief_today() -> String;
    fn brief_heard() -> String;
    fn brief_mark_heard(date: &str);
    fn brief_start(id: &str) -> js_sys::Promise;
    fn brief_toggle(id: &str);
}

const PLAYER: &str = "brief-popup-player";

/// Whether the pop-up opens: always from the push; otherwise only for
/// today's brief this device has not heard yet.
pub fn should_open(from_push: bool, brief_date: &str, today: &str, heard: &str) -> bool {
    from_push || (brief_date == today && heard != brief_date)
}

#[component]
pub fn BriefPopup() -> impl IntoView {
    // The Mac app has no brief: it talks to the server only through the web.
    if crate::api::in_app() {
        return ().into_any();
    }
    let from_push = brief_from_push();
    let brief = load::<Option<Brief>, _>("latest_brief", || NoArgs {});
    let open = RwSignal::new(None::<Brief>);
    let src = RwSignal::new(None::<String>);
    let playing = RwSignal::new(false);
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        let Some(Ok(Some(b))) = brief.data.get() else {
            return false;
        };
        if !should_open(from_push, &b.date, &brief_today(), &brief_heard()) {
            return true;
        }
        open.set(Some(b.clone()));
        if b.has_audio {
            spawn_local(async move {
                if let Ok(url) = JsFuture::from(brief_audio_url(&b.date)).await
                    && let Some(url) = url.as_string()
                {
                    src.set(Some(url));
                    // Once the player has mounted, try to start without a tap.
                    if let Ok(started) = JsFuture::from(brief_start(PLAYER)).await {
                        playing.set(started.as_bool() == Some(true));
                    }
                }
            });
        }
        true
    });
    let close = Callback::new(move |()| {
        if let Some(b) = open.get_untracked() {
            brief_mark_heard(&b.date);
        }
        open.set(None);
    });
    (move || {
        open.get().map(|b| {
            let who = if b.persona.is_empty() {
                "your coach".to_owned()
            } else {
                b.persona.clone()
            };
            let words = plain_script(&b.script);
            let label = who.clone();
            view! {
                <Popup title=format!("{who}: {}", b.title) description=format!("{} {}", b.facts.weekday, b.date) on_close=close>
                    {move || {
                        let label = label.clone();
                        src.get().map(|url| view! {
                            <button
                                type="button"
                                class="flex w-full items-center justify-center gap-2 rounded-xl bg-contrast px-5 py-4 text-lg font-semibold text-fg-on-contrast hover:bg-contrast/90"
                                on:click=move |_| brief_toggle(PLAYER)
                            >
                                {move || if playing.get() { "❚❚ Pause".to_owned() } else { format!("▶ Play {label}") }}
                            </button>
                            <audio
                                id=PLAYER
                                class="w-full"
                                controls=true
                                preload="auto"
                                src=url
                                on:play=move |_| playing.set(true)
                                on:pause=move |_| playing.set(false)
                                on:ended=move |_| {
                                    playing.set(false);
                                    if let Some(b) = open.get_untracked() {
                                        brief_mark_heard(&b.date);
                                    }
                                }
                            ></audio>
                        })
                    }}
                    {(!b.has_audio).then(|| view! { <p class=HINT>"No audio today; here is what was said."</p> })}
                    <details class="text-sm" open=!b.has_audio>
                        <summary class=HINT>"Read it"</summary>
                        <p class=format!("mt-2 {MUTED}")>{words}</p>
                    </details>
                </Popup>
            }
        })
    })
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::should_open;

    #[test]
    fn opens_once_for_today_and_always_from_the_push() {
        assert!(should_open(false, "2026-11-26", "2026-11-26", ""));
        assert!(!should_open(
            false,
            "2026-11-26",
            "2026-11-26",
            "2026-11-26"
        ));
        assert!(!should_open(false, "2026-11-25", "2026-11-26", ""));
        assert!(should_open(true, "2026-11-25", "2026-11-26", "2026-11-25"));
    }
}
