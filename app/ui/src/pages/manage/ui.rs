//! What the Manage screens share (personal-site `components/manage-ui.ts` and
//! the workspace loading of `owner-operations.tsx`).

use finance_types::manage::ManageWorkspace;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::api::{Loaded, NoArgs, invoke, load};
use crate::ds::patterns::SECTION_STACK;
use crate::ds::{Button, Card};
use crate::state;

pub use crate::ds::patterns::{ERROR, FIELDSET};

pub const FEEDBACK: &str = "rounded-lg bg-positive/10 px-4 py-3 text-sm text-fg";
pub const PREVIEW: &str = "grid gap-4 rounded-lg border border-border bg-card-muted p-4 sm:p-5";
pub const CONTROLS: &str =
    "flex flex-col items-stretch gap-4 sm:flex-row sm:flex-wrap sm:items-end";
pub const FILE_INPUT: &str = "py-2 file:mr-3 file:border-0 file:bg-transparent file:p-0 file:text-sm file:font-medium file:text-fg";
pub const DETAILS: &str = "border-t border-line py-4";
pub const SUMMARY: &str = "text-sm font-medium";

/// The site's `date()`: `toLocaleString("en-GB")`, or "Not recorded".
pub fn local_date(value: Option<&str>) -> String {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return "Not recorded".into();
    };
    // WebKit only parses millisecond fractions; Python wrote microseconds.
    let mut text = String::with_capacity(value.len());
    let mut fraction = None::<usize>;
    for c in value.chars() {
        match fraction {
            Some(n) if c.is_ascii_digit() => {
                if n < 3 {
                    text.push(c);
                }
                fraction = Some(n + 1);
            }
            _ => {
                fraction = (c == '.').then_some(0);
                text.push(c);
            }
        }
    }
    let date = js_sys::Date::new(&JsValue::from_str(&text));
    if date.get_time().is_nan() {
        return "Not recorded".into();
    }
    date.to_locale_string("en-GB", &JsValue::UNDEFINED).into()
}

/// One screen's busy flag and its error and notice lines.
#[derive(Clone, Copy, Debug)]
pub struct Ops {
    pub busy: RwSignal<bool>,
    pub error: RwSignal<String>,
    pub notice: RwSignal<String>,
}

impl Ops {
    pub fn new() -> Self {
        Self {
            busy: RwSignal::new(false),
            error: RwSignal::new(String::new()),
            notice: RwSignal::new(String::new()),
        }
    }

    /// Mark busy and clear the messages; `false` while another operation runs.
    pub fn begin(self) -> bool {
        if self.busy.get_untracked() {
            return false;
        }
        self.busy.set(true);
        self.error.set(String::new());
        self.notice.set(String::new());
        true
    }

    /// Run `cmd`; `done` gets its result, a failure shows as the error line.
    pub fn run<T, A>(self, cmd: &'static str, args: A, done: impl FnOnce(T) + 'static)
    where
        T: DeserializeOwned + 'static,
        A: Serialize + 'static,
    {
        if !self.begin() {
            return;
        }
        spawn_local(async move {
            match invoke::<T>(cmd, &args).await {
                Ok(value) => done(value),
                Err(e) => self.error.set(e),
            }
            self.busy.set(false);
        });
    }
}

#[component]
pub fn Messages(ops: Ops) -> impl IntoView {
    view! {
        {move || {
            let e = ops.error.get();
            (!e.is_empty()).then(|| view! { <p role="alert" class=ERROR>{e}</p> })
        }}
        {move || {
            let n = ops.notice.get();
            (!n.is_empty()).then(|| view! { <p role="status" class=FEEDBACK>{n}</p> })
        }}
    }
}

/// The workspace, reloaded after every write and on `retry`; provided by
/// context for the section's panels.
pub fn load_workspace(retry: RwSignal<u32>) -> Loaded<ManageWorkspace> {
    let app = state();
    let workspace = load("manage_workspace", move || {
        app.revision.track();
        retry.track();
        NoArgs {}
    });
    provide_context(workspace);
    workspace
}

/// A panel's `shown`: the section's workspace has loaded.
pub fn workspace_loaded() -> bool {
    use_context::<Loaded<ManageWorkspace>>()
        .is_some_and(|w| w.data.with(|d| matches!(d, Some(Ok(_)))))
}

/// `f` with the loaded workspace, drawn again after every reload.
pub fn with_workspace<V: IntoView + 'static>(
    f: impl Fn(ManageWorkspace) -> V + Send + Sync + 'static,
) -> AnyView {
    let workspace = expect_context::<Loaded<ManageWorkspace>>();
    (move || workspace.data.get().and_then(Result::ok).map(&f)).into_any()
}

/// The owner-operations frame: messages, the loading / retry card until the
/// workspace is loaded, then the section's panel `list` (whose workspace
/// panels wait for it through `workspace_loaded`).
pub fn frame(
    ops: Ops,
    workspace: Loaded<ManageWorkspace>,
    retry: RwSignal<u32>,
    list: impl IntoView + 'static,
) -> impl IntoView {
    Effect::new(move |_| {
        if let Some(Err(e)) = workspace.data.get() {
            ops.error.set(e);
        }
    });
    view! {
        <div class=SECTION_STACK>
            <Messages ops=ops />
            {move || match workspace.data.get() {
                Some(Ok(_)) => None,
                other => {
                    let failed = matches!(other, Some(Err(_)));
                    Some(view! {
                        <Card class="grid justify-items-start gap-3">
                            <p class="text-sm">
                                {if failed { "Owner workspace could not be loaded." } else { "Loading owner workspace…" }}
                            </p>
                            {failed
                                .then(|| {
                                    view! {
                                        <Button on_click=Callback::new(move |_| {
                                            ops.error.set(String::new());
                                            retry.update(|n| *n += 1);
                                        })>"Retry connection"</Button>
                                    }
                                })}
                        </Card>
                    })
                }
            }}
            {list}
        </div>
    }
}
