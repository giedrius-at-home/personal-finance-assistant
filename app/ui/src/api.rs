//! Bridge to the Tauri commands, plus a loader that keeps the previous result
//! on screen while a new one is fetched.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
export function in_tauri() { return !!(window.__TAURI__ && window.__TAURI__.core); }
export async function tauri_invoke(cmd, args) { return await window.__TAURI__.core.invoke(cmd, args); }
export async function dev_invoke(cmd, args) {
  const r = await fetch('/invoke/' + cmd, { method: 'POST', body: JSON.stringify(args) });
  const body = await r.json();
  if (!r.ok) throw body;
  return body;
}
// Disclosures the owner opened stay open when a reload re-renders them. A
// disclosure is known by its summary text and its position among those with
// the same text.
const openDetails = new Set();
function detailsKey(el) {
  const summary = el.querySelector(':scope > summary');
  const text = summary ? summary.textContent.trim() : '';
  const same = [...document.querySelectorAll('details')].filter((d) => {
    const s = d.querySelector(':scope > summary');
    return (s ? s.textContent.trim() : '') === text;
  });
  return text + '#' + same.indexOf(el);
}
export function keep_details_open() {
  document.addEventListener('toggle', (e) => {
    if (!(e.target instanceof HTMLDetailsElement)) return;
    const key = detailsKey(e.target);
    if (e.target.open) openDetails.add(key); else openDetails.delete(key);
  }, true);
  new MutationObserver(() => {
    if (!openDetails.size) return;
    for (const d of document.querySelectorAll('details:not([open])')) {
      if (openDetails.has(detailsKey(d))) d.open = true;
    }
  }).observe(document.body, { childList: true, subtree: true });
}
export function listen(event, callback) {
  if (window.__TAURI__ && window.__TAURI__.event) window.__TAURI__.event.listen(event, () => callback());
}
")]
extern "C" {
    fn in_tauri() -> bool;
    /// Keep open `<details>` open across re-renders (see the JS above).
    pub fn keep_details_open();
    /// Run `callback` whenever the backend emits `event` (no-op in a browser).
    pub fn listen(event: &str, callback: &Closure<dyn FnMut()>);
    #[wasm_bindgen(catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    /// Development only: the loopback `finance-devserver`, used when the UI is
    /// opened in a plain browser instead of the app window.
    #[wasm_bindgen(catch)]
    async fn dev_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

/// Running inside the app window (not the development browser).
pub fn in_app() -> bool {
    in_tauri()
}

/// Call a Tauri command. Errors are the short strings the backend returns.
pub async fn invoke<T: DeserializeOwned>(cmd: &str, args: &impl Serialize) -> Result<T, String> {
    let serializer = serde_wasm_bindgen::Serializer::json_compatible();
    let args = args.serialize(&serializer).map_err(|e| e.to_string())?;
    let result = if in_tauri() {
        tauri_invoke(cmd, args).await
    } else {
        dev_invoke(cmd, args).await
    };
    match result {
        Ok(value) => serde_wasm_bindgen::from_value(value).map_err(|e| e.to_string()),
        Err(err) => Err(err
            .as_string()
            .or_else(|| {
                err.dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "unknown-error".to_owned())),
    }
}

#[derive(Serialize)]
pub struct NoArgs {}

/// The latest result of a command whose arguments are reactive.
#[derive(Debug)]
pub struct Loaded<T: Send + Sync + 'static> {
    pub data: ReadSignal<Option<Result<T, String>>>,
    pub loading: ReadSignal<bool>,
}

impl<T: Send + Sync + 'static> Clone for Loaded<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Send + Sync + 'static> Copy for Loaded<T> {}

/// Re-run `cmd` whenever a signal read inside `args` changes. Results of
/// superseded calls are dropped, and the old data stays until the new arrives.
pub fn load<T, A>(cmd: &'static str, args: impl Fn() -> A + 'static) -> Loaded<T>
where
    T: DeserializeOwned + Send + Sync + 'static,
    A: Serialize + 'static,
{
    let (data, set_data) = signal(None);
    let (loading, set_loading) = signal(true);
    let generation = StoredValue::new(0_u64);
    Effect::new(move |_| {
        let args = args();
        generation.update_value(|g| *g += 1);
        let mine = generation.get_value();
        set_loading.set(true);
        spawn_local(async move {
            let result = invoke::<T>(cmd, &args).await;
            if generation.get_value() == mine {
                set_data.set(Some(result));
                set_loading.set(false);
            }
        });
    });
    Loaded { data, loading }
}
