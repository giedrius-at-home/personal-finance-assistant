//! Manage → Banks: the site's bank list (`owner-banks.tsx`) from local data.
//! Read-only: Enable Banking sync and consent renewal arrive in a later phase.

use std::collections::BTreeMap;

use finance_types::manage::{BankAccount, BanksReport};
use finance_types::report::Currency;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::ui::{ERROR, FEEDBACK, local_date};
use crate::api::{NoArgs, load};
use crate::ds::{
    Badge, Button, Card, CardHeader, CardTitle, Eyebrow, Heading, Size, TABLE, TABLE_CONTAINER,
    TCELL, TCELL_NUM, TH, TH_NUM, TROW, Tone, cn, money,
};
use crate::{ManageSection, format, state};

/// The banks of `lib/finance/bank-status.ts`, in sync order.
const BANKS: [&str; 3] = ["ABN AMRO", "Revolut", "Swedbank"];

#[component]
pub fn Banks() -> impl IntoView {
    let app = state();
    let data = load::<BanksReport, _>("banks", move || {
        app.revision.track();
        NoArgs {}
    });
    view! {
        <div class="mb-6 grid gap-4">
            {move || match data.data.get() {
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
                            {move || if data.data.get().is_some() { "Local data" } else { "Loading" }}
                        </Badge>
                        <Button size=Size::Sm on_click=Callback::new(move |_| app.go_manage(ManageSection::Import))>
                            "Import CSV"
                        </Button>
                    </div>
                </CardHeader>
                <p role="status" class=FEEDBACK>
                    "Enable Banking sync arrives in a later phase. Balances and dates below come from the transactions stored on this Mac."
                </p>
                {move || {
                    let report = data.data.get().and_then(Result::ok);
                    let loaded = report.is_some();
                    let (accounts, ledger_import) = report
                        .map(|r| (r.accounts, r.last_import.map(|i| i.finished_at)))
                        .unwrap_or_default();
                    let mut groups: Vec<(String, Vec<BankAccount>)> = BANKS
                        .iter()
                        .map(|b| {
                            let own = accounts.iter().filter(|a| a.bank.as_deref() == Some(*b)).cloned().collect();
                            ((*b).to_owned(), own)
                        })
                        .collect();
                    let other: Vec<BankAccount> = accounts.iter().filter(|a| a.bank.is_none()).cloned().collect();
                    if !other.is_empty() {
                        groups.push(("Other accounts".into(), other));
                    }
                    groups
                        .into_iter()
                        .map(|(name, accounts)| {
                            view! { <BankBox name=name accounts=accounts ledger_import=ledger_import.clone() loaded=loaded /> }
                        })
                        .collect_view()
                }}
                {move || {
                    app.updated
                        .get()
                        .map(|u| view! { <p class="mt-4 text-xs text-fg-muted">{format!("Updated {}", local_date(Some(&u)))}</p> })
                }}
            </Card>
        </div>
    }
}

#[component]
fn BankBox(
    name: String,
    accounts: Vec<BankAccount>,
    ledger_import: Option<String>,
    loaded: bool,
) -> impl IntoView {
    let last_csv = accounts
        .iter()
        .filter_map(|a| a.last_csv_import.clone())
        .max();
    let last_import = last_csv.or_else(|| ledger_import.filter(|_| !accounts.is_empty()));
    let last_transaction = accounts
        .iter()
        .filter_map(|a| a.account.last_activity.clone())
        .max();
    let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for a in &accounts {
        for m in &a.account.balances {
            *totals.entry(m.currency.clone()).or_default() += m.amount;
        }
    }
    let balance = if totals.is_empty() {
        "—".to_owned()
    } else {
        totals
            .iter()
            .map(|(c, v)| money(*v, &Currency::new(c)))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let (label, attention) = if !loaded {
        ("Loading…", false)
    } else if accounts.is_empty() {
        ("Not connected", true)
    } else {
        ("Imported", false)
    };
    let detail = (loaded && accounts.is_empty()).then_some(
        "No account of this bank is stored on this Mac yet. Import a CSV export to add one.",
    );
    let facts = [
        ("Last import", local_date(last_import.as_deref())),
        (
            "Last transaction",
            last_transaction
                .as_deref()
                .map_or("Not recorded".into(), format::date),
        ),
        ("Balance", balance),
    ];
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
    let has_rows = !rows.is_empty();
    let table_label = format!("{name} accounts");
    view! {
        <div class="mt-4 grid gap-3 rounded-lg border border-border p-4 sm:p-5">
            <div class="flex flex-wrap items-baseline justify-between gap-2.5">
                <strong class="font-semibold">{name}</strong>
                <span class=cn(["text-sm", if attention { "text-warning-fg" } else { "text-accent" }])>{label}</span>
            </div>
            {detail.map(|d| view! { <p class="text-sm text-fg-muted">{d}</p> })}
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
            {has_rows
                .then(|| {
                    view! {
                        <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label=table_label>
                            <table class=TABLE data-density="sm">
                                <thead class="text-left">
                                    <tr>
                                        <th class=TH>"Account"</th>
                                        <th class=TH>"IBAN"</th>
                                        <th class=TH>"Last transaction"</th>
                                        <th class=TH_NUM>"Balance"</th>
                                    </tr>
                                </thead>
                                <tbody>{rows}</tbody>
                            </table>
                        </div>
                    }
                })}
        </div>
    }
}
