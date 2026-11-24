//! Manage → Status: the site's "Review & health" (`owner-operations.tsx`
//! "Connections", and the "Category review" and "Account data health" cards
//! of `integrated-finance.tsx`), with the local ledger standing in for the
//! Firefly and bank-importer state.

use finance_types::manage::{ImportRun, ManageWorkspace};
use finance_types::report::ReviewReport;
use leptos::prelude::*;

use super::ui::{
    FEEDBACK, Ops, PREVIEW, frame, load_workspace, local_date, with_workspace, workspace_loaded,
};
use crate::api::{Loaded, NoArgs, load};
use crate::ds::patterns::{CARD_GRID, CARD_TITLE, TITLE};
use crate::ds::{
    Badge, Button, Card, CardHeader, CardTitle, Eyebrow, FactsRow, Heading, Help, Icon, Lucide,
    Size, Tone, Variant, cn,
};
use crate::layout::{Initial, PageView, PanelDef};
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
    provide_context(Page { ops, review });
    frame(
        ops,
        workspace,
        retry,
        view! { <PageView tab="manage" tab_name="Manage" page=PAGE /> },
    )
}

/// The Status screen's state, for its panels.
#[derive(Clone, Copy)]
struct Page {
    ops: Ops,
    review: Loaded<ReviewReport>,
}

const PAGE: &str = "manage.status";

pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "manage.status.connections",
        title: "Connections",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: false,
        render: || {
            let ops = expect_context::<Page>().ops;
            with_workspace(move |ws| view! { <Connections ws=ws ops=ops /> })
        },
        summary: None,
        shown: Some(workspace_loaded),
    },
    PanelDef {
        id: "manage.status.review",
        title: "Category review",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: true,
        render: || {
            let review = expect_context::<Page>().review;
            view! { <CategoryReview review=review /> }.into_any()
        },
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "manage.status.health",
        title: "Account data health",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: true,
        render: || health().into_any(),
        summary: None,
        shown: None,
    },
];

/// The ledger's accounts and the last bank refresh.
fn health() -> impl IntoView {
    with_report(|report| {
        let accounts = report.cash_overview.accounts.clone();
        let refreshed = report.last_firefly_refresh.clone();
        let failed = report.firefly_refresh_failed;
        view! {
            <Card>
                <CardHeader class="mb-2">
                    <CardTitle level=Heading::H2>"Account data health"</CardTitle>
                    <span aria-hidden="true" class="text-fg-muted"><Lucide icon=Icon::Database size=17 /></span>
                </CardHeader>
                <h3 class=TITLE>
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
                <Help text="Record freshness is distinct from bank import completeness." />
            </Card>
        }
    })
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
            <div class=CARD_GRID>
                {cards
                    .into_iter()
                    .map(|(name, ok, facts)| {
                        view! {
                            <div class="grid content-start gap-3 rounded-lg border border-border p-5">
                                <h3 class=CARD_TITLE>{name}</h3>
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
                <FactsRow rows=vec![("Import".to_owned(), last_import), ("Write".to_owned(), last_write)] />
            </div>
        </Card>
    }
}

/// The queue lives on the Transactions tab; this card only counts it.
#[component]
fn CategoryReview(review: Loaded<ReviewReport>) -> impl IntoView {
    let app = state();
    view! {
        <Card>
            {move || {
                let count = review
                    .data
                    .with(|d| d.as_ref().and_then(|r| r.as_ref().ok()).map_or(0, |r| r.review_items.len()));
                view! {
                    <CardHeader class="mb-2">
                        <CardTitle level=Heading::H2>"Category review"</CardTitle>
                        <Badge tone=Tone::Outline>{format!("{count} proposals")}</Badge>
                    </CardHeader>
                    <h3 class=TITLE>
                        {if count == 0 { "Nothing to review" } else { "Waiting for you" }}
                    </h3>
                    <button
                        type="button"
                        class=cn([crate::ds::button_class(Variant::Ghost, Size::Sm).as_str(), "-ml-3 mt-2 text-accent"])
                        on:click=move |_| app.go(crate::Tab::Transactions)
                    >
                        "Review in Transactions "
                        <Lucide icon=Icon::ArrowRight size=14 />
                    </button>
                }
            }}
        </Card>
    }
}
