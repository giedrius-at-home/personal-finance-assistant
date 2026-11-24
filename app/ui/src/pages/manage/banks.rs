//! Manage → Banks: the site's bank control (`owner-banks.tsx`) over the
//! app's own Enable Banking sync: per-bank status and sync, consent renewal,
//! plus the application key (the site kept it in the importer container) and
//! each bank's ledger accounts.

use std::collections::BTreeMap;
use std::time::Duration;

use finance_types::bank::{
    AccountBalance, BANKS, BankConsent, BankCredentials, BankLinkChoice, BankLinkConfirm,
    BankLinkProposal, BankLinkStart, BankState, BankSyncReport, BankSyncResult, DEFAULT_REDIRECT,
    SyncStatus, bank_name,
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
use crate::api::{Loaded, NoArgs, invoke, load};
use crate::ds::patterns::{
    DD, DETAILS_LIST, Disclosure, FACT_ROW, FILE_FORM, HINT, LIST_STACK, MUTED, ROW_TITLE,
    SECTION_STACK, SUBLINE, TOOLBAR,
};
use crate::ds::{
    Badge, Button, CONTROL, Card, CardHeader, CardTitle, Eyebrow, FactsRow, Field, Heading, Help,
    Input, Select, Size, TABLE, TABLE_CONTAINER, TCELL, TCELL_NUM, TH, TH_NUM, TROW, Tone, Variant,
    cn, money,
};
use crate::layout::{Initial, PageView, PanelDef};
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
struct ReconcileArgs {
    bank: String,
    account_id: i64,
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

    let reconcile = move |(bank, account_id): (String, i64)| {
        ops.run(
            "reconcile_bank_account",
            ReconcileArgs { bank, account_id },
            move |()| {
                ops.notice.set(
                    "Ledger balance set to the bank's balance with a reconciliation entry.".into(),
                );
                app.revision.update(|n| *n += 1);
            },
        );
    };

    provide_context(Page {
        ops,
        status,
        accounts,
        working,
        busy,
        sync: Callback::new(sync),
        sync_all: Callback::new(sync_all),
        reconcile: Callback::new(reconcile),
    });
    view! {
        <div class=SECTION_STACK>
            <Messages ops=ops />
            {move || match status.data.get() {
                Some(Err(e)) => Some(view! { <p role="alert" class=ERROR>{e}</p> }),
                _ => None,
            }}
            <PageView tab="manage" tab_name="Manage" page=PAGE />
        </div>
    }
}

/// The Banks screen's state and actions, for its panels.
#[derive(Clone, Copy)]
struct Page {
    ops: Ops,
    status: Loaded<BankSyncReport>,
    accounts: Loaded<BanksReport>,
    working: RwSignal<Option<String>>,
    busy: Signal<bool>,
    sync: Callback<String>,
    sync_all: Callback<leptos::ev::MouseEvent>,
    reconcile: Callback<(String, i64)>,
}

const PAGE: &str = "manage.banks";

pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "manage.banks.connections",
        title: "Bank connections",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: false,
        render: || connections().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "manage.banks.consent",
        title: "Connect or renew",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: || {
            let p = expect_context::<Page>();
            view! { <ConsentCard ops=p.ops busy=p.busy report=p.status.data /> }.into_any()
        },
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "manage.banks.key",
        title: "Application key",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: || {
            let p = expect_context::<Page>();
            view! { <KeyCard ops=p.ops busy=p.busy report=p.status.data /> }.into_any()
        },
        summary: None,
        shown: None,
    },
];

/// Every bank with its accounts, and "Sync all banks".
fn connections() -> impl IntoView {
    let Page {
        status,
        accounts,
        working,
        busy,
        sync,
        sync_all,
        reconcile,
        ..
    } = expect_context::<Page>();
    let report = move || status.data.get().and_then(Result::ok);
    let running = move || working.get().or_else(|| report().and_then(|r| r.running));
    view! {
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
                    <Button size=Size::Sm disabled=busy on_click=sync_all>
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
                                on_sync=sync
                                on_reconcile=reconcile
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
    on_reconcile: Callback<(String, i64)>,
) -> impl IntoView {
    let status = bank.as_ref().and_then(|b| b.run.status);
    let failure = bank
        .as_ref()
        .filter(|_| status.is_some_and(|s| !s.is_ok()))
        .and_then(|b| b.run.detail.clone());
    let compared = bank
        .as_ref()
        .map(|b| b.run.balances.clone())
        .unwrap_or_default();
    let reconcile_bank = id.clone();
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
    let consent = bank
        .as_ref()
        .and_then(|b| b.run.consent_valid_until.clone())
        .map(|until| {
            let ends = js_sys::Date::new(&JsValue::from_str(&until)).get_time();
            let soon = ends - js_sys::Date::now() < 14.0 * 86_400_000.0;
            (local_date(Some(&until)), soon)
        });
    let sync_label = format!("Sync {name}");
    view! {
        <div class="mt-4 grid gap-3 rounded-lg border border-border p-4 sm:p-5">
            <div class="flex flex-wrap items-baseline justify-between gap-2.5">
                <strong class="font-semibold">{name.clone()}</strong>
                <span class=cn(["text-sm", if attention { "text-warning-fg" } else { "text-accent" }])>{headline}</span>
            </div>
            {note.map(|d| view! { <p class=MUTED>{d}</p> })}
            <dl class=DETAILS_LIST>
                {facts
                    .into_iter()
                    .map(|(term, value)| {
                        view! {
                            <div>
                                <dt class=HINT>{term}</dt>
                                <dd class=DD>{value}</dd>
                            </div>
                        }
                    })
                    .collect_view()}
            </dl>
            {result.filter(|r| !r.is_empty()).map(|r| view! { <p class=MUTED>{r}</p> })}
            <div class=FACT_ROW>
                <FactsRow rows={
                    let mut rows = Vec::new();
                    if let Some(l) = linked {
                        rows.push(("Fills".to_owned(), l));
                    }
                    if let Some((until, _)) = &consent {
                        rows.push(("Consent until".to_owned(), until.clone()));
                    }
                    if let Some(f) = failure {
                        rows.push(("Bank answered".to_owned(), f));
                    }
                    rows
                } />
                {consent.filter(|(_, soon)| *soon).map(|_| view! { <Badge tone=Tone::Warning>"Renew consent"</Badge> })}
            </div>
            <BalanceCheck balances=compared bank=reconcile_bank disabled=disabled on_reconcile=on_reconcile />
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

/// Each synced account's balance at the bank next to the ledger's, with a
/// way to book the difference as a reconciliation (not on the site).
#[component]
fn BalanceCheck(
    balances: Vec<AccountBalance>,
    bank: String,
    disabled: Signal<bool>,
    on_reconcile: Callback<(String, i64)>,
) -> impl IntoView {
    if balances.is_empty() {
        return None;
    }
    let rows = balances
        .into_iter()
        .map(|b| {
            let currency = Currency::new(&b.currency);
            let at_bank: Decimal = b.bank.parse().unwrap_or_default();
            let in_ledger: Decimal = b.ledger.parse().unwrap_or_default();
            let difference = at_bank - in_ledger;
            let off = !difference.is_zero();
            let bank = bank.clone();
            let account_id = b.account_id;
            view! {
                <tr class=TROW>
                    <td class=TCELL>{b.account.clone()}</td>
                    <td class=TCELL_NUM>{money(at_bank, &currency)}</td>
                    <td class=TCELL_NUM>{money(in_ledger, &currency)}</td>
                    <td class=cn([TCELL_NUM, if off { "text-warning-fg" } else { "text-fg-muted" }])>
                        {if off { money(difference, &currency) } else { "Matches".to_owned() }}
                    </td>
                    <td class=TCELL>
                        {off.then(|| view! {
                            <Button
                                variant=Variant::Secondary
                                size=Size::Sm
                                disabled=disabled
                                on_click=Callback::new(move |_| on_reconcile.run((bank.clone(), account_id)))
                            >
                                "Reconcile to bank"
                            </Button>
                        })}
                    </td>
                </tr>
            }
        })
        .collect_view();
    Some(view! {
        <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label="Bank and ledger balances">
            <table class=TABLE data-density="sm">
                <thead class="text-left">
                    <tr>
                        <th class=TH>"Synced account"</th>
                        <th class=TH_NUM>"Bank"</th>
                        <th class=TH_NUM>"Ledger"</th>
                        <th class=TH_NUM>"Difference"</th>
                        <th class=TH></th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    })
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
                        <strong class=ROW_TITLE>{a.account.name.clone()}</strong>
                        {(!a.active).then(|| view! { <small class=SUBLINE>"Closed"</small> })}
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

#[wasm_bindgen(inline_js = "
export function redirect_get() { try { return window.localStorage.getItem('finance.eb-redirect'); } catch { return null; } }
export function redirect_set(v) { try { window.localStorage.setItem('finance.eb-redirect', v); } catch { } }
")]
extern "C" {
    fn redirect_get() -> Option<String>;
    fn redirect_set(value: &str);
}

#[derive(Serialize)]
struct StartArgs {
    start: BankLinkStart,
}

#[derive(Serialize)]
struct FinishArgs {
    bank: String,
    address: String,
}

#[derive(Serialize)]
struct ConfirmArgs {
    confirm: BankLinkConfirm,
}

/// Connect a bank or renew its consent from the app (the importer's
/// Enable Banking link flow): approve in the browser, paste the address the
/// bank returns to, map the shared accounts. The site's file-based renewal
/// stays available below it.
#[component]
fn ConsentCard(
    ops: Ops,
    busy: Signal<bool>,
    report: ReadSignal<Option<Result<BankSyncReport, String>>>,
) -> impl IntoView {
    let app = state();
    let bank = RwSignal::new(BANKS[0].0.to_owned());
    let redirect = RwSignal::new(redirect_get().unwrap_or_else(|| DEFAULT_REDIRECT.to_owned()));
    let started = RwSignal::new(false);
    let address = RwSignal::new(String::new());
    let proposal = RwSignal::new(None::<BankLinkProposal>);
    let choices = RwSignal::new(Vec::<(String, RwSignal<String>)>::new());
    let has_file = RwSignal::new(false);
    let file = NodeRef::<InputElement>::new();
    let credentials = move || {
        report.with(|r| {
            r.as_ref()
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|r| r.credentials)
        })
    };
    let cooling = move || {
        report.with(|r| {
            r.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|r| {
                r.banks
                    .iter()
                    .any(|b| b.id == bank.get() && b.retry_after.is_some())
            })
        })
    };
    let reset = move || {
        started.set(false);
        address.set(String::new());
        proposal.set(None);
        choices.set(Vec::new());
    };
    let open = move |_| {
        let start = BankLinkStart {
            bank: bank.get_untracked(),
            redirect_url: redirect.get_untracked().trim().to_owned(),
        };
        redirect_set(&start.redirect_url);
        reset();
        ops.run("bank_link_start", StartArgs { start }, move |_: String| {
            started.set(true);
            ops.notice.set(
                "Approve access in your browser. Then copy the address of the page the bank returns you to and paste it below.".into(),
            );
        });
    };
    let finish = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let args = FinishArgs {
            bank: bank.get_untracked(),
            address: address.get_untracked(),
        };
        ops.run("bank_link_finish", args, move |p: BankLinkProposal| {
            choices.set(
                p.accounts
                    .iter()
                    .map(|a| {
                        let target = a.suggested.map(|t| t.to_string()).unwrap_or_default();
                        (a.uid.clone(), RwSignal::new(target))
                    })
                    .collect(),
            );
            proposal.set(Some(p));
        });
    };
    let save = move |_| {
        let confirm = BankLinkConfirm {
            bank: bank.get_untracked(),
            choices: choices
                .get_untracked()
                .into_iter()
                .map(|(uid, target)| BankLinkChoice {
                    uid,
                    target: target.get_untracked().parse().ok(),
                })
                .collect(),
        };
        ops.run(
            "bank_link_confirm",
            ConfirmArgs { confirm },
            move |result: BankSyncResult| {
                match outcome(&result) {
                    Ok(notice) => ops.notice.set(format!("Connected. {notice}")),
                    Err(error) => ops.error.set(format!("Connected. {error}")),
                }
                reset();
                app.revision.update(|n| *n += 1);
            },
        );
    };
    let submit_file = move |e: leptos::ev::SubmitEvent| {
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
    let mapping = move || {
        proposal.get().map(|p| {
            let targets = p.targets.clone();
            let options: Vec<(String, String)> = std::iter::once((String::new(), "Don't import".to_owned()))
                .chain(targets.iter().map(|t| {
                    let label = match &t.iban_tail {
                        Some(tail) => format!("{} · •••• {tail}", t.name),
                        None => t.name.clone(),
                    };
                    (t.id.to_string(), label)
                }))
                .collect();
            let required: Vec<String> = p
                .required
                .iter()
                .filter_map(|id| targets.iter().find(|t| t.id == *id).map(|t| t.name.clone()))
                .collect();
            let rows = p
                .accounts
                .iter()
                .map(|a| {
                    let target = choices
                        .get_untracked()
                        .into_iter()
                        .find(|(uid, _)| *uid == a.uid)
                        .map(|(_, t)| t)
                        .unwrap_or_else(|| RwSignal::new(String::new()));
                    let id = format!("map-{}", a.uid);
                    let detail = [a.iban_tail.as_ref().map(|t| format!("•••• {t}")), a.currency.clone()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let options = options.clone();
                    view! {
                        <tr class=TROW>
                            <td class=TCELL>
                                <strong class=ROW_TITLE>{a.name.clone()}</strong>
                                <small class=SUBLINE>{detail}</small>
                            </td>
                            <td class=TCELL>
                                <Select id=id value=target options=Signal::derive(move || options.clone()) label="Ledger account" />
                            </td>
                        </tr>
                    }
                })
                .collect_view();
            view! {
                <div class=LIST_STACK>
                    <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label="Shared bank accounts">
                        <table class=TABLE data-density="sm">
                            <thead class="text-left">
                                <tr>
                                    <th class=TH>"Bank account"</th>
                                    <th class=TH>"Fills ledger account"</th>
                                </tr>
                            </thead>
                            <tbody>{rows}</tbody>
                        </table>
                    </div>
                    {(!required.is_empty()).then(|| view! {
                        <span title="The previous connection filled these accounts."><FactsRow rows=vec![("Keep mapped".to_owned(), required.join(", "))] /></span>
                    })}
                    {p.valid_until.as_deref().map(|u| view! {
                        <FactsRow rows=vec![("Access until".to_owned(), local_date(Some(u)))] />
                    })}
                    <div>
                        <Button disabled=busy on_click=Callback::new(save)>"Save connection"</Button>
                    </div>
                </div>
            }
        })
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"BANK CONSENT"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Connect or renew"</CardTitle>
            <Help text="Approve access at the bank in your browser. The bank then opens the redirect address registered with Enable Banking; that page may not load, its address is all the app needs." />
            <div class="grid items-end gap-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)_auto]">
                <Field label="Bank" id="consent-bank">
                    <Select
                        id="consent-bank"
                        value=bank
                        options=Signal::derive(|| BANKS.iter().map(|(id, name)| ((*id).to_owned(), (*name).to_owned())).collect())
                    />
                </Field>
                <Field label="Redirect URL" id="consent-redirect">
                    <Input id="consent-redirect" value=redirect />
                </Field>
                <Button
                    disabled=Signal::derive(move || busy.get() || cooling() || !credentials())
                    on_click=Callback::new(open)
                >
                    "Open bank approval"
                </Button>
            </div>
            {move || started.get().then(|| view! {
                <form class=FILE_FORM on:submit=finish>
                    <Field label="Address after approval" id="consent-address">
                        <Input id="consent-address" value=address placeholder="https://localhost:8444/eb-callback?state=…&code=…" />
                    </Field>
                    <Button submit=true disabled=Signal::derive(move || busy.get() || address.get().trim().is_empty())>
                        "Continue"
                    </Button>
                </form>
            })}
            {mapping}
            <Help text="Existing accounts must stay mapped to the same ledger accounts; new accounts may be added." />
            <Disclosure class=cn([super::ui::DETAILS, "mt-4 pb-0"]) summary_class=super::ui::SUMMARY summary="Renew from an importer configuration file">
                <form class=FILE_FORM on:submit=submit_file>
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
            </Disclosure>
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
                        .set("Application key saved privately on this Mac.".into());
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
                .set("Application key removed from this Mac.".into());
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
            <Help text="Every bank request is signed with your Enable Banking application key. The id and key stay in a private file on this Mac that only your user can read." />
            {move || if stored() {
                view! {
                    <div class=TOOLBAR>
                        <p class="text-sm">"Stored privately on this Mac."</p>
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
                    <Help text="The old bank importer saved consents on this Mac. Take them over to sync those banks here." />
                    <Button variant=Variant::Secondary size=Size::Sm disabled=busy on_click=Callback::new(adopt)>
                        "Use saved consents"
                    </Button>
                </div>
            })}
        </Card>
    }
}
