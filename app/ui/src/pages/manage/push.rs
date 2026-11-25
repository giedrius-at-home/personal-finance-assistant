//! Manage → Settings → Phone notifications: Web Push from the server to the
//! web app installed on the phone (budget alerts and the evening overspend
//! summary). The browser's side is the service worker (`sw.js`) and the
//! JS below; the server keeps the subscriptions and signs every push.

use finance_types::notify::{PushStatus, PushSubscription};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::ui::{Messages, Ops};
use crate::api::{NoArgs, invoke, load};
use crate::ds::patterns::{ACTION_BAR, HINT, LIST_STACK, MUTED, STATUS_NOTE, SUBPANEL};
use crate::ds::{Button, Card, CardTitle, Eyebrow, Heading, Size, Variant};

#[wasm_bindgen(inline_js = "
export function push_support() {
  if (window.__TAURI__) return 'app';
  const ios = /iPhone|iPad|iPod/.test(navigator.userAgent);
  if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) {
    return ios && !navigator.standalone ? 'install' : 'none';
  }
  return Notification.permission === 'denied' ? 'denied' : 'ok';
}
export function push_current() {
  return (async () => {
    if (push_support() !== 'ok') return '';
    const r = await navigator.serviceWorker.getRegistration('/');
    const s = r && await r.pushManager.getSubscription();
    return s ? s.endpoint : '';
  })();
}
// Called straight from the tap: Safari asks for permission only then.
export function push_enable(key) {
  const asked = Notification.requestPermission();
  return (async () => {
    if (await asked !== 'granted') throw 'Notifications were not allowed on this device.';
    const r = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
    await navigator.serviceWorker.ready;
    const b64 = key.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - key.length % 4) % 4);
    const applicationServerKey = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    let s = await r.pushManager.getSubscription();
    if (!s) s = await r.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey });
    const j = s.toJSON();
    return JSON.stringify({ endpoint: j.endpoint, p256dh: j.keys.p256dh, auth: j.keys.auth, origin: location.origin });
  })();
}
export function push_disable() {
  return (async () => {
    const r = await navigator.serviceWorker.getRegistration('/');
    const s = r && await r.pushManager.getSubscription();
    if (!s) return '';
    const endpoint = s.endpoint;
    await s.unsubscribe();
    return endpoint;
  })();
}
")]
extern "C" {
    /// `ok`, `app` (the Mac window), `install` (iPhone Safari before Add to
    /// Home Screen), `denied` or `none`.
    fn push_support() -> String;
    /// This device's push endpoint, or empty.
    fn push_current() -> js_sys::Promise;
    /// Ask permission, subscribe with the server key; JSON subscription.
    fn push_enable(key: &str) -> js_sys::Promise;
    /// Unsubscribe; the endpoint it had, or empty.
    fn push_disable() -> js_sys::Promise;
}

async fn text(promise: js_sys::Promise) -> Result<String, String> {
    match JsFuture::from(promise).await {
        Ok(v) => Ok(v.as_string().unwrap_or_default()),
        Err(e) => Err(e
            .as_string()
            .or_else(|| {
                e.dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "The browser refused.".to_owned())),
    }
}

#[derive(Serialize)]
struct SubscribeArgs {
    subscription: PushSubscription,
}

#[derive(Serialize)]
struct EndpointArgs {
    endpoint: String,
}

#[component]
pub fn PushCard() -> impl IntoView {
    let ops = Ops::new();
    let busy = ops.busy;
    let changed = RwSignal::new(0_u32);
    let status = load::<PushStatus, _>("push_status", move || {
        changed.track();
        NoArgs {}
    });
    let support = push_support();
    let in_mac_app = support == "app";
    // This device's endpoint, once the browser said.
    let here = RwSignal::new(String::new());
    Effect::new(move |_| {
        changed.track();
        spawn_local(async move {
            here.set(text(push_current()).await.unwrap_or_default());
        });
    });
    let enable = move |key: String| {
        if !ops.begin() {
            return;
        }
        let asked = push_enable(&key);
        spawn_local(async move {
            let result = async {
                let json = text(asked).await?;
                let subscription: PushSubscription =
                    serde_json::from_str(&json).map_err(|e| e.to_string())?;
                invoke::<PushStatus>("push_subscribe", &SubscribeArgs { subscription }).await
            }
            .await;
            match result {
                Ok(_) => ops
                    .notice
                    .set("Notifications are on for this device.".into()),
                Err(e) => ops.error.set(e),
            }
            ops.busy.set(false);
            changed.update(|n| *n += 1);
        });
    };
    let disable = move |_| {
        if !ops.begin() {
            return;
        }
        let gone = push_disable();
        spawn_local(async move {
            let result = async {
                let endpoint = text(gone).await?;
                if !endpoint.is_empty() {
                    invoke::<PushStatus>("push_unsubscribe", &EndpointArgs { endpoint }).await?;
                }
                Ok::<(), String>(())
            }
            .await;
            match result {
                Ok(()) => ops
                    .notice
                    .set("Notifications are off for this device.".into()),
                Err(e) => ops.error.set(e),
            }
            ops.busy.set(false);
            changed.update(|n| *n += 1);
        });
    };
    let test = move |_| {
        ops.run("push_test", NoArgs {}, move |message: String| {
            ops.notice.set(message);
        });
    };
    let body = move |s: PushStatus| {
        let devices = s.devices.len();
        let facts = if devices == 0 {
            "No device receives notifications yet.".to_owned()
        } else {
            format!(
                "{devices} device{} receive{} notifications.",
                if devices == 1 { "" } else { "s" },
                if devices == 1 { "s" } else { "" }
            )
        };
        let Some(key) = s.public_key else {
            return view! { <p class=STATUS_NOTE>"Push is not set up on this server."</p> }
                .into_any();
        };
        let control = match support.as_str() {
            "ok" => {
                let key = key.clone();
                view! {
                    <div class=ACTION_BAR>
                        {move || {
                            if here.get().is_empty() {
                                let key = key.clone();
                                view! {
                                    <Button
                                        size=Size::Sm
                                        disabled=Signal::derive(move || busy.get())
                                        on_click=Callback::new(move |_| enable(key.clone()))
                                    >
                                        "Turn on for this device"
                                    </Button>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <Button
                                        size=Size::Sm
                                        variant=Variant::Secondary
                                        disabled=Signal::derive(move || busy.get())
                                        on_click=Callback::new(disable)
                                    >
                                        "Turn off for this device"
                                    </Button>
                                }
                                    .into_any()
                            }
                        }}
                        <Button
                            size=Size::Sm
                            variant=Variant::Ghost
                            disabled=Signal::derive(move || busy.get() || devices == 0)
                            on_click=Callback::new(test)
                        >
                            "Send a test"
                        </Button>
                    </div>
                }
                    .into_any()
            }
            "install" => view! {
                <p class=MUTED>
                    "On iPhone, notifications need the installed app: Share → Add to Home Screen, then open Finance from the Home Screen and come back here."
                </p>
            }
                .into_any(),
            "denied" => view! {
                <p class=MUTED>
                    "Notifications are blocked for Finance on this device. Allow them in the phone's settings, then come back here."
                </p>
            }
                .into_any(),
            _ => view! { <p class=MUTED>"This browser cannot receive push notifications."</p> }
                .into_any(),
        };
        view! {
            <div class=SUBPANEL>
                <p class=MUTED>
                    "Budget alerts (a plan line at 80% or over) arrive at once; on a day that went over, the evening summary says what pushed it over and which purchases could have been skipped."
                </p>
                <p class=HINT>{facts}</p>
                {control}
            </div>
        }
            .into_any()
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"PHONE"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Phone notifications"</CardTitle>
            <div class=LIST_STACK>
                <Messages ops=ops />
            </div>
            {move || {
                if in_mac_app {
                    return view! {
                        <p class=STATUS_NOTE>
                            "The Mac shows alerts as notifications. Turn on phone notifications from Finance on the phone."
                        </p>
                    }
                        .into_any();
                }
                match status.data.get() {
                    None => view! { <p class=STATUS_NOTE>"Loading…"</p> }.into_any(),
                    Some(Err(e)) => {
                        view! { <p class=STATUS_NOTE>{format!("Notifications are not available ({e}).")}</p> }
                            .into_any()
                    }
                    Some(Ok(s)) => body(s).into_any(),
                }
            }}
        </Card>
    }
}
