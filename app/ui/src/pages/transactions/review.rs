//! "Transaction inbox": the daily review queue, earlier history, set-aside
//! items, the automatic-category audit and the current batch, with decisions
//! saved in the background (personal-site `owner-review.tsx`).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use finance_types::report::ReviewReport;
use finance_types::report::review::{
    Audited, BatchDecision, BatchSaved, CategoryChange, CategorySaved, DailyInboxItem,
    DecisionStatus, DecisionsSaved, ProposalStatus, ReviewAction, ReviewDecision, ReviewItem,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;

use super::consolidation::CategoryConsolidation;
use super::details::TransactionDetails;
use super::research::{MerchantResearch, RequestSuggestions};
use super::{Choice, clean_error, focus_id, options, region_name};
use crate::api::{NoArgs, invoke, load};
use crate::ds::patterns::{
    BATCH_TITLE, BUTTON_GROUP, CHIPS, Disclosure, HINT, INSET, ITEM_HEADING, ITEM_SUMMARY, MUTED,
    QUEUE_TITLE, SECTION_GRID, STACK, SUGGESTION_BADGE, WARNING_LINE,
};
use crate::ds::{
    Badge, CONTROL, CardTone, EmptyState, Eyebrow, FactsRow, Field, Heading, Money, Size, Skeleton,
    Tone, Variant, button_class, card_class, cn,
};
use crate::pages::Problem;
use crate::{AppState, state};

const SWIPE_BUTTON: &str =
    "h-auto min-h-14 whitespace-normal rounded-lg px-2 py-2 text-sm font-semibold";
const REVIEW_LAYOUT: &str =
    "grid items-start gap-4 md:grid-cols-[minmax(230px,290px)_minmax(0,1fr)]";
const QUEUE: &str = "flex overflow-x-auto overflow-y-hidden rounded-xl border border-border bg-card-muted md:block md:max-h-[min(70vh,760px)] md:overflow-x-hidden md:overflow-y-auto";
const QUEUE_EMPTY: &str = "p-4 text-sm text-fg-muted";
const FOCUS: &str = "grid min-w-0 gap-4 rounded-xl border border-border bg-card p-4 sm:p-5";
const FOCUS_NAV: &str = "flex flex-col items-start justify-between gap-3 text-xs text-fg-muted sm:flex-row sm:items-center";
/// `ListItemButton variant="queue"`.
const QUEUE_ITEM: &str = "text-left text-fg transition-colors focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 relative grid w-[220px] min-w-[190px] max-w-[240px] shrink-0 gap-1 border-r border-line px-4 py-3 hover:bg-soft focus-visible:-outline-offset-2 md:w-full md:min-w-0 md:max-w-none md:border-r-0 md:border-b";
const QUEUE_SELECTED: &str = "bg-accent-soft after:absolute after:inset-x-0 after:bottom-0 after:h-[3px] after:bg-accent md:after:inset-x-auto md:after:inset-y-0 md:after:left-0 md:after:h-auto md:after:w-[3px]";
const MISSING: &str = "Missing bank description";

#[derive(Serialize)]
struct DecisionArgs {
    decisions: Vec<ReviewDecision>,
}

#[derive(Serialize)]
struct BatchArgs {
    items: Vec<BatchDecision>,
}

#[derive(Serialize)]
struct ChangeArgs {
    change: CategoryChange,
}

#[derive(Serialize)]
struct AuditArgs {
    transaction: String,
}

/// JavaScript `a || b`: the first non-empty text.
fn first(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .flatten()
        .find(|v| !v.is_empty())
        .map(|v| (*v).to_owned())
}

fn percent(value: Decimal) -> String {
    format!(
        "{}%",
        (value * Decimal::from(100)).round().to_i64().unwrap_or(0)
    )
}

fn day(at: Option<&str>) -> Option<String> {
    at.and_then(|a| a.get(..10))
        .filter(|d| !d.is_empty())
        .map(str::to_owned)
}

fn country_label(code: &str) -> String {
    format!("{} ({code})", region_name(code))
}

fn queue_name(item: &ReviewItem) -> String {
    first(&[
        Some(item.owner_details.values.merchant_name.as_str()),
        item.extras.merchant_name.as_ref().map(|m| m.value.as_str()),
        item.transaction_description.as_deref(),
        Some(item.merchant.as_str()),
    ])
    .unwrap_or_else(|| MISSING.into())
}

fn is_jev(item: &ReviewItem) -> bool {
    item.model_version
        .as_deref()
        .is_some_and(|v| v.starts_with("jev-"))
}

fn matches(query: &str, values: &[Option<&str>]) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || values
            .iter()
            .flatten()
            .any(|v| v.to_lowercase().contains(&q))
}

/// Everything the inbox shows, derived from the review report and the
/// owner's local state.
#[derive(Clone, Debug, Default, PartialEq)]
struct Model {
    daily: bool,
    today: String,
    pending: Vec<ReviewItem>,
    historical: Vec<ReviewItem>,
    deferred: Vec<ReviewItem>,
    open: Vec<DailyInboxItem>,
    current_batch: Vec<ReviewItem>,
    inventory: Vec<DailyInboxItem>,
    history_queue: Vec<ReviewItem>,
    queue: Vec<DailyInboxItem>,
    high_score: usize,
}

fn model(
    report: &ReviewReport,
    filter: &str,
    query: &str,
    hidden: &[String],
    reviewed_inventory: &[String],
) -> Model {
    let is_hidden = |id: &str| hidden.iter().any(|h| h == id);
    let pending: Vec<ReviewItem> = report
        .review_items
        .iter()
        .filter(|i| i.status == ProposalStatus::Pending && !is_hidden(&i.proposal_id))
        .cloned()
        .collect();
    let daily = report.daily_review.as_ref();
    let items: Vec<DailyInboxItem> = daily.map(|d| d.items.clone()).unwrap_or_default();
    let today = daily.map(|d| d.today.clone()).unwrap_or_default();
    let daily_ids: HashSet<&str> = items
        .iter()
        .filter_map(|e| e.proposal.as_ref().map(|p| p.proposal_id.as_str()))
        .collect();
    let historical: Vec<ReviewItem> = pending
        .iter()
        .filter(|p| !p.deferred && !daily_ids.contains(p.proposal_id.as_str()))
        .cloned()
        .collect();
    let deferred: Vec<ReviewItem> = pending.iter().filter(|p| p.deferred).cloned().collect();
    let proposal_deferred = |e: &DailyInboxItem| e.proposal.as_ref().is_some_and(|p| p.deferred);
    let proposal_hidden = |e: &DailyInboxItem| {
        e.proposal
            .as_ref()
            .is_some_and(|p| is_hidden(&p.proposal_id))
    };
    let open: Vec<DailyInboxItem> = items
        .iter()
        .filter(|e| !proposal_deferred(e) && e.status != "done" && !proposal_hidden(e))
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    let current_batch: Vec<ReviewItem> = items
        .iter()
        .filter(|e| {
            !proposal_deferred(e)
                && e.entry.batch_date == today
                && e.status != "done"
                && e.proposal.as_ref().is_some_and(|p| {
                    p.status == ProposalStatus::Pending
                        && p.proposed_category.is_some()
                        && !is_hidden(&p.proposal_id)
                })
        })
        .filter_map(|e| e.proposal.clone())
        .filter(|p| seen.insert(p.proposal_id.clone()))
        .collect();
    let reviewed = |e: &DailyInboxItem| reviewed_inventory.contains(&e.entry.transaction_id);
    let automatic_open = |e: &DailyInboxItem| {
        e.status == "done"
            && e.entry.resolution.as_deref() == Some("automatic")
            && e.entry.audited_at.is_none()
            && !reviewed(e)
    };
    let inventory: Vec<DailyInboxItem> = items
        .iter()
        .filter(|e| automatic_open(e))
        .cloned()
        .collect();
    let history_queue: Vec<ReviewItem> = if filter == "unknown" {
        &deferred
    } else {
        &historical
    }
    .iter()
    .filter(|i| {
        matches(
            query,
            &[
                i.transaction_description.as_deref(),
                Some(i.merchant.as_str()),
                i.proposed_category.as_deref(),
                Some(i.owner_details.values.merchant_name.as_str()),
            ],
        )
    })
    .cloned()
    .collect();
    let queue: Vec<DailyInboxItem> = items
        .iter()
        .filter(|e| {
            let in_view = match filter {
                "inventory" => automatic_open(e),
                "done" => e.status == "done" && !automatic_open(e),
                _ => {
                    e.status != "done"
                        && (filter == "open"
                            || (filter == "today" && e.entry.batch_date == today)
                            || (filter == "carryover" && e.entry.batch_date < today)
                            || (filter == "missed" && e.status == "missed"))
                }
            };
            !proposal_deferred(e)
                && !proposal_hidden(e)
                && in_view
                && matches(
                    query,
                    &[
                        e.description.as_deref(),
                        e.proposal
                            .as_ref()
                            .and_then(|p| p.proposed_category.as_deref()),
                        e.proposal
                            .as_ref()
                            .map(|p| p.owner_details.values.merchant_name.as_str()),
                    ],
                )
        })
        .cloned()
        .collect();
    let high = |i: &ReviewItem| {
        is_jev(i)
            && i.proposed_category.is_some()
            && i.confidence.is_some_and(|c| c >= Decimal::new(9, 1))
    };
    let high_score = if daily.is_some() {
        historical
            .iter()
            .chain(deferred.iter())
            .chain(open.iter().filter_map(|e| e.proposal.as_ref()))
            .filter(|i| high(i))
            .count()
    } else {
        pending
            .iter()
            .filter(|i| is_jev(i) && i.confidence.is_some_and(|c| c >= Decimal::new(9, 1)))
            .count()
    };
    Model {
        daily: daily.is_some(),
        today,
        pending,
        historical,
        deferred,
        open,
        current_batch,
        inventory,
        history_queue,
        queue,
        high_score,
    }
}

/// Decisions waiting to be saved, sent in small batches in the background.
#[derive(Clone, Copy)]
struct Stream {
    app: AppState,
    queue: StoredValue<Vec<ReviewDecision>>,
    active: StoredValue<bool>,
    timer: StoredValue<Option<TimeoutHandle>>,
    refresh_timer: StoredValue<Option<TimeoutHandle>>,
    needs_refresh: StoredValue<bool>,
    hidden: RwSignal<Vec<String>>,
    saving: RwSignal<Vec<String>>,
    deferred_ids: RwSignal<Vec<String>>,
    error: RwSignal<String>,
}

impl Stream {
    fn new(app: AppState) -> Self {
        Self {
            app,
            queue: StoredValue::new(Vec::new()),
            active: StoredValue::new(false),
            timer: StoredValue::new(None),
            refresh_timer: StoredValue::new(None),
            needs_refresh: StoredValue::new(false),
            hidden: RwSignal::new(Vec::new()),
            saving: RwSignal::new(Vec::new()),
            deferred_ids: RwSignal::new(Vec::new()),
            error: RwSignal::new(String::new()),
        }
    }

    fn clear_timer(timer: StoredValue<Option<TimeoutHandle>>) {
        if let Some(Some(handle)) = timer.try_get_value() {
            handle.clear();
        }
        timer.try_set_value(None);
    }

    fn schedule_refresh(self) {
        Self::clear_timer(self.refresh_timer);
        let app = self.app;
        self.refresh_timer.set_value(
            set_timeout_with_handle(move || app.refresh(), Duration::from_millis(350)).ok(),
        );
    }

    fn push(self, decision: ReviewDecision) {
        if self
            .queue
            .with_value(|q| q.iter().any(|d| d.proposal_id == decision.proposal_id))
        {
            return;
        }
        Self::clear_timer(self.refresh_timer);
        self.error.set(String::new());
        let id = decision.proposal_id.clone();
        for list in [self.hidden, self.saving] {
            list.update(|l| {
                if !l.contains(&id) {
                    l.push(id.clone());
                }
            });
        }
        if decision.decision == ReviewAction::Defer {
            self.deferred_ids.update(|l| l.push(id.clone()));
        }
        self.queue.update_value(|q| q.push(decision));
        Self::clear_timer(self.timer);
        if self.queue.with_value(Vec::len) >= 20 {
            self.flush();
            return;
        }
        self.timer.set_value(
            set_timeout_with_handle(move || self.flush(), Duration::from_millis(220)).ok(),
        );
    }

    fn flush(self) {
        if self.active.get_value() || self.queue.with_value(Vec::is_empty) {
            return;
        }
        Self::clear_timer(self.timer);
        self.active.set_value(true);
        let batch: Vec<ReviewDecision> = self
            .queue
            .with_value(|q| q.iter().take(20).cloned().collect());
        self.queue.update_value(|q| {
            q.drain(..batch.len());
        });
        let ids: HashSet<String> = batch.iter().map(|d| d.proposal_id.clone()).collect();
        spawn_local(async move {
            let result =
                invoke::<DecisionsSaved>("review_decisions", &DecisionArgs { decisions: batch })
                    .await;
            match result {
                Ok(body) => {
                    let retry: Vec<_> = body
                        .results
                        .iter()
                        .filter(|r| r.status == DecisionStatus::Retry)
                        .collect();
                    let retry_ids: HashSet<&str> =
                        retry.iter().map(|r| r.proposal_id.as_str()).collect();
                    if body
                        .results
                        .iter()
                        .any(|r| r.status != DecisionStatus::Retry)
                    {
                        self.needs_refresh.set_value(true);
                    }
                    self.saving.update(|l| l.retain(|id| !ids.contains(id)));
                    self.hidden
                        .update(|l| l.retain(|id| !retry_ids.contains(id.as_str())));
                    if let Some(first) = retry.first() {
                        let n = retry.len();
                        self.error.set(format!(
                            "{n} categor{} another try. {}",
                            if n == 1 { "y needs" } else { "ies need" },
                            first
                                .error
                                .clone()
                                .unwrap_or_else(|| "The items were restored.".into())
                        ));
                    }
                }
                Err(e) => {
                    let mut all = ids;
                    self.queue.update_value(|q| {
                        all.extend(q.drain(..).map(|d| d.proposal_id));
                    });
                    self.needs_refresh.set_value(false);
                    let e = clean_error(e);
                    self.error.set(if e.is_empty() {
                        "The category stream was interrupted.".into()
                    } else {
                        e
                    });
                    self.app.refresh();
                    self.saving.update(|l| l.retain(|id| !all.contains(id)));
                    self.hidden.update(|l| l.retain(|id| !all.contains(id)));
                }
            }
            self.active.set_value(false);
            if !self.queue.with_value(Vec::is_empty) {
                self.flush();
            } else if self.needs_refresh.get_value() {
                self.needs_refresh.set_value(false);
                self.schedule_refresh();
            }
        });
    }
}

#[component]
pub fn OwnerReview() -> impl IntoView {
    let app = state();
    let review = load::<ReviewReport, _>("review", move || {
        app.revision.track();
        NoArgs {}
    });
    let data = Memo::new(move |_| {
        review
            .data
            .with(|d| d.as_ref().and_then(|r| r.as_ref().ok()).cloned())
    });
    let catalog = Memo::new(move |_| {
        app.report.data.with(|d| {
            d.as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(|r| r.category_catalog.clone())
        })
    });
    let categories = Memo::new(move |_| match catalog.get() {
        Some(c) => c.entries.into_iter().map(|e| e.name).collect::<Vec<_>>(),
        None => data.with(|d| {
            let mut names: Vec<String> = d
                .iter()
                .flat_map(|r| {
                    r.review_items
                        .iter()
                        .filter_map(|i| i.proposed_category.clone())
                        .chain(r.daily_review.iter().flat_map(|d| {
                            d.items.iter().flat_map(|i| {
                                [
                                    i.current_category.clone(),
                                    i.entry.auto_category.clone(),
                                    i.proposal
                                        .as_ref()
                                        .and_then(|p| p.proposed_category.clone()),
                                ]
                                .into_iter()
                                .flatten()
                            })
                        }))
                })
                .collect();
            names.sort();
            names.dedup();
            names
        }),
    });
    let filter = RwSignal::new("open".to_owned());
    let query = RwSignal::new(String::new());
    let active_id = RwSignal::new(String::new());
    let inventory_busy = RwSignal::new(String::new());
    let inventory_message = RwSignal::new(String::new());
    let reviewed_inventory = RwSignal::new(Vec::<String>::new());
    let stream = Stream::new(app);

    on_cleanup(move || {
        Stream::clear_timer(stream.timer);
        Stream::clear_timer(stream.refresh_timer);
        let left = stream
            .queue
            .try_update_value(std::mem::take)
            .unwrap_or_default();
        if !left.is_empty() && !stream.active.try_get_value().unwrap_or(false) {
            spawn_local(async move {
                let _ =
                    invoke::<DecisionsSaved>("review_decisions", &DecisionArgs { decisions: left })
                        .await;
                app.refresh();
            });
        }
    });

    // Deferred decisions stay hidden until the report confirms them.
    Effect::new(move |_| {
        let Some(report) = data.get() else { return };
        let ids = stream.deferred_ids.get();
        let confirmed: HashSet<String> = report
            .review_items
            .iter()
            .filter(|i| i.deferred && ids.contains(&i.proposal_id))
            .map(|i| i.proposal_id.clone())
            .collect();
        if confirmed.is_empty() {
            return;
        }
        stream
            .hidden
            .update(|l| l.retain(|id| !confirmed.contains(id)));
        stream
            .deferred_ids
            .update(|l| l.retain(|id| !confirmed.contains(id)));
    });

    let view_model = Memo::new(move |_| {
        data.with(|d| {
            d.as_ref().map(|r| {
                model(
                    r,
                    &filter.get(),
                    &query.get(),
                    &stream.hidden.get(),
                    &reviewed_inventory.get(),
                )
            })
        })
        .unwrap_or_default()
    });
    let history_view = Memo::new(move |_| matches!(filter.get().as_str(), "history" | "unknown"));
    let selected_index = Memo::new(move |_| {
        let id = active_id.get();
        view_model.with(|m| {
            m.queue
                .iter()
                .position(|e| e.entry.transaction_id == id)
                .unwrap_or(0)
        })
    });
    let selected =
        Memo::new(move |_| view_model.with(|m| m.queue.get(selected_index.get()).cloned()));
    let history_index = Memo::new(move |_| {
        let id = active_id.get();
        view_model.with(|m| {
            m.history_queue
                .iter()
                .position(|i| i.proposal_id == id)
                .unwrap_or(0)
        })
    });
    let history_selected =
        Memo::new(move |_| view_model.with(|m| m.history_queue.get(history_index.get()).cloned()));

    let on_decision = Callback::new(move |decision: ReviewDecision| {
        // Choose the successor before hiding the current item.
        view_model.with_untracked(|m| {
            if history_view.get_untracked() {
                let index = m
                    .history_queue
                    .iter()
                    .position(|i| i.proposal_id == decision.proposal_id);
                let next = index
                    .and_then(|i| {
                        m.history_queue
                            .get(i + 1)
                            .or_else(|| i.checked_sub(1).and_then(|j| m.history_queue.get(j)))
                    })
                    .map(|i| i.proposal_id.clone());
                active_id.set(next.unwrap_or_default());
            } else {
                let index = m.queue.iter().position(|e| {
                    e.proposal
                        .as_ref()
                        .is_some_and(|p| p.proposal_id == decision.proposal_id)
                });
                let next = match index {
                    Some(i) => m
                        .queue
                        .get(i + 1)
                        .or_else(|| i.checked_sub(1).and_then(|j| m.queue.get(j))),
                    None => m.queue.first(),
                }
                .map(|e| e.entry.transaction_id.clone());
                active_id.set(next.unwrap_or_default());
            }
        });
        stream.push(decision);
    });

    let audit = move |transaction: String| async move {
        invoke::<Audited>("audit_daily_review", &AuditArgs { transaction })
            .await
            .map_err(|e| {
                let e = clean_error(e);
                if e.is_empty() {
                    "Could not mark the item reviewed.".to_owned()
                } else {
                    e
                }
            })
    };
    let accept_inventory = move |transaction: String| {
        inventory_busy.set(transaction.clone());
        inventory_message.set(String::new());
        spawn_local(async move {
            match audit(transaction.clone()).await {
                Ok(_) => {
                    reviewed_inventory.update(|l| l.push(transaction));
                    inventory_message.set("Checked. The next automatic category is ready.".into());
                    app.refresh();
                }
                Err(e) => inventory_message.set(e),
            }
            inventory_busy.set(String::new());
        });
    };
    let correct_inventory = move |transaction: String,
                                  expected: Option<String>,
                                  category: String| {
        if category.is_empty() || Some(&category) == expected.as_ref() {
            return;
        }
        inventory_busy.set(transaction.clone());
        inventory_message.set(String::new());
        spawn_local(async move {
            let change = CategoryChange {
                transaction_id: transaction.clone(),
                expected_category: expected,
                category: category.clone(),
            };
            let saved = invoke::<CategorySaved>("set_transaction_category", &ChangeArgs { change })
                .await
                .map_err(|e| {
                    let e = clean_error(e);
                    if e.is_empty() {
                        "Could not correct the category.".to_owned()
                    } else {
                        e
                    }
                });
            let result = match saved {
                Ok(body) => audit(transaction.clone()).await.map(|_| body.category),
                Err(e) => Err(e),
            };
            match result {
                Ok(name) => {
                    reviewed_inventory.update(|l| l.push(transaction));
                    inventory_message.set(format!("Corrected to {name} and marked checked."));
                    app.refresh();
                }
                Err(e) => inventory_message.set(e),
            }
            inventory_busy.set(String::new());
        });
    };

    let heading = move || {
        view_model.with(|m| {
            if m.daily && history_view.get() {
                format!(
                    "{} {}",
                    m.history_queue.len(),
                    if filter.get() == "unknown" {
                        "set aside"
                    } else {
                        "older items"
                    }
                )
            } else {
                format!(
                    "{} need review",
                    if m.daily {
                        m.open.len()
                    } else {
                        m.pending.len()
                    }
                )
            }
        })
    };
    let show_search = move || {
        view_model.with(|m| {
            if history_view.get() {
                (if filter.get() == "unknown" {
                    m.deferred.len()
                } else {
                    m.historical.len()
                }) > 6
            } else {
                m.open.len() > 6
            }
        })
    };
    let pick_filter = move |value: &str| {
        filter.set(value.to_owned());
        active_id.set(String::new());
    };

    // The queue of daily entries (left column).
    let queue_list = move || {
        let m = view_model.get();
        let current = selected.get().map(|e| e.entry.transaction_id);
        let len = m.queue.len();
        if m.queue.is_empty() {
            return view! { <p class=QUEUE_EMPTY>"No transactions match this view."</p> }
                .into_any();
        }
        let ids: Vec<String> = m
            .queue
            .iter()
            .map(|e| e.entry.transaction_id.clone())
            .collect();
        let ids = StoredValue::new(ids);
        m.queue
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let id = entry.entry.transaction_id.clone();
                let is_selected = current.as_deref() == Some(id.as_str());
                let name = first(&[
                    entry.proposal.as_ref().map(|p| p.owner_details.values.merchant_name.as_str()),
                    entry.description.as_deref(),
                ])
                .unwrap_or_else(|| MISSING.into());
                let when = first(&[
                    entry.proposal.as_ref().and_then(|p| day(p.transaction_occurred_at.as_deref())).as_deref(),
                ])
                .unwrap_or_else(|| entry.entry.batch_date.clone());
                let amount = entry.amount.zip(entry.currency.clone());
                let click_id = id.clone();
                view! {
                    <button
                        type="button"
                        id=format!("review-queue-{index}")
                        class=cn([QUEUE_ITEM, if is_selected { QUEUE_SELECTED } else { "" }])
                        aria-current=is_selected.then_some("true")
                        tabindex=if is_selected { 0 } else { -1 }
                        on:keydown=move |e| {
                            let key = e.key();
                            if key != "ArrowDown" && key != "ArrowUp" {
                                return;
                            }
                            e.prevent_default();
                            let next = if key == "ArrowDown" { (index + 1).min(len - 1) } else { index.saturating_sub(1) };
                            active_id.set(ids.with_value(|ids| ids[next].clone()));
                            focus_id(&format!("review-queue-{next}"));
                        }
                        on:click=move |_| active_id.set(click_id.clone())
                    >
                        <span class=QUEUE_TITLE>{name}</span>
                        <span class=HINT>
                            {format!("{when} · ")}
                            {match amount {
                                Some((value, unit)) => view! { <Money value=value unit=unit neutral=true /> }.into_any(),
                                None => "Amount unavailable".into_any(),
                            }}
                        </span>
                    </button>
                }
            })
            .collect_view()
            .into_any()
    };

    // The earlier-history queue (left column).
    let history_list = move || {
        let m = view_model.get();
        let current = history_selected.get().map(|i| i.proposal_id);
        let len = m.history_queue.len();
        if m.history_queue.is_empty() {
            return view! { <p class=QUEUE_EMPTY>"No earlier transactions match this search."</p> }
                .into_any();
        }
        let ids = StoredValue::new(
            m.history_queue
                .iter()
                .map(|i| i.proposal_id.clone())
                .collect::<Vec<_>>(),
        );
        m.history_queue
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let is_selected = current.as_deref() == Some(item.proposal_id.as_str());
                let when = day(item.transaction_occurred_at.as_deref()).unwrap_or_else(|| "Date unavailable".into());
                let amount = item.transaction_amount.zip(item.transaction_currency.clone());
                let category = item.proposed_category.clone().unwrap_or_else(|| "Choose a category".into());
                let click_id = item.proposal_id.clone();
                view! {
                    <button
                        type="button"
                        id=format!("review-history-{index}")
                        class=cn([QUEUE_ITEM, if is_selected { QUEUE_SELECTED } else { "" }])
                        aria-current=is_selected.then_some("true")
                        on:keydown=move |e| {
                            let key = e.key();
                            if key != "ArrowDown" && key != "ArrowUp" {
                                return;
                            }
                            e.prevent_default();
                            let next = if key == "ArrowDown" { (index + 1).min(len - 1) } else { index.saturating_sub(1) };
                            active_id.set(ids.with_value(|ids| ids[next].clone()));
                            focus_id(&format!("review-history-{next}"));
                        }
                        on:click=move |_| active_id.set(click_id.clone())
                    >
                        <span class=QUEUE_TITLE>{queue_name(&item)}</span>
                        <span class=HINT>
                            {format!("{when} · ")}
                            {match amount {
                                Some((value, unit)) => view! { <Money value=value unit=unit neutral=true /> }.into_any(),
                                None => "Amount unavailable".into_any(),
                            }}
                        </span>
                        <Badge tone=Tone::Accent class="justify-self-start">{category}</Badge>
                    </button>
                }
            })
            .collect_view()
            .into_any()
    };

    let step = move |history: bool, index: usize, delta: isize| {
        let target = index.checked_add_signed(delta).unwrap_or(0);
        view_model.with_untracked(|m| {
            let id = if history {
                m.history_queue.get(target).map(|i| i.proposal_id.clone())
            } else {
                m.queue.get(target).map(|e| e.entry.transaction_id.clone())
            };
            if let Some(id) = id {
                active_id.set(id);
            }
        });
    };

    let nav = move |index: usize, len: usize, label: String, history: bool| {
        view! {
            <div class=FOCUS_NAV>
                <span>{format!("{} of {len} · {label}", index + 1)}</span>
                <div class=BUTTON_GROUP>
                    <button
                        type="button"
                        class=button_class(Variant::Secondary, Size::Sm)
                        disabled=index == 0
                        on:click=move |_| step(history, index, -1)
                    >
                        "Previous"
                    </button>
                    <button
                        type="button"
                        class=button_class(Variant::Secondary, Size::Sm)
                        disabled={index + 1 >= len}
                        on:click=move |_| step(history, index, 1)
                    >
                        "Next"
                    </button>
                </div>
            </div>
        }
    };
    let focus_pane = move || {
        let m = view_model.get();
        let index = selected_index.get();
        let Some(entry) = selected.get() else {
            return ().into_any();
        };
        let label = if filter.get() == "inventory" {
            "Automatic category audit".to_owned()
        } else if entry.status == "missed" {
            "Missed deadline".to_owned()
        } else if entry.status == "done" {
            "Completed".to_owned()
        } else {
            format!("Review by {}", entry.entry.due_at.get(..10).unwrap_or(""))
        };
        nav(index, m.queue.len(), label, false).into_any()
    };
    let focus_body = move || {
        let Some(entry) = selected.get() else {
            return ().into_any();
        };
        let description = first(&[entry.description.as_deref()]).unwrap_or_else(|| MISSING.into());
        let inventory = filter.get() == "inventory";
        let busy_here = {
            let id = entry.entry.transaction_id.clone();
            move || inventory_busy.get() == id
        };
        view! {
            {entry.entry.auto_error.is_some().then(|| view! {
                <p class="text-sm font-semibold text-warning-fg">
                    "Automatic save failed. Confirm the suggestion to retry, or choose another category."
                </p>
            })}
            {match (entry.status.as_str(), entry.proposal.clone()) {
                ("done", proposal) if inventory => {
                    let category = first(&[
                        entry.entry.auto_category.as_deref(),
                        proposal.as_ref().and_then(|p| p.proposed_category.as_deref()),
                    ])
                    .unwrap_or_else(|| "Unknown".into());
                    let mut meta = Vec::new();
                    if let Some(bank) = proposal.as_ref().and_then(|p| p.bank_name.clone()) {
                        meta.push(("Bank".to_owned(), bank));
                    }
                    if let Some(c) = proposal.as_ref().and_then(|p| p.confidence).filter(|c| !c.is_zero()) {
                        meta.push(("Jev probability".to_owned(), percent(c)));
                    }
                    let id = entry.entry.transaction_id.clone();
                    let id2 = id.clone();
                    let current = entry.current_category.clone();
                    let busy = Signal::derive(busy_here.clone());
                    let busy2 = busy_here.clone();
                    let busy3 = busy_here.clone();
                    view! {
                        <div class=STACK>
                            <div>
                                <Eyebrow level=Heading::P>"CATEGORY"</Eyebrow>
                                <h3 class="mt-1 break-words text-xl font-semibold">{description.clone()}</h3>
                            </div>
                            <p><Badge tone=Tone::Accent class="text-sm"><strong>{category}</strong></Badge></p>
                            <FactsRow rows=meta />
                            <Field label="Current category" id="inventory-category" class="max-w-md">
                                <Choice
                                    id="inventory-category"
                                    value=current.clone().unwrap_or_default()
                                    options=Signal::derive(move || {
                                        let mut list = options([("", "Choose category")]);
                                        list.extend(categories.get().into_iter().map(|c| (c.clone(), c, false)));
                                        list
                                    })
                                    disabled=busy
                                    on_change=Callback::new(move |v| correct_inventory(id.clone(), current.clone(), v))
                                />
                            </Field>
                            <div class=CHIPS>
                                <button
                                    type="button"
                                    class=button_class(Variant::Primary, Size::Md)
                                    disabled=busy2.clone()
                                    on:click=move |_| accept_inventory(id2.clone())
                                >
                                    {move || if busy3() { "Saving…" } else { "Looks right" }}
                                </button>
                            </div>
                            {proposal.map(|p| view! { <TransactionDetails item=p /> })}
                        </div>
                    }
                    .into_any()
                }
                ("done", proposal) => view! {
                    <div class=ITEM_SUMMARY>
                        <h3 class=ITEM_HEADING>{description}</h3>
                        <FactsRow rows=vec![("Completed".to_owned(), entry.entry.resolution.clone().unwrap_or_default())] />
                        {proposal.map(|p| view! { <TransactionDetails item=p /> })}
                    </div>
                }
                .into_any(),
                (_, Some(p)) if p.status == ProposalStatus::Pending => view! {
                    <ReviewRow item=p categories=categories on_decision=on_decision />
                }
                .into_any(),
                (_, None) => view! {
                    <div class=ITEM_SUMMARY>
                        <h3 class=ITEM_HEADING>{description}</h3>
                        <p class=MUTED>"Category is being prepared."</p>
                    </div>
                }
                .into_any(),
                _ => ().into_any(),
            }}
        }
        .into_any()
    };
    let history_pane = move || {
        let Some(item) = history_selected.get() else {
            return view! { <EmptyState title="Choose an earlier transaction to categorize." /> }
                .into_any();
        };
        let len = view_model.with_untracked(|m| m.history_queue.len());
        let label = if filter.get_untracked() == "unknown" {
            "Set aside — choose a category whenever you can"
        } else {
            "Earlier history"
        };
        view! {
            {nav(history_index.get_untracked(), len, label.to_owned(), true)}
            <ReviewRow item=item categories=categories on_decision=on_decision />
        }
        .into_any()
    };
    let batch = Memo::new(move |_| {
        if filter.get() == "open" {
            view_model.with(|m| m.current_batch.clone())
        } else {
            Vec::new()
        }
    });

    view! {
        <article class=cn([card_class(CardTone::Default), SECTION_GRID])>
            <div>
                <Eyebrow level=Heading::P>"TRANSACTION INBOX"</Eyebrow>
                <h2 class="mt-1 text-2xl font-semibold">{heading}</h2>
            </div>
            {move || {
                let n = stream.saving.with(Vec::len);
                (n > 0).then(|| view! {
                    <p class="max-w-full justify-self-start rounded-md bg-positive/10 px-2.5 py-1.5 text-xs text-positive-fg" role="status">
                        {format!("Saving {n} decision{}… You can continue.", if n == 1 { "" } else { "s" })}
                    </p>
                })
            }}
            {move || {
                let e = stream.error.get();
                (!e.is_empty()).then(|| view! {
                    <p class="rounded-lg border border-negative/30 bg-negative/10 px-3 py-2 text-sm text-negative-fg" role="alert">
                        {format!("Not saved: {e}")}
                    </p>
                })
            }}
            {move || match review.data.get() {
                None => Some(view! { <Skeleton class="h-40 rounded-xl" /> }.into_any()),
                Some(Err(e)) => Some(view! { <Problem error=e /> }.into_any()),
                Some(Ok(_)) => None,
            }}
            <Show when=move || data.with(Option::is_some)>
                {move || view! {
                    <FactsRow rows=vec![(
                        "Pending Jev suggestions at 90%+".to_owned(),
                        view_model.with(|m| m.high_score).to_string(),
                    )] />
                }}
                <RequestSuggestions />
                {move || catalog.get().map(|c| view! { <CategoryConsolidation catalog=c /> })}
                <Show
                    when=move || view_model.with(|m| m.daily)
                    fallback=move || {
                        view! {
                            <div class=CHIPS>
                                <button
                                    type="button"
                                    class=button_class(Variant::Secondary, Size::Md)
                                    on:click=move |_| pick_filter(if filter.get_untracked() == "unknown" { "open" } else { "unknown" })
                                >
                                    {move || if filter.get() == "unknown" { "Needs review".to_owned() } else { format!("No idea ({})", view_model.with(|m| m.deferred.len())) }}
                                </button>
                            </div>
                            {move || {
                                let unknown = filter.get() == "unknown";
                                view_model
                                    .get()
                                    .pending
                                    .into_iter()
                                    .filter(|i| i.deferred == unknown)
                                    .map(|item| view! { <ReviewRow item=item categories=categories on_decision=on_decision /> })
                                    .collect_view()
                            }}
                            {move || (view_model.with(|m| m.pending.is_empty()) && stream.saving.with(Vec::is_empty)).then(|| view! {
                                <EmptyState title="No pending proposals. This does not mean every transaction is categorized." />
                            })}
                        }
                    }
                >
                    {move || {
                        let items = batch.get();
                        (!items.is_empty()).then(|| view! { <BatchReview items=items categories=categories /> })
                    }}
                    <div class=CHIPS aria-label="Filter transactions">
                        {move || {
                            let m = view_model.get();
                            [
                                ("open", "Needs review".to_owned()),
                                ("unknown", format!("No idea ({})", m.deferred.len())),
                                ("inventory", format!("Automatically categorized ({})", m.inventory.len())),
                            ]
                            .into_iter()
                            .map(|(value, label)| {
                                let on = filter.get() == value;
                                view! {
                                    <button
                                        type="button"
                                        class=button_class(if on { Variant::Primary } else { Variant::Secondary }, Size::Md)
                                        aria-pressed=on.to_string()
                                        on:click=move |_| pick_filter(value)
                                    >
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view()
                        }}
                    </div>
                    <Show when=show_search>
                        <input
                            class=cn([CONTROL, "max-w-md"])
                            aria-label="Search transactions"
                            type="search"
                            placeholder="Search transactions"
                            prop:value=move || query.get()
                            on:input=move |e| {
                                query.set(event_target_value(&e));
                                active_id.set(String::new());
                            }
                        />
                    </Show>
                    <Show
                        when=move || history_view.get()
                        fallback=move || view! {
                            <div class=REVIEW_LAYOUT>
                                <div class=QUEUE role="group" aria-label="Transaction queue">{queue_list}</div>
                                <div class=FOCUS>
                                    {focus_pane}
                                    {focus_body}
                                    {move || {
                                        let m = inventory_message.get();
                                        (!m.is_empty()).then(|| view! { <p role="status" class=MUTED>{m}</p> })
                                    }}
                                </div>
                            </div>
                        }
                    >
                        <div class=REVIEW_LAYOUT>
                            <div class=QUEUE role="group" aria-label="Earlier transaction queue">{history_list}</div>
                            <div class=FOCUS>{history_pane}</div>
                        </div>
                    </Show>
                </Show>
            </Show>
        </article>
    }
}

/// "Current batch": every suggestion of today's batch, saved at once.
#[component]
fn BatchReview(items: Vec<ReviewItem>, categories: Memo<Vec<String>>) -> impl IntoView {
    let app = state();
    let selected = RwSignal::new(
        items
            .iter()
            .map(|i| {
                (
                    i.proposal_id.clone(),
                    i.proposed_category.clone().unwrap_or_default(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    );
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let items = StoredValue::new(items);
    let incomplete = move || {
        selected.with(|s| {
            items.with_value(|items| {
                items
                    .iter()
                    .any(|i| s.get(&i.proposal_id).is_none_or(|c| c.trim().is_empty()))
            })
        })
    };
    let save = move || {
        if busy.get_untracked() || incomplete() {
            return;
        }
        busy.set(true);
        message.set(String::new());
        let chosen = selected.get_untracked();
        let batch: Vec<BatchDecision> = items.with_value(|items| {
            items
                .iter()
                .map(|i| BatchDecision {
                    proposal_id: i.proposal_id.clone(),
                    category: chosen
                        .get(&i.proposal_id)
                        .cloned()
                        .unwrap_or_default()
                        .trim()
                        .to_owned(),
                    expected_proposed_category: i.proposed_category.clone().unwrap_or_default(),
                    expected_original_category: i.original_category.clone(),
                })
                .collect()
        });
        let count = batch.len();
        spawn_local(async move {
            match invoke::<BatchSaved>("review_batch", &BatchArgs { items: batch }).await {
                Ok(body) => {
                    message.set(format!(
                        "{} transactions saved.",
                        if body.saved > 0 { body.saved } else { count }
                    ));
                    app.refresh();
                }
                Err(e) => {
                    let e = clean_error(e);
                    message.set(if e.is_empty() {
                        "Could not save this batch.".into()
                    } else {
                        e
                    });
                }
            }
            busy.set(false);
        });
    };
    let count = items.with_value(Vec::len);
    view! {
        <section class=cn([card_class(CardTone::Muted), "grid gap-3 p-4 sm:p-5"])>
            <div class="flex flex-col items-stretch justify-between gap-3 sm:flex-row sm:items-center">
                <div>
                    <Eyebrow level=Heading::P>"CURRENT BATCH"</Eyebrow>
                    <h3 class=BATCH_TITLE>{format!("{count} ready")}</h3>
                </div>
                <button
                    type="button"
                    class=button_class(Variant::Primary, Size::Md)
                    disabled=move || busy.get() || incomplete()
                    on:click=move |_| save()
                >
                    {move || if busy.get() { "Saving…".to_owned() } else { format!("Save all {count}") }}
                </button>
            </div>
            <div class="grid border-t border-line">
                {items
                    .get_value()
                    .into_iter()
                    .map(|item| {
                        let id = item.proposal_id.clone();
                        let id2 = id.clone();
                        let base: Vec<String> = item
                            .proposed_category
                            .iter()
                            .cloned()
                            .chain(item.category_options.iter().map(|o| o.category.clone()))
                            .chain(item.runner_up.iter().cloned())
                            .collect();
                        let choices = Signal::derive(move || {
                            let mut names: Vec<String> = Vec::new();
                            for n in base.iter().cloned().chain(categories.get()) {
                                if !n.is_empty() && !names.contains(&n) {
                                    names.push(n);
                                }
                            }
                            names.into_iter().map(|n| (n.clone(), n, false)).collect::<Vec<_>>()
                        });
                        let when = day(item.transaction_occurred_at.as_deref()).unwrap_or_default();
                        let amount = item.transaction_amount.zip(item.transaction_currency.clone());
                        let label = format!(
                            "Category for {}",
                            first(&[item.transaction_description.as_deref(), Some(item.merchant.as_str())]).unwrap_or_default()
                        );
                        view! {
                            <label class="grid items-center gap-2 border-b border-line py-2.5 sm:grid-cols-[minmax(0,1fr)_minmax(150px,210px)] sm:gap-3.5">
                                <span class="grid min-w-0 gap-0.5">
                                    <strong class="truncate text-sm font-semibold">{queue_name(&item)}</strong>
                                    <small class=HINT>
                                        {when}
                                        {amount.map(|(value, unit)| view! { " · " <Money value=value unit=unit neutral=true /> })}
                                    </small>
                                </span>
                                <Choice
                                    label=label
                                    value=Signal::derive(move || selected.with(|s| s.get(&id).cloned().unwrap_or_default()))
                                    options=choices
                                    disabled=busy
                                    on_change=Callback::new(move |v| { selected.update(|s| { s.insert(id2.clone(), v); }); })
                                />
                            </label>
                        }
                    })
                    .collect_view()}
            </div>
            {move || {
                let m = message.get();
                (!m.is_empty()).then(|| view! { <p role="status" class=MUTED>{m}</p> })
            }}
        </section>
    }
}

/// One proposal: the transaction, the suggestion and the decision buttons.
#[component]
fn ReviewRow(
    item: ReviewItem,
    categories: Memo<Vec<String>>,
    on_decision: Callback<ReviewDecision>,
) -> impl IntoView {
    let details_editing = RwSignal::new(false);
    let category = RwSignal::new(item.proposed_category.clone().unwrap_or_default());
    let decision = RwSignal::new(None::<ReviewAction>);
    let busy = RwSignal::new(false);
    let done = RwSignal::new(false);
    let swipe_start = StoredValue::new(None::<(i32, i32)>);
    let swiped = StoredValue::new(false);
    let proposed = item.proposed_category.clone();
    let description = item
        .transaction_description
        .clone()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let saved_merchant =
        Some(item.owner_details.values.merchant_name.trim().to_owned()).filter(|m| !m.is_empty());
    let suggested_merchant = item
        .extras
        .merchant_name
        .as_ref()
        .map(|m| m.value.trim().to_owned())
        .filter(|m| !m.is_empty());
    let display_name = saved_merchant
        .clone()
        .or_else(|| suggested_merchant.clone())
        .unwrap_or_else(|| {
            if description.chars().count() <= 90 {
                description.clone()
            } else {
                "Transaction details".into()
            }
        });
    let placeholder = |text: &str| {
        let t = text.trim().to_lowercase();
        t == "(no description)" || t == "no description"
    };
    let title = if display_name.is_empty() || placeholder(&display_name) {
        MISSING.to_owned()
    } else {
        display_name
    };
    let missing_description = description.is_empty() || placeholder(&description);
    let jev = is_jev(&item);
    let probability = item.confidence.map(percent);
    let model_confidence = item.reasons.iter().find_map(|r| {
        r.strip_prefix("TypeSafe Choice confidence ")
            .and_then(|v| v.parse::<Decimal>().ok())
    });
    let mut method_line = vec![(
        "Source".to_owned(),
        if jev {
            "Jev"
        } else if item.method == finance_types::report::review::CategorisationMethod::Baseline {
            "Transaction history"
        } else {
            "Suggestion"
        }
        .to_owned(),
    )];
    method_line.push((
        "Category probability".to_owned(),
        match (&proposed, &probability) {
            (Some(_), Some(p)) => p.to_string(),
            _ => "Not confident".into(),
        },
    ));
    if let (true, Some(c)) = (jev, model_confidence) {
        method_line.push(("Model confidence".to_owned(), percent(c).to_string()));
    }
    let possible = {
        let parts: Vec<String> = item
            .extras
            .merchant_location
            .iter()
            .map(|l| l.value.clone())
            .chain(
                item.extras
                    .tags
                    .iter()
                    .map(|t| format!("{} / {}", t.parent, t.label)),
            )
            .filter(|p| !p.is_empty())
            .collect();
        (!item.extras.tags.is_empty() || item.extras.merchant_location.is_some())
            .then(|| vec![("Possible details".to_owned(), parts.join(" · "))])
    };
    let item = StoredValue::new(item);

    let save = move |action: ReviewAction, chosen: String| {
        if details_editing.get_untracked() || busy.get_untracked() || done.get_untracked() {
            return;
        }
        let proposed = item.with_value(|i| i.proposed_category.clone());
        if action == ReviewAction::Approve
            && (category.get_untracked() != proposed.clone().unwrap_or_default()
                || Some(&chosen) != proposed.as_ref())
        {
            return;
        }
        category.set(chosen.clone());
        let canonical = categories
            .get_untracked()
            .into_iter()
            .find(|n| n.to_lowercase() == chosen.trim().to_lowercase());
        busy.set(true);
        let (original, proposal_id) =
            item.with_value(|i| (i.original_category.clone(), i.proposal_id.clone()));
        on_decision.run(ReviewDecision {
            proposal_id,
            decision: action,
            category: match action {
                ReviewAction::Reject | ReviewAction::Defer => None,
                ReviewAction::Correct => {
                    Some(canonical.unwrap_or_else(|| chosen.trim().to_owned()))
                }
                ReviewAction::Approve => Some(chosen),
            },
            expected_proposed_category: proposed,
            expected_original_category: original,
        });
        done.set(true);
        decision.set(None);
    };
    let choose = move |action: ReviewAction| {
        let has_proposal = item.with_value(|i| i.proposed_category.is_some());
        if busy.get_untracked()
            || done.get_untracked()
            || details_editing.get_untracked()
            || (action != ReviewAction::Correct && !has_proposal)
        {
            return;
        }
        if action == ReviewAction::Correct {
            category.set(String::new());
            decision.set(Some(action));
            return;
        }
        let chosen = if action == ReviewAction::Approve {
            item.with_value(|i| i.proposed_category.clone().unwrap_or_default())
        } else {
            category.get_untracked()
        };
        save(action, chosen);
    };
    let click = move |action: ReviewAction| {
        if swiped.get_value() {
            swiped.set_value(false);
            return;
        }
        choose(action);
    };
    let has_proposal = proposed.is_some();
    let locked = move || details_editing.get() || busy.get() || done.get();
    let decide_button = move |action: ReviewAction, label: &'static str, needs_proposal: bool| {
        view! {
            <button
                type="button"
                class=move || cn([
                    button_class(if decision.get() == Some(action) { Variant::Primary } else { Variant::Ghost }, Size::Md).as_str(),
                    SWIPE_BUTTON,
                ])
                disabled=move || (needs_proposal && !has_proposal) || locked()
                on:click=move |_| click(action)
            >
                {label}
            </button>
        }
    };
    let i = item.get_value();
    let original =
        first(&[i.original_category.as_deref()]).unwrap_or_else(|| "Uncategorised".into());
    let suggestion =
        first(&[i.proposed_category.as_deref()]).unwrap_or_else(|| "No suggestion".into());
    let mut date_line = vec![(
        "Date".to_owned(),
        day(i.transaction_occurred_at.as_deref()).unwrap_or_else(|| "—".into()),
    )];
    if let Some(bank) = &i.bank_name {
        date_line.push(("Bank".to_owned(), bank.clone()));
    }
    let ranked = i.category_options.clone();
    let ranked2 = ranked.clone();
    let hint = i.description_country.as_ref().map(|c| {
        vec![
            ("Country in description".to_owned(), country_label(&c.code)),
            (
                "From".to_owned(),
                match c.basis.as_str() {
                    "description_bic" => "BIC bank code",
                    "description_iban" => "IBAN",
                    _ => "Stated country code",
                }
                .to_owned(),
            ),
            ("Sender/recipient".to_owned(), "Not confirmed".to_owned()),
        ]
    });
    let amount = i.transaction_amount.zip(i.transaction_currency.clone());

    view! {
        <article class="grid min-w-0 gap-4 border-t border-line pt-5 first:border-t-0 first:pt-0">
            <div class="flex flex-col items-start justify-between gap-3 sm:flex-row">
                <div class="min-w-0">
                    <Eyebrow level=Heading::P>"TRANSACTION"</Eyebrow>
                    <h3 class="mt-1 break-words text-xl font-semibold leading-tight sm:text-2xl">{title}</h3>
                    {(suggested_merchant.is_some() && saved_merchant.is_none()).then(|| view! {
                        <span class=HINT>"Suggested name"</span>
                    })}
                </div>
                {amount.map(|(value, unit)| view! {
                    <Money class="whitespace-nowrap text-lg font-semibold" value=value unit=unit neutral=true />
                })}
            </div>
            <FactsRow rows=date_line />
            <div class=INSET>
                <Eyebrow level=Heading::P>"BANK DESCRIPTION"</Eyebrow>
                <p class="mt-1.5 whitespace-pre-wrap break-words text-sm leading-relaxed">
                    {if missing_description { "The bank import did not include a description.".to_owned() } else { description }}
                </p>
            </div>
            <div class="flex flex-wrap items-center gap-2.5 border-b border-line pb-3 text-sm">
                <Badge tone=Tone::Outline>{original}</Badge>
                <span aria-hidden="true" class="text-fg-muted">"→"</span>
                <Badge tone=Tone::Accent class=SUGGESTION_BADGE><strong>{suggestion}</strong></Badge>
            </div>
            <FactsRow rows=method_line />
            {(!ranked.is_empty()).then(|| view! {
                <div class="flex flex-wrap gap-1.5" aria-label="Jev category probabilities">
                    {ranked
                        .into_iter()
                        .map(|o| view! { <Badge tone=Tone::Neutral>{format!("{} {}", o.category, percent(o.probability))}</Badge> })
                        .collect_view()}
                </div>
            })}
            {possible.map(|p| view! { <FactsRow rows=p /> })}
            {(i.source_country.is_some() || i.destination_country.is_some()).then(|| view! {
                <div class="flex flex-wrap gap-x-4 gap-y-1.5 text-xs text-fg-muted">
                    {i.source_country.as_ref().map(|c| view! { <span class="break-words">{format!("From {}", country_label(&c.code))}</span> })}
                    {i.destination_country.as_ref().map(|c| view! { <span class="break-words">{format!("To {}", country_label(&c.code))}</span> })}
                </div>
            })}
            {hint.map(|h| view! { <FactsRow rows=h /> })}
            <TransactionDetails item=i.clone() on_editing=Callback::new(move |dirty| details_editing.set(dirty)) />
            <Disclosure class="text-sm" summary_class="text-sm font-medium text-fg-muted" summary="Unfamiliar merchant?">
                <div class="mt-3"><MerchantResearch id=i.proposal_id.clone() /></div>
            </Disclosure>
            <Show when=move || details_editing.get()>
                <p role="status" class=WARNING_LINE>"Save or discard your detail edits before deciding on the category."</p>
            </Show>
            <div>
                <div
                    class="grid touch-pan-y select-none grid-cols-3 gap-1 rounded-xl border border-border bg-card-muted p-1"
                    aria-label="Category decision"
                    on:pointerdown=move |e| swipe_start.set_value(Some((e.client_x(), e.client_y())))
                    on:pointerup=move |e| {
                        let Some((x, y)) = swipe_start.get_value() else { return };
                        swipe_start.set_value(None);
                        let (dx, dy) = (e.client_x() - x, e.client_y() - y);
                        if dx.abs() < 65 || f64::from(dx.abs()) < f64::from(dy.abs()) * 1.3 {
                            return;
                        }
                        swiped.set_value(true);
                        choose(if dx < 0 { ReviewAction::Reject } else { ReviewAction::Approve });
                    }
                    on:pointercancel=move |_| swipe_start.set_value(None)
                >
                    {decide_button(ReviewAction::Reject, "Decline", true)}
                    {decide_button(ReviewAction::Correct, "Choose category", false)}
                    {decide_button(ReviewAction::Approve, "Confirm", true)}
                </div>
            </div>
            {(!i.deferred).then(|| view! {
                <button
                    type="button"
                    class=cn([button_class(Variant::Ghost, Size::Md).as_str(), "justify-self-start"])
                    disabled=locked
                    on:click=move |_| save(ReviewAction::Defer, category.get_untracked())
                >
                    "No idea"
                </button>
            })}
            <Show when=move || decision.get() == Some(ReviewAction::Correct)>
                <div class="grid gap-3 text-sm">
                    <strong class="font-semibold">"Suggested categories"</strong>
                    <div class="grid gap-2 sm:grid-cols-2" aria-label="Category choices">
                        {ranked2
                            .iter()
                            .cloned()
                            .enumerate()
                            .map(|(index, option)| {
                                let name = option.category.clone();
                                let name2 = name.clone();
                                view! {
                                    <button
                                        type="button"
                                        class=move || cn([
                                            button_class(if category.get() == name { Variant::Primary } else { Variant::Secondary }, Size::Md).as_str(),
                                            "h-auto min-h-14 justify-start gap-3 whitespace-normal rounded-lg px-3 py-2.5 text-left",
                                        ])
                                        disabled=move || busy.get() || done.get()
                                        on:click=move |_| save(ReviewAction::Correct, name2.clone())
                                    >
                                        <span class="grid size-6 flex-none place-items-center rounded-pill bg-accent-soft text-xs font-bold text-accent">
                                            {index + 1}
                                        </span>
                                        {format!("{} · {}", option.category, percent(option.probability))}
                                    </button>
                                }
                            })
                            .collect_view()}
                        {ranked2.is_empty().then(|| view! { <p class="text-fg-muted">"Choose from the categories below."</p> })}
                    </div>
                    <Field label="All existing categories" id=format!("all-categories-{}", item.with_value(|i| i.proposal_id.clone()))>
                        <Choice
                            id=format!("all-categories-{}", item.with_value(|i| i.proposal_id.clone()))
                            value=Signal::derive(move || {
                                let c = category.get();
                                if categories.with(|list| list.contains(&c)) { c } else { String::new() }
                            })
                            options=Signal::derive(move || {
                                let mut list = options([("", "Choose a category")]);
                                list.extend(categories.get().into_iter().map(|c| (c.clone(), c, false)));
                                list
                            })
                            disabled=Signal::derive(move || busy.get() || done.get())
                            on_change=Callback::new(move |v: String| {
                                if !v.is_empty() {
                                    save(ReviewAction::Correct, v);
                                }
                            })
                        />
                    </Field>
                    <Disclosure class="grid gap-3 border-t border-line pt-2" summary_class="text-fg-muted" summary="Create a new category only if none fits">
                        <div class="mt-3 grid gap-3">
                            <Field label="New category name" id=format!("new-category-{}", item.with_value(|i| i.proposal_id.clone()))>
                                <input
                                    id=format!("new-category-{}", item.with_value(|i| i.proposal_id.clone()))
                                    class=CONTROL
                                    maxlength=128
                                    placeholder="New category name"
                                    disabled=move || busy.get() || done.get()
                                    prop:value=move || {
                                        let c = category.get();
                                        if categories.with(|list| list.contains(&c)) { String::new() } else { c }
                                    }
                                    on:input=move |e| category.set(event_target_value(&e))
                                />
                            </Field>
                            <button
                                type="button"
                                class=cn([button_class(Variant::Primary, Size::Md).as_str(), "justify-self-start"])
                                disabled=move || {
                                    let c = category.get();
                                    busy.get() || done.get() || c.trim().is_empty() || categories.with(|list| list.contains(&c))
                                }
                                on:click=move |_| save(ReviewAction::Correct, category.get_untracked())
                            >
                                {move || if busy.get() { "Saving…" } else { "Create and use category" }}
                            </button>
                        </div>
                    </Disclosure>
                </div>
            </Show>
        </article>
    }
}
