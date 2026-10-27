//! Manage → Banks: the site's bank control (`owner-banks.tsx`) over the
//! app's own Enable Banking sync: per-bank status and sync, consent renewal,
//! plus the application key (the site kept it in the importer container) and
//! each bank's ledger accounts.

use std::collections::BTreeMap;
use std::time::Duration;

use finance_types::bank::{
    BANKS, BankConsent, BankCredentials, BankState, BankSyncReport, BankSyncResult, SyncStatus,
    bank_name,
};
use finance_types::manage::{BankAccount, BanksReport};
use finance_types::report::Currency;
use leptos::html::Input as InputElement;
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::ui::{ERROR, FEEDBACK, FILE_INPUT, Messages, Ops, local_date};
use crate::api::{NoArgs, invoke, load};
use crate::ds::{
    Badge, Button, CONTROL, Card, CardDescription, CardHeader, CardTitle, Eyebrow, Field, Heading,
    Input, Select, Size, TABLE, TABLE_CONTAINER, TCELL, TCELL_NUM, TH, TH_NUM, TROW, Tone, Variant,
    cn, money,
};
use crate::pages::closure::bank_sync::SyncArgs;
use crate::{format, state};

#[wasm_bindgen(inline_js = "
export async function read_small_text(input, limit, what) {
  const file = input.files && input.files[0];
  if (!file) throw 'Choose a ' + what + ' file.';
  if (file.size > limit) throw 'The ' + what + ' file is too large.';
  return await file.text();
}
")]
extern "C" {
    /// The chosen file's text, at most `limit` bytes.
    #[wasm_bindgen(catch)]
    async fn read_small_text(
        input: &web_sys::HtmlInputElement,
        limit: u32,
        what: &str,
    ) -> Result<JsValue, JsValue>;
}

/// Read the file chosen in `input` as text.
async fn file_text(input: NodeRef<InputElement>, limit: u32, what: &str) -> Result<String, String> {
    let input = input.get_untracked().ok_or("The file could not be read.")?;
    read_small_text(&input, limit, what)
        .await
        .map_err(|e| {
            e.as_string()
                .unwrap_or_else(|| "The file could not be read.".into())
        })?
        .as_string()
        .ok_or_else(|| "The file could not be read.".into())
}

/// The current local time, as the site's `toLocaleString()`.
fn now_label() -> String {
    js_sys::Date::new_0()
        .to_locale_string("en-GB", &JsValue::UNDEFINED)
        .into()
}

/// `statusLabels` of the site: the headline and what to do.
fn status_label(status: SyncStatus) -> (&'static str, &'static str) {
    match status {
        SyncStatus::Success => (
            "Last sync succeeded",
            "The last import completed successfully.",
        ),
        SyncStatus::Verified => ("Connected", "Ready to sync."),
        SyncStatus::WaitingForCompletedDay => (
            "Up to date for completed days",
            "Waiting for the next completed bank day.",
        ),
        SyncStatus::ReauthorizationRequired => ("Reconnect required", "Renew bank consent below."),
        SyncStatus::RateLimited => ("Waiting to retry", "Try again later."),
        SyncStatus::UpstreamUnavailable => ("Bank unavailable", "Try again later."),
        SyncStatus::RuntimeUnavailable => ("Sync unavailable", "Try again later."),
        SyncStatus::MissingConfig => ("Reconnect required", "Renew the connection below."),
        SyncStatus::MappingMismatch => (
            "Reconnect required",
            "Renew the connection below. Existing accounts must stay mapped; new ones may be added.",
        ),
        SyncStatus::InvalidMapping | SyncStatus::InvalidConfiguration => {
            ("Reconnect required", "Renew the connection below.")
        }
        SyncStatus::MissingCredentials => (
            "Application key missing",
            "Add the Enable Banking application key below.",
        ),
        SyncStatus::ProviderPaginationError => (
            "Last sync failed",
            "The bank returned inconsistent pages. Try again later.",
        ),
        SyncStatus::ImportFailed => (
            "Import failed",
            "The connection ran, but transaction import did not complete.",
        ),
        SyncStatus::Failed => (
            "Last sync failed",
            "The last operation did not complete successfully.",
        ),
    }
}

#[derive(Serialize)]
struct ConsentArgs {
    consent: BankConsent,
}

#[derive(Serialize)]
struct CredentialArgs {
    credentials: BankCredentials,
}

fn outcome(result: &BankSyncResult) -> Result<String, String> {
    let name = bank_name(&result.bank);
    if result.status.is_ok() {
        Ok(match result.created {
            Some(n) => format!("{name}: Completed — {n} new transactions."),
            None => format!("{name}: {}.", status_label(result.status).0),
        })
    } else {
        Err(format!("{name}: {}.", result.status.reason()))
    }
}

#[component]
pub fn Banks() -> impl IntoView {
    let app = state();
    let ops = Ops::new();
    let poll = RwSignal::new(0_u32);
    let working = RwSignal::new(None::<String>);
    let status = load::<BankSyncReport, _>("bank_sync_status", move || {
        app.revision.track();
        poll.track();
        NoArgs {}
    });
    let accounts = load::<BanksReport, _>("banks", move || {
        app.revision.track();
        NoArgs {}
    });
    // The scheduler may sync at any time; keep the screen current.
    if let Ok(handle) = set_interval_with_handle(
        move || {
            if !document().hidden() {
                poll.update(|n| *n += 1);
            }
        },
        Duration::from_secs(10),
    ) {
        on_cleanup(move || handle.clear());
    }
    let report = move || status.data.get().and_then(Result::ok);
    let running = move || working.get().or_else(|| report().and_then(|r| r.running));
    let busy = Signal::derive(move || ops.busy.get() || running().is_some() || report().is_none());

    let sync = move |bank: String| {
        if !ops.begin() {
            return;
        }
        working.set(Some(bank.clone()));
        spawn_local(async move {
            match invoke::<BankSyncResult>("sync_bank", &SyncArgs { bank: &bank }).await {
                Ok(result) => match outcome(&result) {
                    Ok(notice) => ops.notice.set(notice),
                    Err(error) => ops.error.set(error),
                },
                Err(e) => ops.error.set(e),
            }
            working.set(None);
            ops.busy.set(false);
            app.revision.update(|n| *n += 1);
        });
    };
    let sync_all = move |_| {
        let Some(current) = report() else {
            return;
        };
        if !ops.begin() {
            return;
        }
        spawn_local(async move {
            let mut failures = Vec::new();
            for (id, name) in BANKS {
                if current
                    .banks
                    .iter()
                    .any(|b| b.id == id && b.retry_after.is_some())
                {
                    failures.push(format!("{name}: rate limited"));
                    continue;
                }
                working.set(Some(id.to_owned()));
                ops.notice.set(format!("Syncing {name}…"));
                match invoke::<BankSyncResult>("sync_bank", &SyncArgs { bank: id }).await {
                    Ok(result) if result.status.is_ok() => {}
                    Ok(result) => {
                        failures.push(format!("{name}: {}", result.status.reason().to_lowercase()))
                    }
                    Err(e) => failures.push(format!("{name}: {e}")),
                }
                poll.update(|n| *n += 1);
            }
            working.set(None);
            if failures.is_empty() {
                ops.notice.set("All banks synced.".into());
            } else {
                ops.notice.set(String::new());
                ops.error
                    .set(format!("Not synced — {}.", failures.join("; ")));
            }
            ops.busy.set(false);
            app.revision.update(|n| *n += 1);
        });
    };

    view! {
        <div class="mb-6 grid gap-4">
            <Messages ops=ops />
            {move || match status.data.get() {
                Some(Err(e)) => Some(view! { <p role="alert" class=ERROR>{e}</p> }),
                _ => None,
            }}
            <Card>
                <CardHeader>
                    <div>
                        <Eyebrow level=Heading::P>"THIS MAC"</Eyebrow>
                        <CardTitle level=Heading::H2 class="mt-1">"Bank connections"</CardTitle>
                    </div>
                    <div class="flex items-center gap-2.5">
                        <Badge tone=Tone::Outline>
                            {move || match report() {
                                None => "Loading",
                                Some(r) if r.credentials && r.banks.iter().any(|b| b.connected) => "Connected",
                                Some(_) => "Not set up",
                            }}
                        </Badge>
                        <Button size=Size::Sm disabled=busy on_click=Callback::new(sync_all)>
                            "Sync all banks"
                        </Button>
                    </div>
                </CardHeader>
                {move || {
                    running().map(|bank| {
                        view! { <p role="status" class=FEEDBACK>{format!("{}: Importing transactions", bank_name(&bank))}</p> }
                    })
                }}
                {move || {
                    report().filter(|r| !r.credentials).map(|_| {
                        view! {
                            <p role="status" class=FEEDBACK>
                                "Add the Enable Banking application key below to start syncing. Each bank then syncs once a day while Finance is open."
                            </p>
                        }
                    })
                }}
                {move || {
                    let current = report();
                    let ledger = accounts.data.get().and_then(Result::ok).map(|r| r.accounts).unwrap_or_default();
                    let active = running();
                    BANKS
                        .iter()
                        .map(|(id, name)| {
                            let bank = current.as_ref().and_then(|r| r.banks.iter().find(|b| b.id == *id).cloned());
                            let own: Vec<BankAccount> = ledger.iter().filter(|a| a.bank.as_deref() == Some(*name)).cloned().collect();
                            view! {
                                <BankBox
                                    id=(*id).to_owned()
                                    name=(*name).to_owned()
                                    bank=bank
                                    loaded=current.is_some()
                                    working=active.as_deref() == Some(*id)
                                    accounts=own
                                    disabled=busy
                                    on_sync=Callback::new(sync)
                                />
                            }
                        })
                        .collect_view()
                }}
                {move || {
                    status.data.get().is_some().then(|| {
                        view! { <p class="mt-4 text-xs text-fg-muted">{format!("Updated {}", now_label())}</p> }
                    })
                }}
            </Card>
            <ConsentCard ops=ops busy=busy report=status.data />
            <KeyCard ops=ops busy=busy report=status.data />
        </div>
    }
}

#[component]
fn BankBox(
    id: String,
    name: String,
    bank: Option<BankState>,
    loaded: bool,
    working: bool,
    accounts: Vec<BankAccount>,
    disabled: Signal<bool>,
    on_sync: Callback<String>,
) -> impl IntoView {
    let status = bank.as_ref().and_then(|b| b.run.status);
    let cooling = bank.as_ref().is_some_and(|b| b.retry_after.is_some());
    let connected = bank.as_ref().is_some_and(|b| b.connected);
    let (label, detail) = match status {
        Some(s) => status_label(s),
        None if !loaded => ("Loading…", ""),
        None if connected => ("Connected", "Ready to sync."),
        None => ("Not connected", "Renew bank consent below."),
    };
    let attention = (status.is_some_and(|s| !s.is_ok()) || (loaded && !connected)) && !working;
    let headline = if working {
        "Importing transactions"
    } else if cooling {
        "Rate limited · waiting to retry"
    } else {
        label
    };
    let note = if working {
        Some("Sync in progress.")
    } else {
        attention.then_some(detail)
    };
    let date = |v: Option<&String>| local_date(v.map(String::as_str));
    let facts = [
        (
            "Last import",
            date(bank.as_ref().and_then(|b| b.run.last_success.as_ref())),
        ),
        (
            "Last attempt",
            date(bank.as_ref().and_then(|b| b.run.last_attempt.as_ref())),
        ),
        (
            if cooling {
                "Retry after"
            } else {
                "Next eligible"
            },
            date(
                bank.as_ref()
                    .and_then(|b| b.retry_after.as_ref().or(b.next_eligible.as_ref())),
            ),
        ),
    ];
    let result = (status == Some(SyncStatus::Success)).then(|| {
        let run = bank.as_ref().map(|b| &b.run);
        match (run.and_then(|r| r.created), run.and_then(|r| r.checked)) {
            (Some(n), _) => format!("{n} new transactions."),
            (None, Some(n)) => format!("{n} transactions checked."),
            _ => String::new(),
        }
    });
    let linked = bank
        .as_ref()
        .map(|b| b.accounts.join(", "))
        .filter(|l| !l.is_empty());
    let sync_label = format!("Sync {name}");
    view! {
        <div class="mt-4 grid gap-3 rounded-lg border border-border p-4 sm:p-5">
            <div class="flex flex-wrap items-baseline justify-between gap-2.5">
                <strong class="font-semibold">{name.clone()}</strong>
                <span class=cn(["text-sm", if attention { "text-warning-fg" } else { "text-accent" }])>{headline}</span>
            </div>
            {note.map(|d| view! { <p class="text-sm text-fg-muted">{d}</p> })}
            <dl class="grid gap-4 sm:grid-cols-3">
                {facts
                    .into_iter()
                    .map(|(term, value)| {
                        view! {
                            <div>
                                <dt class="text-xs text-fg-muted">{term}</dt>
                                <dd class="mt-1.5 text-sm">{value}</dd>
                            </div>
                        }
                    })
                    .collect_view()}
            </dl>
            {result.filter(|r| !r.is_empty()).map(|r| view! { <p class="text-sm text-fg-muted">{r}</p> })}
            {linked.map(|l| view! { <p class="text-xs text-fg-muted">{format!("Fills {l}")}</p> })}
            <AccountTable name=name accounts=accounts />
            <div>
                <Button
                    variant=Variant::Secondary
                    size=Size::Sm
                    disabled=Signal::derive(move || disabled.get() || cooling || !connected)
                    on_click=Callback::new(move |_| on_sync.run(id.clone()))
                >
                    {sync_label}
                </Button>
            </div>
        </div>
    }
}

/// The bank's ledger accounts with their balances (not on the site).
#[component]
fn AccountTable(name: String, accounts: Vec<BankAccount>) -> impl IntoView {
    if accounts.is_empty() {
        return None;
    }
    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for a in &accounts {
        for m in &a.account.balances {
            *totals.entry(m.currency.clone()).or_default() += m.amount;
        }
    }
    let rows = accounts
        .into_iter()
        .map(|a| {
            let balance = a
                .account
                .balances
                .iter()
                .map(|m| money(m.amount, &Currency::new(&m.currency)))
                .collect::<Vec<_>>()
                .join(" · ");
            let negative = a.account.balances.iter().any(|m| m.amount.is_sign_negative() && !m.amount.is_zero());
            view! {
                <tr class=TROW>
                    <td class=TCELL>
                        <strong class="block font-semibold">{a.account.name.clone()}</strong>
                        {(!a.active).then(|| view! { <small class="mt-1 block text-xs text-fg-muted">"Closed"</small> })}
                    </td>
                    <td class=cn([TCELL, "whitespace-nowrap text-fg-muted tabular-nums"])>
                        {a.account.iban_tail.map(|t| format!("•••• {t}")).unwrap_or_else(|| "—".into())}
                    </td>
                    <td class=cn([TCELL, "text-fg-muted tabular-nums"])>
                        {a.account.last_activity.as_deref().map_or("—".into(), format::date)}
                    </td>
                    <td class=cn([TCELL_NUM, "whitespace-nowrap", if negative { "text-negative-fg" } else { "" }])>
                        {if balance.is_empty() { "—".to_owned() } else { balance }}
                    </td>
                </tr>
            }
        })
        .collect::<Vec<_>>();
    let total = totals
        .iter()
        .map(|(c, v)| money(*v, &Currency::new(c)))
        .collect::<Vec<_>>()
        .join(" · ");
    let table_label = format!("{name} accounts");
    Some(view! {
        <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label=table_label>
            <table class=TABLE data-density="sm">
                <thead class="text-left">
                    <tr>
                        <th class=TH>"Account"</th>
                        <th class=TH>"IBAN"</th>
                        <th class=TH>"Last transaction"</th>
                        <th class=TH_NUM>{format!("Balance · {total}")}</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    })
}

#[component]
fn ConsentCard(
    ops: Ops,
    busy: Signal<bool>,
    report: ReadSignal<Option<Result<BankSyncReport, String>>>,
) -> impl IntoView {
    let app = state();
    let bank = RwSignal::new(BANKS[0].0.to_owned());
    let has_file = RwSignal::new(false);
    let file = NodeRef::<InputElement>::new();
    let cooling = move || {
        report.with(|r| {
            r.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|r| {
                r.banks
                    .iter()
                    .any(|b| b.id == bank.get() && b.retry_after.is_some())
            })
        })
    };
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if !ops.begin() {
            return;
        }
        let chosen = bank.get_untracked();
        spawn_local(async move {
            let result = async {
                let configuration = file_text(file, 64_000, "configuration").await?;
                invoke::<BankSyncResult>(
                    "renew_bank_consent",
                    &ConsentArgs {
                        consent: BankConsent {
                            bank: chosen,
                            configuration,
                        },
                    },
                )
                .await
            }
            .await;
            match result.as_ref().map(outcome) {
                Ok(Ok(notice)) => ops.notice.set(format!("Consent renewed. {notice}")),
                Ok(Err(error)) => ops.error.set(format!("Consent renewed. {error}")),
                Err(error) => ops.error.set(error.clone()),
            }
            ops.busy.set(false);
            app.revision.update(|n| *n += 1);
        });
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"BANK CONSENT"</Eyebrow>
            <CardTitle level=Heading::H2 class="mb-4 mt-1">"Renew consent"</CardTitle>
            <form class="grid items-end gap-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)_auto]" on:submit=submit>
                <Field label="Bank" id="consent-bank">
                    <Select
                        id="consent-bank"
                        value=bank
                        options=Signal::derive(|| BANKS.iter().map(|(id, name)| ((*id).to_owned(), (*name).to_owned())).collect())
                    />
                </Field>
                <Field label="Configuration" id="consent-file">
                    <input
                        id="consent-file"
                        node_ref=file
                        type="file"
                        accept=".json,application/json"
                        class=cn([CONTROL, FILE_INPUT])
                        on:change=move |e| {
                            let input: web_sys::HtmlInputElement = event_target(&e);
                            has_file.set(input.files().is_some_and(|f| f.length() > 0));
                        }
                    />
                </Field>
                <Button submit=true disabled=Signal::derive(move || busy.get() || !has_file.get() || cooling())>
                    "Renew consent"
                </Button>
            </form>
            <p class="mt-3 text-xs text-fg-muted">
                "Existing accounts must stay mapped to the same ledger accounts; new accounts may be added."
            </p>
        </Card>
    }
}

#[component]
fn KeyCard(
    ops: Ops,
    busy: Signal<bool>,
    report: ReadSignal<Option<Result<BankSyncReport, String>>>,
) -> impl IntoView {
    let app = state();
    let app_id = RwSignal::new(String::new());
    let has_file = RwSignal::new(false);
    let file = NodeRef::<InputElement>::new();
    let current = move || report.get().and_then(Result::ok);
    let stored = move || current().is_some_and(|r| r.credentials);
    let legacy = move || current().is_some_and(|r| r.legacy_consents);
    let save = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if !ops.begin() {
            return;
        }
        spawn_local(async move {
            let result = async {
                let private_key = file_text(file, 16_000, "private key").await?;
                invoke::<()>(
                    "set_bank_credentials",
                    &CredentialArgs {
                        credentials: BankCredentials {
                            app_id: app_id.get_untracked(),
                            private_key,
                        },
                    },
                )
                .await
            }
            .await;
            match result {
                Ok(()) => {
                    app_id.set(String::new());
                    has_file.set(false);
                    if let Some(input) = file.get_untracked() {
                        input.set_value("");
                    }
                    ops.notice
                        .set("Application key saved in the Keychain.".into());
                }
                Err(e) => ops.error.set(e),
            }
            ops.busy.set(false);
            app.revision.update(|n| *n += 1);
        });
    };
    let forget = move |_| {
        ops.run("forget_bank_credentials", NoArgs {}, move |()| {
            ops.notice
                .set("Application key removed from the Keychain.".into());
            app.revision.update(|n| *n += 1);
        });
    };
    let adopt = move |_| {
        ops.run(
            "adopt_bank_consents",
            NoArgs {},
            move |banks: Vec<String>| {
                let names: Vec<&str> = banks.iter().map(|b| bank_name(b)).collect();
                ops.notice.set(if names.is_empty() {
                    "No saved consent to take over.".into()
                } else {
                    format!("Took over the consents of {}.", names.join(", "))
                });
                app.revision.update(|n| *n += 1);
            },
        );
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"ENABLE BANKING"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Application key"</CardTitle>
            <CardDescription class="mb-4">
                "Every bank request is signed with your Enable Banking application key. The id and key stay in the Keychain on this Mac."
            </CardDescription>
            {move || if stored() {
                view! {
                    <div class="flex flex-wrap items-center justify-between gap-3">
                        <p class="text-sm">"Stored in the Keychain."</p>
                        <Button variant=Variant::Secondary size=Size::Sm disabled=busy on_click=Callback::new(forget)>
                            "Remove key"
                        </Button>
                    </div>
                }
                .into_any()
            } else {
                view! {
                    <form class="grid items-end gap-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]" on:submit=save>
                        <Field label="Application id" id="eb-app-id">
                            <Input id="eb-app-id" value=app_id />
                        </Field>
                        <Field label="Private key (.pem)" id="eb-key">
                            <input
                                id="eb-key"
                                node_ref=file
                                type="file"
                                accept=".pem,application/x-pem-file"
                                class=cn([CONTROL, FILE_INPUT])
                                on:change=move |e| {
                                    let input: web_sys::HtmlInputElement = event_target(&e);
                                    has_file.set(input.files().is_some_and(|f| f.length() > 0));
                                }
                            />
                        </Field>
                        <Button
                            submit=true
                            disabled=Signal::derive(move || busy.get() || !has_file.get() || app_id.get().trim().is_empty())
                        >
                            "Save key"
                        </Button>
                    </form>
                }
                .into_any()
            }}
            {move || legacy().then(|| view! {
                <div class="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
                    <p class="text-sm text-fg-muted">
                        "The old bank importer saved consents on this Mac. Take them over to sync those banks here."
                    </p>
                    <Button variant=Variant::Secondary size=Size::Sm disabled=busy on_click=Callback::new(adopt)>
                        "Use saved consents"
                    </Button>
                </div>
            })}
        </Card>
    }
}
