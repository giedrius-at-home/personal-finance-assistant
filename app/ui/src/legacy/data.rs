use finance_types::{DataStatus, UpdateStatus};
use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::api::{NoArgs, invoke, load};
use crate::format;
use crate::state;
use crate::widgets::{Problem, Skeleton};

#[component]
pub fn Data() -> impl IntoView {
    let status = load::<DataStatus, _>("status", || NoArgs {});
    view! {
        {move || match status.data.get() {
            None => view! { <Skeleton rows=3 /> }.into_any(),
            Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
            Some(Ok(s)) => view! { <LocalData s=s /> }.into_any(),
        }}
        <Updates />
    }
}

#[component]
fn LocalData(s: DataStatus) -> impl IntoView {
    view! {
        <section class="card panel">
            <div class="panel-head">
                <h2>"Local data"</h2>
            </div>
            <dl class="facts wide">
                <dt>"Stored at"</dt>
                <dd>
                    <code>"~/Library/Application Support/Finance/finance.sqlite3"</code>
                </dd>
                <dt>"Transactions"</dt>
                <dd>{format::count(s.transactions)}</dd>
                <dt>"Covers"</dt>
                <dd>
                    {match (s.first_date, s.last_date) {
                        (Some(a), Some(b)) => format!("{} – {}", format::date(&a), format::date(&b)),
                        _ => "—".to_owned(),
                    }}
                </dd>
                <dt>"Imported"</dt>
                <dd>
                    {s
                        .imported_at
                        .map(|t| format!("{} {} UTC", format::date(&t), t.get(11..16).unwrap_or("")))
                        .unwrap_or_default()}
                </dd>
                <dt>"Source"</dt>
                <dd>{s.source.unwrap_or_default()}</dd>
            </dl>
            <p class="muted pad">
                "Everything stays on this Mac. Bank sync and archiving to Google Drive arrive in later releases."
            </p>
        </section>
    }
}

#[derive(Serialize)]
struct TokenArgs {
    token: String,
}

/// Run a command, then ask for fresh update state; show its error if any.
fn act(
    cmd: &'static str,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    args: impl Serialize + 'static,
) {
    let app = state();
    busy.set(true);
    error.set(None);
    spawn_local(async move {
        if let Err(e) = invoke::<()>(cmd, &args).await {
            error.set(Some(e));
        }
        busy.set(false);
        app.updates_changed.update(|n| *n += 1);
    });
}

#[component]
fn Updates() -> impl IntoView {
    let app = state();
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let token_input = NodeRef::<Input>::new();
    let save_token = move || {
        let Some(el) = token_input.get() else { return };
        let token = el.value();
        el.set_value("");
        act("set_release_token", error, busy, TokenArgs { token });
    };

    let body = move |u: UpdateStatus| {
        let status_line = if u.installing {
            "Installing… the app restarts when done.".to_owned()
        } else if u.checking {
            "Checking for updates…".to_owned()
        } else if let Some(e) = &u.error {
            format!("Last check failed: {e}")
        } else if u.available.is_some() {
            "An update is ready.".to_owned()
        } else if let Some(t) = &u.last_checked {
            format!("Up to date · checked {t}")
        } else {
            "Not checked yet.".to_owned()
        };
        view! {
            <dl class="facts wide">
                <dt>"Installed version"</dt>
                <dd>{u.current_version.clone()}</dd>
                <dt>"Status"</dt>
                <dd class:negative-text=u.error.is_some()>{status_line}</dd>
            </dl>
            {u
                .available
                .clone()
                .map(|a| {
                    view! {
                        <div class="update-box">
                            <div>
                                <strong>{format!("Version {} is available", a.version)}</strong>
                                {a.date.map(|d| view! { <span class="muted">{format!(" · {}", format::date(&d))}</span> })}
                                {a.notes.filter(|n| !n.trim().is_empty()).map(|n| view! { <p class="notes">{n}</p> })}
                            </div>
                            <button
                                class="button primary"
                                disabled=move || busy.get() || u.installing
                                on:click=move |_| act("install_update", error, busy, NoArgs {})
                            >
                                "Install and restart"
                            </button>
                        </div>
                    }
                })}
            {if u.token_set {
                view! {
                    <div class="row-actions">
                        <button
                            class="button"
                            disabled=move || busy.get() || u.checking
                            on:click=move |_| act("check_for_updates", error, busy, NoArgs {})
                        >
                            "Check now"
                        </button>
                        <button class="link" on:click=move |_| act("forget_release_token", error, busy, NoArgs {})>
                            "Remove release token"
                        </button>
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="token-setup">
                        <p class="muted">
                            "Updates come from this app's GitHub releases. Paste a fine-grained GitHub token with read-only "
                            <em>"Contents"</em>
                            " access to the personal-finance-assistant repository. It is kept in your Keychain."
                        </p>
                        <form
                            class="row-actions"
                            on:submit=move |e| {
                                e.prevent_default();
                                save_token();
                            }
                        >
                            <input
                                node_ref=token_input
                                class="text-input"
                                type="password"
                                autocomplete="off"
                                placeholder="github_pat_…"
                                aria-label="GitHub release token"
                            />
                            <button class="button primary" type="submit" disabled=move || busy.get()>
                                "Save and check"
                            </button>
                        </form>
                    </div>
                }
                    .into_any()
            }}
        }
    };

    view! {
        <section class="card panel">
            <div class="panel-head">
                <h2>"Updates"</h2>
            </div>
            {move || match app.updates.data.get() {
                None => view! { <Skeleton rows=1 /> }.into_any(),
                Some(Err(e)) => view! { <p class="muted">{format!("Updates are not available here ({e}).")}</p> }.into_any(),
                Some(Ok(u)) => body(u).into_any(),
            }}
            {move || error.get().map(|e| view! { <p class="negative-text">{e}</p> })}
        </section>
    }
}
