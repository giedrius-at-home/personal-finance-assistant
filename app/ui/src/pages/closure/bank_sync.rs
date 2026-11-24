//! The sync step (personal-site `components/month-closure/bank-sync.tsx`):
//! syncs every bank one after another through Enable Banking, then reports
//! banks that failed as missing, except those the owner marks as covered by
//! an imported statement CSV.

use finance_types::bank::BankSyncResult;
pub use finance_types::bank::{BANKS, bank_name};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use super::ui::{ACTIONS, LIST, SMALL, TextLink};
use crate::api::invoke;
use crate::ds::charts::ProgressBar;
use crate::ds::patterns::SECTION_BODY;
use crate::ds::{Button, Size, Variant, cn};
use crate::{ManageSection, state};

#[derive(Serialize)]
pub struct SyncArgs<'a> {
    pub bank: &'a str,
}

/// Sync `bank`; `Err` carries the short reason the step shows.
pub async fn sync_one(bank: &str) -> Result<BankSyncResult, String> {
    let result = invoke::<BankSyncResult>("sync_bank", &SyncArgs { bank }).await?;
    if result.status.is_ok() {
        Ok(result)
    } else {
        Err(result.status.reason().to_owned())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Waiting,
    Syncing,
    Finished { ok: bool, reason: Option<String> },
}

#[component]
pub fn BankSyncPanel(
    #[prop(into)] action: String,
    disabled: Signal<bool>,
    on_continue: Callback<Vec<String>>,
) -> impl IntoView {
    let app = state();
    let results = RwSignal::new(Vec::<(&'static str, Outcome)>::new());
    let running = RwSignal::new(false);
    let csv_imported = RwSignal::new(Vec::<&'static str>::new());

    let failed = move || -> Vec<&'static str> {
        results.with(|r| {
            r.iter()
                .filter(|(_, o)| matches!(o, Outcome::Finished { ok: false, .. }))
                .map(|(b, _)| *b)
                .collect()
        })
    };
    let missing = move || -> Vec<&'static str> {
        let csv = csv_imported.get();
        failed().into_iter().filter(|b| !csv.contains(b)).collect()
    };
    let finished = move || {
        results.with(|r| {
            r.len() == BANKS.len() && r.iter().all(|(_, o)| matches!(o, Outcome::Finished { .. }))
        })
    };
    let done_count = move || {
        results.with(|r| {
            r.iter()
                .filter(|(_, o)| matches!(o, Outcome::Finished { .. }))
                .count()
        })
    };

    let run = move |_| {
        running.set(true);
        csv_imported.set(Vec::new());
        results.set(BANKS.iter().map(|(b, _)| (*b, Outcome::Waiting)).collect());
        spawn_local(async move {
            let mut failures = false;
            for (bank, _) in BANKS {
                let set = |outcome: Outcome| {
                    results.try_update(|r| {
                        if let Some(slot) = r.iter_mut().find(|(b, _)| *b == bank) {
                            slot.1 = outcome;
                        }
                    })
                };
                if set(Outcome::Syncing).is_none() {
                    return; // unmounted meanwhile
                }
                let outcome = sync_one(bank).await;
                failures |= outcome.is_err();
                let finished = Outcome::Finished {
                    ok: outcome.is_ok(),
                    reason: outcome.err(),
                };
                if set(finished).is_none() {
                    return;
                }
            }
            running.set(false);
            app.revision.update(|n| *n += 1);
            if !failures {
                on_continue.run(Vec::new());
            }
        });
    };

    let action = StoredValue::new(action);
    let list = move || {
        let r = results.get();
        if r.is_empty() {
            return None;
        }
        let is_running = running.get();
        let items = BANKS
            .iter()
            .map(|(bank, name)| {
                let bank = *bank;
                let outcome = r
                    .iter()
                    .find(|(b, _)| *b == bank)
                    .map(|(_, o)| o.clone())
                    .unwrap_or(Outcome::Waiting);
                let (label, tone) = match &outcome {
                    Outcome::Waiting => ("Waiting".to_owned(), "text-fg-muted"),
                    Outcome::Syncing => ("Syncing…".to_owned(), "text-fg-muted"),
                    Outcome::Finished { ok: true, .. } => ("✓ Synced".to_owned(), "text-positive-fg"),
                    Outcome::Finished { reason, .. } => (
                        format!("✗ {}", reason.clone().unwrap_or_else(|| "Not synced".into())),
                        "text-negative-fg",
                    ),
                };
                let bad = matches!(outcome, Outcome::Finished { ok: false, .. });
                let marked = move || csv_imported.with(|c| c.contains(&bank));
                view! {
                    <li class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 py-2.5 text-sm">
                        <strong>{*name}</strong>
                        <span class=tone>{label}</span>
                        {(bad && !is_running).then(|| view! {
                            <div class=cn([ACTIONS, "basis-full"])>
                                <TextLink on_click=Callback::new(move |_| app.go_manage(ManageSection::Import))>"Upload statement CSV"</TextLink>
                                <button
                                    type="button"
                                    class=crate::ds::button_class(Variant::Secondary, Size::Sm)
                                    aria-pressed=move || marked().to_string()
                                    on:click=move |_| csv_imported.update(|c| {
                                        if c.contains(&bank) {
                                            c.retain(|b| *b != bank);
                                        } else {
                                            c.push(bank);
                                        }
                                    })
                                >
                                    {move || if marked() { "✓ CSV imported — undo" } else { "Mark CSV imported" }}
                                </button>
                                <small class=cn([SMALL, "basis-full"])>
                                    {move || if marked() { "Counts as synced." } else { "Will be recorded as missing." }}
                                </small>
                            </div>
                        })}
                    </li>
                }
            })
            .collect_view();
        Some(view! { <ul class=LIST>{items}</ul> })
    };
    let footer = move || {
        (finished() && !running.get()).then(|| {
            let missing_now = missing();
            let names: Vec<&str> = missing_now.iter().map(|b| bank_name(b)).collect();
            let label = if names.is_empty() {
                action.get_value()
            } else {
                format!("{} without {}", action.get_value(), names.join(", "))
            };
            let ids: Vec<String> = missing_now.iter().map(|b| (*b).to_owned()).collect();
            view! {
                <div class=ACTIONS>
                    {(!names.is_empty()).then(|| view! {
                        <small class=cn([SMALL, "basis-full"])>
                            {format!(
                                "{} will be recorded as missing for this month. Import a statement CSV and mark it as imported to count it as synced.",
                                names.join(", "),
                            )}
                        </small>
                    })}
                    <Button disabled=disabled on_click=Callback::new(move |_| on_continue.run(ids.clone()))>{label}</Button>
                </div>
            }
        })
    };
    view! {
        <div class=SECTION_BODY>
            <div>
                <Button disabled=Signal::derive(move || disabled.get() || running.get()) on_click=Callback::new(run)>
                    {move || if running.get() {
                        "Syncing banks…".to_owned()
                    } else {
                        format!("Sync all banks{}", if finished() { " again" } else { "" })
                    }}
                </Button>
            </div>
            {move || running.get().then(|| view! {
                <ProgressBar
                    label=format!("{} of {} banks finished", done_count(), BANKS.len())
                    value=done_count() as f64
                    max=BANKS.len() as f64
                />
            })}
            {list}
            {footer}
        </div>
    }
}
