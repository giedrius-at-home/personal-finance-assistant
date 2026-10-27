//! Manage → Status: the site's "Review & health" (`owner-operations.tsx`
//! "Connections", and the "Category review" and "Account data health" cards
//! of `integrated-finance.tsx`), with the local ledger standing in for the
//! Firefly and bank-importer state.

use finance_types::manage::{ImportRun, ManageWorkspace};
use finance_types::report::ReviewReport;
use finance_types::report::review::{CategorisationMethod, ProposalStatus};
use leptos::prelude::*;

use super::ui::{FEEDBACK, Ops, PREVIEW, frame, load_workspace, local_date};
use crate::api::{NoArgs, load};
use crate::ds::{
    Badge, Button, Card, CardHeader, CardTitle, Eyebrow, Heading, Icon, Lucide, Size, Tone,
    Variant, cn,
};
use crate::pages::with_report;
use crate::{ManageSection, state};

#[component]
pub fn Status() -> impl IntoView {
    let app = state();
    let ops = Ops::new();
    let retry = RwSignal::new(0_u32);
    let workspace = load_workspace(retry);
    let review = load::<ReviewReport, _>("review", move || {
        app.revision.track();
        NoArgs {}
    });
    view! {
        {frame(ops, workspace, retry, move |ws| view! { <Connections ws=ws ops=ops /> }.into_any())}
        <div class="grid gap-3 md:grid-cols-2">
            <CategoryReview review=review />
            {with_report(|report| {
                let accounts = report.cash_overview.accounts.clone();
                let refreshed = report.last_firefly_refresh.clone();
                let failed = report.firefly_refresh_failed;
                view! {
                    <Card>
                        <CardHeader class="mb-2">
                            <CardTitle level=Heading::H2>"Account data health"</CardTitle>
                            <span aria-hidden="true" class="text-fg-muted"><Lucide icon=Icon::Database size=17 /></span>
                        </CardHeader>
                        <h3 class="text-base font-semibold">
                            {if failed {
                                "Refresh failed"
                            } else if refreshed.is_some() {
                                "Refresh recorded"
                            } else {
                                "No bank refresh recorded."
                            }}
                        </h3>
                        <p class="mb-4 mt-1 text-sm text-fg-muted">
                            {refreshed
                                .map(|r| local_date(Some(&r)))
                                .unwrap_or_else(|| "No source refresh timestamp is available.".into())}
                        </p>
                        {accounts
                            .into_iter()
                            .map(|account| {
                                view! {
                                    <div class="grid gap-1.5 border-t border-line py-4 text-xs text-fg-muted">
                                        <strong class="text-sm font-semibold text-fg">{account.name}</strong>
                                        <span>{format!("{} recorded transactions", account.transaction_count)}</span>
                                        <span>
                                            {format!(
                                                "{} of {} expenses uncategorised",
                                                account.uncategorised_outflows,
                                                account.outflow_count,
                                            )}
                                        </span>
                                        <span>
                                            {format!(
                                                "History: {} — {}",
                                                account.first_transaction_on.as_deref().unwrap_or("Unknown"),
                                                account.last_transaction_on.as_deref().unwrap_or("Unknown"),
                                            )}
                                        </span>
                                    </div>
                                }
                            })
                            .collect_view()}
                        <p class="text-xs text-fg-muted">"Record freshness is distinct from bank import completeness."</p>
                    </Card>
                }
            })}
        </div>
    }
}

fn source_label(run: &ImportRun) -> String {
    if run.source == "csv-import" {
        "Bank CSV import".into()
    } else if run.source.starts_with("firefly") {
        "Ledger copied from Firefly".into()
    } else {
        run.source.clone()
    }
}

#[component]
fn Connections(ws: ManageWorkspace, ops: Ops) -> impl IntoView {
    let app = state();
    let busy = ops.busy;
    let ledger = ws.last_import.clone();
    let csv = ws.last_csv_import.clone();
    let imported = csv
        .as_ref()
        .and_then(|r| r.stats.get("imported").and_then(|v| v.as_u64()));
    let csv_account = csv.as_ref().and_then(|r| {
        let id = r.stats.get("account_id")?.as_i64()?.to_string();
        ws.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.name.clone())
    });
    let cards = vec![
        (
            "Local ledger".to_owned(),
            ledger.is_some(),
            vec![
                (
                    "Last change",
                    local_date(ledger.as_ref().map(|r| r.finished_at.as_str())),
                ),
                (
                    "Source",
                    ledger.as_ref().map_or("Not recorded".into(), source_label),
                ),
                ("Records", format!("{} transactions", ws.transactions)),
            ],
        ),
        (
            "Bank CSV import".to_owned(),
            csv.is_some(),
            vec![
                (
                    "Last import",
                    local_date(csv.as_ref().map(|r| r.finished_at.as_str())),
                ),
                (
                    "Account",
                    csv_account.unwrap_or_else(|| "Not recorded".into()),
                ),
                (
                    "Imported",
                    imported.map_or("Not recorded".into(), |n| format!("{n} new transactions")),
                ),
            ],
        ),
    ];
    let last_import = local_date(ws.last_import.as_ref().map(|r| r.finished_at.as_str()));
    let last_write = local_date(ws.last_write.as_deref());
    view! {
        <Card>
            <CardHeader class="flex-wrap">
                <div>
                    <Eyebrow level=Heading::P>"STATUS"</Eyebrow>
                    <CardTitle level=Heading::H2 class="mt-1">"Connections"</CardTitle>
                </div>
                <Button
                    size=Size::Sm
                    variant=Variant::Secondary
                    disabled=Signal::derive(move || busy.get())
                    on_click=Callback::new(move |_| app.refresh())
                >
                    "Refresh status"
                </Button>
            </CardHeader>
            <p class=cn([FEEDBACK, "mb-5"])>
                "Banks sync through Enable Banking once a day while Finance is open (see Banks). Statements can also come from bank CSV files you "
                <button
                    type="button"
                    class="underline underline-offset-2 hover:no-underline"
                    on:click=move |_| app.go_manage(ManageSection::Import)
                >
                    "import"
                </button>
                "."
            </p>
            <div class="grid gap-4 md:grid-cols-2">
                {cards
                    .into_iter()
                    .map(|(name, ok, facts)| {
                        view! {
                            <div class="grid content-start gap-3 rounded-lg border border-border p-5">
                                <h3 class="text-lg font-semibold">{name}</h3>
                                <Badge class="justify-self-start" tone=if ok { Tone::Positive } else { Tone::Warning }>
                                    {if ok { "success" } else { "not recorded" }}
                                </Badge>
                                <dl class="mt-2 grid gap-1.5 text-sm">
                                    {facts
                                        .into_iter()
                                        .map(|(term, value)| {
                                            view! {
                                                <dt class="text-fg-muted">{term}</dt>
                                                <dd class="mb-2 tabular-nums">{value}</dd>
                                            }
                                        })
                                        .collect_view()}
                                </dl>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
            <div class=cn([PREVIEW, "mt-5 gap-1"])>
                <strong class="font-semibold">"Last refresh"</strong>
                <p class="text-sm">{format!("Import: {last_import} · Write: {last_write}")}</p>
            </div>
        </Card>
    }
}

fn method(m: CategorisationMethod) -> &'static str {
    match m {
        CategorisationMethod::Baseline => "baseline",
        CategorisationMethod::Model => "model",
        CategorisationMethod::Abstain => "abstain",
        CategorisationMethod::Unsupported => "unsupported",
    }
}

fn status(s: ProposalStatus) -> String {
    format!("{s:?}").to_lowercase()
}

/// The queue lives on the Transactions tab; this card only previews it.
const SHOWN: usize = 8;

#[component]
fn CategoryReview(review: crate::api::Loaded<ReviewReport>) -> impl IntoView {
    let app = state();
    view! {
        <Card>
            {move || {
                let items = review.data.get().and_then(Result::ok).map(|r| r.review_items).unwrap_or_default();
                let count = items.len();
                view! {
                    <CardHeader class="mb-2">
                        <CardTitle level=Heading::H2>"Category review"</CardTitle>
                        <Badge tone=Tone::Outline>{format!("{count} proposals")}</Badge>
                    </CardHeader>
                    <p class="text-sm text-fg-muted">
                        "Read-only summary. Approve or correct categories on the "
                        <button
                            type="button"
                            class="underline underline-offset-2 hover:no-underline"
                            on:click=move |_| app.go(crate::Tab::Transactions)
                        >
                            "Transactions"
                        </button>
                        " tab."
                    </p>
                    <div class="mt-3">
                        {items
                            .into_iter()
                            .take(SHOWN)
                            .map(|item| {
                                view! {
                                    <details class=cn(["group", super::ui::DETAILS])>
                                        <summary class=cn([super::ui::SUMMARY, "flex list-none items-center justify-between gap-3 [&::-webkit-details-marker]:hidden"])>
                                            <span>
                                                {item.merchant}
                                                <small class="mt-1 block text-xs text-fg-muted">
                                                    {format!(
                                                        "{} · {}",
                                                        item.proposed_category.as_deref().unwrap_or("Abstained"),
                                                        method(item.method),
                                                    )}
                                                </small>
                                            </span>
                                            <span aria-hidden="true" class="shrink-0 text-fg-muted transition-transform group-open:rotate-180">
                                                <Lucide icon=Icon::ChevronDown size=15 />
                                            </span>
                                        </summary>
                                        <p class="mt-3 text-sm leading-relaxed text-fg-muted">{item.reasons.join(" ")}</p>
                                        <span class="mt-2 block text-xs text-fg-muted">{format!("Status: {}", status(item.status))}</span>
                                    </details>
                                }
                            })
                            .collect_view()}
                    </div>
                    {(count > SHOWN)
                        .then(|| {
                            view! {
                                <p class="border-t border-line pt-4 text-xs text-fg-muted">
                                    {format!("Showing {SHOWN} of {count} proposals.")}
                                </p>
                            }
                        })}
                }
            }}
        </Card>
    }
}
