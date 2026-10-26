//! Manage → Close month: the month-closure wizard and the Overview nudge
//! (personal-site `components/month-closure.tsx`).

mod bank_sync;
mod panels;
mod split;
mod steps;
mod suggestion;
mod transfer;
mod ui;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::future::Future;

use finance_types::report::Dashboard;
use finance_types::report::closure::{
    AccountReview, AccountRoleKind, AllocationLine, ClosureAction, ClosureStatus, ClosureView,
    LineKind, PotDraft, PotView, TransferStatus, UnclassifiedAccounts,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;

use self::bank_sync::{BankSyncPanel, bank_name};
use self::panels::{AccountsPanel, PotsPanel};
use self::split::{DraftStore, SplitEditor, SplitTarget};
use self::steps::{AllocationList, ReviewNames, ReviewStep, UnclassifiedAccounts as NewAccounts};
use self::transfer::Hop;
use self::ui::{
    ACTIONS, CALLOUT, CONFLICT, Choice, ERROR, FOOTNOTE, FieldBox, LIST, SUBHEADING, StepCard,
    WARN, date_time, field_id, load_closure, load_unclassified, money, month_label, month_to_close,
    next_month, post_closure, post_pot, post_role, previous_month, recent_months,
};
use crate::ds::{
    Button, Card, CardTone, EmptyState, Eyebrow, Icon, Lucide, Size, StepState, Variant, step_class,
};
use crate::{AppState, ManageSection, state};

const STEP_NAMES: [&str; 4] = ["Sync", "Review", "Split", "Transfers"];

/// Steps finished in each status (pills before this index show as done).
fn step_index(status: ClosureStatus) -> usize {
    match status {
        ClosureStatus::Open => 0,
        ClosureStatus::Settling => 1,
        ClosureStatus::Settled => 3,
        ClosureStatus::Closed | ClosureStatus::ChangedAfterClose => 4,
    }
}

/// The step opened by default: the one the status can act on.
fn default_step(status: ClosureStatus) -> usize {
    match status {
        ClosureStatus::Open => 0,
        ClosureStatus::Settling => 1,
        ClosureStatus::Settled | ClosureStatus::Closed => 3,
        ClosureStatus::ChangedAfterClose => 2,
    }
}

/// How far along the closure is; a move up jumps to the new default step.
fn status_rank(status: ClosureStatus) -> u8 {
    match status {
        ClosureStatus::Open => 0,
        ClosureStatus::Settling => 1,
        ClosureStatus::Settled => 2,
        ClosureStatus::Closed | ClosureStatus::ChangedAfterClose => 3,
    }
}

thread_local! {
    /// The banner hands the month it is about to the wizard, which mounts
    /// after the tab switch.
    static WANTED_MONTH: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn initial_month() -> String {
    let wanted = WANTED_MONTH.with(|w| w.borrow_mut().take());
    match wanted {
        Some(month) if recent_months().contains(&month) => month,
        _ => month_to_close(),
    }
}

/// Budget group and planned payment names from the report, for review items
/// known only by id.
fn review_names(report: Option<&Dashboard>) -> ReviewNames {
    let Some(report) = report else {
        return ReviewNames::default();
    };
    ReviewNames {
        groups: report
            .monthly_budgets
            .iter()
            .flat_map(|b| b.plan.groups.iter().map(|g| (g.id.clone(), g.name.clone())))
            .collect(),
        payments: report
            .planned_payments
            .iter()
            .map(|p| (p.plan_id.clone(), p.name.clone()))
            .collect(),
    }
}

/// The wizard's state, shared with its parts through context.
#[derive(Clone, Copy, Debug)]
pub struct Wizard {
    app: AppState,
    month: RwSignal<String>,
    view: RwSignal<Option<ClosureView>>,
    unclassified: RwSignal<Option<UnclassifiedAccounts>>,
    error: RwSignal<String>,
    notice: RwSignal<String>,
    busy: RwSignal<bool>,
    /// The step the owner opened; `None` follows the status.
    selected: RwSignal<Option<usize>>,
    /// The split being typed, kept across reloads of the month.
    drafts: RwSignal<DraftStore>,
}

impl Wizard {
    /// Re-read the month (and, while it can be synced, the new accounts).
    async fn reload(self) -> Result<(), String> {
        let month = self.month.get_untracked();
        let next = load_closure(&month).await?;
        // The month changed, or the screen closed, meanwhile.
        if self.month.try_get_untracked().as_ref() != Some(&month) {
            return Ok(());
        }
        let early = matches!(
            next.period.status,
            ClosureStatus::Open | ClosureStatus::Settling
        );
        self.view.set(Some(next));
        if early {
            let accounts = load_unclassified().await.ok();
            self.unclassified.set(accounts);
        } else {
            self.unclassified.set(None);
        }
        Ok(())
    }

    /// Runs one closure write; on success shows `done`, on a revision
    /// conflict reloads the month.
    fn write<F, Fut>(self, task: F, done: String)
    where
        F: FnOnce() -> Fut + 'static,
        Fut: Future<Output = Result<Option<ClosureView>, String>> + 'static,
    {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.error.set(String::new());
        self.notice.set(String::new());
        spawn_local(async move {
            match task().await {
                Ok(next) => {
                    if let Some(next) = next {
                        self.view.set(Some(next));
                    }
                    self.notice.set(done);
                    self.app.refresh();
                }
                Err(e) => {
                    let conflict = e == CONFLICT;
                    self.error.set(if e.is_empty() {
                        "Could not update the month closure.".into()
                    } else {
                        e
                    });
                    if conflict {
                        let _ = self.reload().await;
                    }
                }
            }
            self.busy.set(false);
        });
    }

    fn base(self) -> Option<(String, u32)> {
        let revision = self
            .view
            .with_untracked(|v| v.as_ref().map(|v| v.period.revision))?;
        Some((self.month.get_untracked(), revision))
    }

    /// A sync re-reads the month underneath us: post with the revision as it
    /// is now.
    async fn latest(self) -> Result<(String, u32), String> {
        let month = self.month.get_untracked();
        let revision = load_closure(&month).await?.period.revision;
        Ok((month, revision))
    }

    fn go(self, index: usize) {
        self.notice.set(String::new());
        self.selected.set(Some(index));
    }

    fn sync(self, missing: Vec<String>) {
        let done = if missing.is_empty() {
            "All banks synced.".to_owned()
        } else {
            let names: Vec<&str> = missing.iter().map(|b| bank_name(b)).collect();
            format!("Sync recorded without {}.", names.join(", "))
        };
        self.write(
            move || async move {
                let (month, revision) = self.latest().await?;
                let next = post_closure(ClosureAction::Sync {
                    month,
                    revision,
                    missing_banks: missing,
                })
                .await?;
                self.unclassified.set(load_unclassified().await.ok());
                Ok(Some(next))
            },
            done,
        );
    }

    fn finalize(self, missing: Vec<String>) {
        let done = format!(
            "{} closed.",
            month_label(&self.month.get_untracked(), false)
        );
        self.write(
            move || async move {
                let (month, revision) = self.latest().await?;
                post_closure(ClosureAction::Finalize {
                    month,
                    revision,
                    missing_banks: missing,
                })
                .await
                .map(Some)
            },
            done,
        );
    }

    fn settle(self, lines: Vec<AllocationLine>) {
        let Some((month, revision)) = self.base() else {
            return;
        };
        self.write(
            move || async move {
                let next = post_closure(ClosureAction::Settle {
                    month,
                    revision,
                    lines,
                })
                .await?;
                self.selected.set(None);
                Ok(Some(next))
            },
            "Split confirmed. Make the transfers below.".into(),
        );
    }

    fn resettle(self, lines: Vec<AllocationLine>) {
        let Some((month, revision)) = self.base() else {
            return;
        };
        self.write(
            move || async move {
                post_closure(ClosureAction::Resettle {
                    month,
                    revision,
                    lines,
                })
                .await
                .map(Some)
            },
            "Changes assigned. The month is closed again.".into(),
        );
    }

    fn reopen(self, confirming: RwSignal<bool>) {
        let Some((month, revision)) = self.base() else {
            return;
        };
        let name = month_label(&month, false);
        self.write(
            move || async move {
                let next = post_closure(ClosureAction::Reopen { month, revision }).await?;
                let _ = confirming.try_set(false);
                Ok(Some(next))
            },
            format!(
                "{name} reopened. Edit the split or finish the transfers, then close it again."
            ),
        );
    }

    pub fn mark(self, line_id: String, hop: Hop, ids: Vec<String>, picking: RwSignal<bool>) {
        let Some((month, revision)) = self.base() else {
            return;
        };
        self.write(
            move || async move {
                let result = post_closure(ClosureAction::Mark {
                    month,
                    revision,
                    line_id,
                    hop: hop.index(),
                    transaction_ids: ids,
                })
                .await;
                let _ = picking.try_set(false);
                result.map(Some)
            },
            "Transfer marked done.".into(),
        );
    }

    pub fn unmark(self, line_id: String, hop: Hop) {
        let Some((month, revision)) = self.base() else {
            return;
        };
        self.write(
            move || async move {
                post_closure(ClosureAction::Unmark {
                    month,
                    revision,
                    line_id,
                    hop: hop.index(),
                })
                .await
                .map(Some)
            },
            "Mark removed.".into(),
        );
    }

    /// Save an account's role; `via` `None` keeps its route. `confirming`
    /// closes the "Don't track" confirmation once saved.
    pub fn set_role(
        self,
        account_id: String,
        role: &str,
        goal: Option<String>,
        via: Option<Option<String>>,
        confirming: Option<RwSignal<Option<String>>>,
    ) {
        let role = role.to_owned();
        self.write(
            move || async move {
                post_role(account_id, &role, goal, via).await?;
                self.reload().await?;
                if let Some(c) = confirming {
                    let _ = c.try_set(None);
                }
                Ok(None)
            },
            "Account saved.".into(),
        );
    }

    /// Saves a pot and reloads the month so balances and destinations
    /// follow; resolves to the saved pot.
    async fn save_pot_now(self, pot: PotDraft) -> Option<PotView> {
        if self.busy.get_untracked() {
            return None;
        }
        self.busy.set(true);
        self.error.set(String::new());
        self.notice.set(String::new());
        let existing = pot.pot_id.is_some();
        let result = post_pot(pot).await;
        let saved = match result {
            Ok(saved) => {
                self.notice.set(if existing {
                    format!("{} saved.", saved.name)
                } else {
                    format!("{} created.", saved.name)
                });
                let _ = self.reload().await;
                Some(saved)
            }
            Err(e) => {
                self.error.set(if e.is_empty() {
                    "Could not save the pot.".into()
                } else {
                    e
                });
                None
            }
        };
        self.busy.set(false);
        saved
    }

    /// Save a pot from the Pots panel; `editing` closes the edit form.
    pub fn save_pot(self, pot: PotDraft, editing: Option<RwSignal<bool>>) {
        spawn_local(async move {
            if self.save_pot_now(pot).await.is_some()
                && let Some(editing) = editing
            {
                let _ = editing.try_set(false);
            }
        });
    }

    /// "New pot…" inside a split line: create the pot, then point the line
    /// at it.
    pub fn create_pot_for_line(
        self,
        name: String,
        location: Option<String>,
        account_id: String,
        key: String,
        creating: RwSignal<bool>,
    ) {
        creating.set(true);
        spawn_local(async move {
            let draft = PotDraft {
                pot_id: None,
                name,
                account_id: location,
                archived: false,
            };
            if let Some(pot) = self.save_pot_now(draft).await {
                self.drafts.update(|s| {
                    if let Some(line) = s
                        .accounts
                        .get_mut(&account_id)
                        .and_then(|lines| lines.iter_mut().find(|d| d.key == key))
                    {
                        line.destination = format!("pot:{}", pot.pot_id);
                    }
                });
            }
            let _ = creating.try_set(false);
        });
    }
}

/// A step that cannot act in this status: one line why, and the button
/// that gets there.
fn locked(wizard: Wizard, message: String, target: Option<usize>, label: Option<&str>) -> AnyView {
    let button = target.map(|target| {
        let label = label.map_or_else(|| format!("Go to {}", STEP_NAMES[target]), str::to_owned);
        view! {
            <div class=ACTIONS>
                <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| wizard.go(target))>{label}</Button>
            </div>
        }
    });
    view! {
        <div class=CALLOUT role="note">
            <p>{message}</p>
            {button}
        </div>
    }
    .into_any()
}

fn reopen_here(month: String, message: String) -> AnyView {
    view! {
        <div class=CALLOUT role="note">
            <p>{message}</p>
            <ReopenMonth month=month />
        </div>
    }
    .into_any()
}

/// Reopen is offered only while the next month has not been settled or
/// closed (the backend enforces it too).
#[component]
fn ReopenMonth(month: String) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let busy = Signal::derive(move || wizard.busy.get());
    let allowed = RwSignal::new(false);
    let confirming = RwSignal::new(false);
    let next = next_month(&month);
    spawn_local(async move {
        let ok = match load_closure(&next).await {
            Ok(next) => matches!(
                next.period.status,
                ClosureStatus::Open | ClosureStatus::Settling
            ),
            // No period yet for the next month.
            Err(_) => true,
        };
        let _ = allowed.try_set(ok);
    });
    let name = month_label(&month, false);
    move || {
        if !allowed.get() {
            return None;
        }
        let name = name.clone();
        Some(if confirming.get() {
            view! {
                <div class=CALLOUT role="alertdialog" aria-label=format!("Reopen {name}?")>
                    <p><strong>{format!("Reopen {name}?")}</strong></p>
                    <ul class="list-disc pl-5 text-fg-muted">
                        <li>{format!("The cutoff is removed: new transactions count towards {name} again until you close it.")}</li>
                        <li>"The split becomes editable again."</li>
                        <li>"Transfers already matched stay matched unless you change those split lines."</li>
                    </ul>
                    <div class=ACTIONS>
                        <Button size=Size::Sm disabled=busy on_click=Callback::new(move |_| wizard.reopen(confirming))>
                            {format!("Reopen {name}")}
                        </Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| confirming.set(false))>"Keep closed"</Button>
                    </div>
                </div>
            }
            .into_any()
        } else {
            view! {
                <div class=ACTIONS>
                    <Button size=Size::Sm variant=Variant::Secondary disabled=busy on_click=Callback::new(move |_| confirming.set(true))>
                        {format!("Reopen {name}")}
                    </Button>
                </div>
            }
            .into_any()
        })
    }
}

#[component]
fn ClosedSummary(view: ClosureView) -> impl IntoView {
    let period = &view.period;
    let split: Vec<AllocationLine> = period
        .allocations
        .iter()
        .filter(|l| l.kind == LineKind::Split)
        .cloned()
        .collect();
    let delta: Vec<AllocationLine> = period
        .allocations
        .iter()
        .filter(|l| l.kind == LineKind::Delta)
        .cloned()
        .collect();
    let pending = period
        .allocations
        .iter()
        .filter(|l| l.transfer_status == TransferStatus::Pending)
        .count();
    let description = format!(
        "Closed {}. The next month started at that moment.{}",
        date_time(period.cutoff_at.as_deref()),
        if period.missing_banks.is_empty() {
            String::new()
        } else {
            format!(" Closed without {}.", period.missing_banks.join(", "))
        }
    );
    let month = period.month.get(..10).unwrap_or(&period.month).to_owned();
    let has_delta = !delta.is_empty();
    let delta_view = view.clone();
    view! {
        <StepCard title="4. Transfers · closed" description=Some(description)>
            {(pending > 0).then(|| view! {
                <p class=WARN>
                    {format!(
                        "{pending} transfer{} still pending. {} can still match on a later sync.",
                        if pending == 1 { " is" } else { "s are" },
                        if pending == 1 { "It" } else { "They" },
                    )}
                </p>
            })}
            <h4 class=SUBHEADING>"Split"</h4>
            <AllocationList lines=split view=view.clone() actions=true />
            {has_delta.then(|| view! {
                <h4 class=SUBHEADING>"Changes after the split"</h4>
                <AllocationList lines=delta view=delta_view />
            })}
            <ReopenMonth month=month />
        </StepCard>
    }
}

#[component]
fn ChangedAfterClose(
    view: ClosureView,
    pots: Vec<PotView>,
    locations: Vec<(String, String)>,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    // Mirrors the backend's delta validation: clamped leftovers, and
    // accounts that dropped out of the review since the split still count
    // (their leftover is now 0).
    let snapshot: Vec<AccountReview> = view
        .period
        .settled_snapshot
        .as_ref()
        .map(|s| s.accounts.clone())
        .unwrap_or_default();
    let before: HashMap<&str, Decimal> = snapshot
        .iter()
        .map(|a| (a.account_id.as_str(), a.leftover))
        .collect();
    let current: BTreeSet<&str> = view
        .review
        .accounts
        .iter()
        .map(|a| a.account_id.as_str())
        .collect();
    let clamp = |v: Decimal| v.max(Decimal::ZERO);
    let dropped = snapshot
        .iter()
        .filter(|a| !current.contains(a.account_id.as_str()))
        .map(|a| AccountReview {
            leftover: Decimal::ZERO,
            ..a.clone()
        });
    let deltas: Vec<(AccountReview, Decimal)> = view
        .review
        .accounts
        .iter()
        .cloned()
        .chain(dropped)
        .map(|a| {
            let delta = clamp(a.leftover)
                - clamp(
                    before
                        .get(a.account_id.as_str())
                        .copied()
                        .unwrap_or_default(),
                );
            (a, delta)
        })
        .collect();
    let rows = deltas
        .iter()
        .map(|(account, delta)| {
            let reviewed = current.contains(account.account_id.as_str());
            let text = if delta.is_zero() {
                "No change".to_owned()
            } else {
                format!(
                    "{}{}",
                    if *delta > Decimal::ZERO { "+" } else { "" },
                    money(*delta, &account.currency)
                )
            };
            view! {
                <li class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 py-2.5 text-sm">
                    <span>{format!("{}{}", account.name, if reviewed { "" } else { " · no longer reviewed" })}</span>
                    <strong class="tabular-nums">{text}</strong>
                </li>
            }
        })
        .collect_view();
    let changed: Vec<SplitTarget> = deltas
        .iter()
        .filter(|(_, d)| !d.is_zero())
        .map(|(a, d)| SplitTarget {
            account_id: a.account_id.clone(),
            name: a.name.clone(),
            currency: a.currency.clone(),
            target: *d,
            note: None,
        })
        .collect();
    let none_changed = changed.is_empty();
    let month = view
        .period
        .month
        .get(..10)
        .unwrap_or(&view.period.month)
        .to_owned();
    let key = format!("{month}:{}:resettle", view.period.revision);
    let goals = view.goals.clone();
    let allocations = view.period.allocations.clone();
    view! {
        <StepCard
            title="Changed after close"
            description=Some("Transactions for this month arrived after it closed. Assign only the change; the original split stays.".to_owned())
        >
            <ul class=LIST>{rows}</ul>
            <h4 class=SUBHEADING>"Previous split"</h4>
            <AllocationList lines=allocations view=view />
            <h4 class=SUBHEADING>"Assign the change"</h4>
            {none_changed.then(|| view! { <p class=FOOTNOTE>"No leftover changed. Confirm to close the month again."</p> })}
            <SplitEditor
                month=month
                pots=pots
                locations=locations
                goals=goals
                allow_negative=true
                submit_label="Confirm changes"
                on_submit=Callback::new(move |lines| wizard.resettle(lines))
                accounts=changed
                store_key=key
            />
        </StepCard>
    }
}

/// The wizard: month picker, steps, then the Accounts and Pots panels.
#[component]
pub fn MonthClosure() -> impl IntoView {
    let app = state();
    let wizard = Wizard {
        app,
        month: RwSignal::new(initial_month()),
        view: RwSignal::new(None),
        unclassified: RwSignal::new(None),
        error: RwSignal::new(String::new()),
        notice: RwSignal::new(String::new()),
        busy: RwSignal::new(false),
        selected: RwSignal::new(None),
        drafts: RwSignal::new(DraftStore::default()),
    };
    provide_context(wizard);
    let Wizard {
        month,
        view,
        unclassified,
        error,
        notice,
        busy,
        selected,
        ..
    } = wizard;

    Effect::new(move |_| {
        let wanted = month.get();
        view.set(None);
        selected.set(None);
        error.set(String::new());
        notice.set(String::new());
        spawn_local(async move {
            if let Err(e) = wizard.reload().await
                && month.try_get_untracked().as_ref() == Some(&wanted)
            {
                error.set(if e.is_empty() {
                    "Could not load the month closure.".into()
                } else {
                    e
                });
            }
        });
    });
    // A refresh (⌘R, or after a write) re-reads the month in place.
    Effect::new(move |previous: Option<()>| {
        app.revision.track();
        if previous.is_some() && !busy.get_untracked() {
            spawn_local(async move {
                let _ = wizard.reload().await;
            });
        }
    });

    let status = Memo::new(move |_| view.with(|v| v.as_ref().map(|v| v.period.status)));
    // A refresh keeps the open step; an action that moved the status forward
    // opens its new default step.
    Effect::new(move |before: Option<Option<ClosureStatus>>| {
        let now = status.get();
        if let (Some(Some(before)), Some(now)) = (before, now)
            && status_rank(now) > status_rank(before)
        {
            selected.set(None);
        }
        now
    });
    let step = Memo::new(move |_| {
        selected
            .get()
            .unwrap_or_else(|| status.get().map_or(0, default_step))
    });

    let month_id = field_id();
    let month_options: Vec<(String, String, bool)> = recent_months()
        .into_iter()
        .map(|m| {
            let label = month_label(&m, true);
            (m, label, false)
        })
        .collect();
    let stepper = STEP_NAMES
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let state = move || {
                let current = status.get().map_or(usize::MAX, step_index);
                if status.get().is_some() && index == step.get() {
                    StepState::Current
                } else if index < current && status.get().is_some() {
                    StepState::Done
                } else {
                    StepState::Upcoming
                }
            };
            view! {
                <li>
                    <button
                        type="button"
                        aria-current=move || (state() == StepState::Current).then_some("step")
                        data-state=move || match state() {
                            StepState::Current => "current",
                            StepState::Done => "done",
                            StepState::Upcoming => "upcoming",
                        }
                        class=move || step_class(state())
                        disabled=move || status.get().is_none()
                        on:click=move |_| wizard.go(index)
                    >
                        {format!("{}. {label}", index + 1)}
                    </button>
                </li>
            }
        })
        .collect_view();

    let body = move || {
        let v = view.get()?;
        let step = step.get();
        let report = app
            .report
            .data
            .with(|d| d.as_ref().and_then(|r| r.as_ref().ok()).cloned());
        let unclassified = unclassified.get();
        Some(step_view(wizard, v, step, report.as_ref(), unclassified))
    };
    let panels = move || {
        let v = view.get()?;
        let locations: Vec<(String, String)> = v
            .review
            .accounts
            .iter()
            .filter(|a| a.role != AccountRoleKind::Ignored)
            .map(|a| (a.account_id.clone(), a.name.clone()))
            .collect();
        Some(view! {
            <AccountsPanel accounts=v.accounts.clone() goals=v.goals.clone() />
            <PotsPanel view=v locations=locations />
        })
    };

    view! {
        <section class="grid min-w-0 gap-4" aria-label="Close month">
            <Card class="grid gap-4">
                <div class="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
                    <div>
                        <Eyebrow>"Month closure"</Eyebrow>
                        <h2 class="mt-1 text-2xl font-semibold tracking-tight">
                            {move || format!("Close {}", month_label(&month.get(), true))}
                        </h2>
                    </div>
                    <FieldBox label="Month".into_any() id=month_id.clone() class="sm:w-56">
                        <Choice
                            id=month_id
                            value=month
                            options=Signal::stored(month_options)
                            disabled=Signal::derive(move || busy.get())
                            on_change=Callback::new(move |m| month.set(m))
                        />
                    </FieldBox>
                </div>
                <ol class="flex flex-wrap gap-2">{stepper}</ol>
                {move || {
                    let n = notice.get();
                    (!n.is_empty()).then(|| view! { <p class="text-sm text-fg" role="status">{n}</p> })
                }}
                {move || {
                    let e = error.get();
                    (!e.is_empty()).then(|| view! { <p class=ERROR role="alert">{e}</p> })
                }}
                {move || (view.with(Option::is_none) && error.with(String::is_empty)).then(|| view! {
                    <p class="text-sm text-fg-muted" role="status">
                        {format!("Loading {}…", month_label(&month.get(), true))}
                    </p>
                })}
            </Card>
            {body}
            {panels}
        </section>
    }
}

/// The open step's card.
fn step_view(
    wizard: Wizard,
    view: ClosureView,
    step: usize,
    report: Option<&Dashboard>,
    unclassified: Option<UnclassifiedAccounts>,
) -> AnyView {
    let status = view.period.status;
    let month = wizard.month.get_untracked();
    let name = month_label(&month, false);
    let pots: Vec<PotView> = view.pots.iter().filter(|p| !p.archived).cloned().collect();
    let locations: Vec<(String, String)> = view
        .review
        .accounts
        .iter()
        .filter(|a| a.role != AccountRoleKind::Ignored)
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect();
    use ClosureStatus as S;
    match step {
        0 => {
            let description = match status {
                S::Open => Some(
                    "Import the latest transactions for every bank, then re-read the local data before reviewing the month."
                        .to_owned(),
                ),
                S::Settling => Some(format!(
                    "Synced{}. Sync again if new transactions arrived.",
                    if view.period.missing_banks.is_empty() {
                        String::new()
                    } else {
                        format!(" without {}", view.period.missing_banks.join(", "))
                    }
                )),
                _ => None,
            };
            let body = match status {
                S::Open | S::Settling => view! {
                    <BankSyncPanel
                        action="Continue"
                        disabled=Signal::derive(move || wizard.busy.get())
                        on_continue=Callback::new(move |missing| wizard.sync(missing))
                    />
                    {unclassified.map(|data| view! { <NewAccounts data=data goals=view.goals.clone() /> })}
                }
                .into_any(),
                S::Settled => locked(
                    wizard,
                    format!("The split is confirmed: the final sync that closes {name} runs from Transfers."),
                    Some(3),
                    None,
                ),
                S::Closed => reopen_here(month, format!("{name} is closed. Reopen {name} to sync it again.")),
                S::ChangedAfterClose => locked(
                    wizard,
                    format!("{name} changed after it closed. Assign the change in Split."),
                    Some(2),
                    None,
                ),
            };
            view! {
                <StepCard title="1. Sync banks" description=description>{body}</StepCard>
            }
            .into_any()
        }
        1 => {
            let names = review_names(report);
            view! {
                <StepCard title="2. Review">
                    {(status == S::Open).then(|| locked(wizard, "Sync banks first so the review is complete.".into(), Some(0), None))}
                    <ReviewStep review=view.review.clone() names=names />
                    {(status == S::Settling).then(|| view! {
                        <div class=ACTIONS>
                            <Button size=Size::Sm on_click=Callback::new(move |_| wizard.go(2))>"Continue to Split"</Button>
                        </div>
                    })}
                </StepCard>
            }
            .into_any()
        }
        2 if status == S::ChangedAfterClose => {
            view! { <ChangedAfterClose view=view pots=pots locations=locations /> }.into_any()
        }
        2 => {
            let title = format!(
                "3. {}",
                if status == S::Settled {
                    "Edit the split"
                } else {
                    "Split the leftover"
                }
            );
            let description = match status {
                S::Settling => Some("Assign every account's leftover to a pot, a planned purchase, a goal, the next-month buffer, or leave it where it is.".to_owned()),
                S::Settled => Some("Change where each leftover goes. Confirming replaces the split and its transfer list.".to_owned()),
                _ => None,
            };
            let body = match status {
                S::Open => locked(
                    wizard,
                    "Sync banks first, then split the leftover.".into(),
                    Some(0),
                    None,
                ),
                S::Settling | S::Settled => {
                    // Without a spending account the split has nothing to
                    // assign (e.g. every account set to "Don't track").
                    let has_spending = view
                        .review
                        .accounts
                        .iter()
                        .any(|a| a.role == AccountRoleKind::Spending);
                    let editor = if has_spending {
                        split_editor(wizard, &view, &month, pots, locations, report)
                    } else {
                        view! {
                            <EmptyState class="p-4" title="No accounts to split" description="Set account types in the Accounts panel below." />
                        }
                        .into_any()
                    };
                    view! {
                        {editor}
                        {(status == S::Settled).then(|| view! {
                            <div>
                                <Button variant=Variant::Ghost size=Size::Sm disabled=Signal::derive(move || wizard.busy.get()) on_click=Callback::new(move |_| wizard.go(3))>"Cancel"</Button>
                            </div>
                        })}
                    }
                    .into_any()
                }
                S::Closed => {
                    let split: Vec<AllocationLine> = view
                        .period
                        .allocations
                        .iter()
                        .filter(|l| l.kind == LineKind::Split)
                        .cloned()
                        .collect();
                    view! {
                        {reopen_here(month, format!("Reopen {name} to change the split."))}
                        <AllocationList lines=split view=view.clone() />
                    }
                    .into_any()
                }
                S::ChangedAfterClose => ().into_any(),
            };
            view! {
                <StepCard title=title description=description>{body}</StepCard>
            }
            .into_any()
        }
        _ if status == S::Closed => view! { <ClosedSummary view=view /> }.into_any(),
        _ => {
            let description = (status == S::Settled).then(|| {
                "The app never moves money. Make these transfers in your bank apps, then run the final sync to tick them off and close the month. A transfer made with another amount or split differently can be marked done by hand.".to_owned()
            });
            let allocations = view.period.allocations.clone();
            let body = match status {
                S::Open | S::Settling => {
                    let open = status == S::Open;
                    locked(
                        wizard,
                        "Confirm a split first.".into(),
                        Some(if open { 0 } else { 2 }),
                        Some(if open { "Go to Sync" } else { "Go to Split" }),
                    )
                }
                S::ChangedAfterClose => view! {
                    {locked(wizard, "Assign the change first; this is the split as it was closed.".into(), Some(2), None)}
                    <AllocationList lines=allocations view=view.clone() transfers_only=true />
                }
                .into_any(),
                _ => view! {
                    <AllocationList lines=allocations view=view.clone() transfers_only=true actions=true />
                    <div>
                        <Button variant=Variant::Secondary size=Size::Sm disabled=Signal::derive(move || wizard.busy.get()) on_click=Callback::new(move |_| wizard.go(2))>"Edit split"</Button>
                    </div>
                    <BankSyncPanel
                        action=format!("Close {name}")
                        disabled=Signal::derive(move || wizard.busy.get())
                        on_continue=Callback::new(move |missing| wizard.finalize(missing))
                    />
                }
                .into_any(),
            };
            view! {
                <StepCard title="4. Transfers" description=description>{body}</StepCard>
            }
            .into_any()
        }
    }
}

/// The first split or its edit.
fn split_editor(
    wizard: Wizard,
    view: &ClosureView,
    month: &str,
    pots: Vec<PotView>,
    locations: Vec<(String, String)>,
    report: Option<&Dashboard>,
) -> AnyView {
    let settled = view.period.status == ClosureStatus::Settled;
    let allocations = &view.period.allocations;
    // A deficit account is offered only to take from a pot (it has one, or a
    // pot holds its currency).
    let can_take = |account: &AccountReview| {
        allocations
            .iter()
            .any(|l| l.from_account_id == account.account_id && l.kind == LineKind::Split)
            || pots.iter().any(|p| {
                p.balances
                    .get(&account.currency.code)
                    .is_some_and(|b| *b > Decimal::ZERO)
            })
    };
    let targets: Vec<SplitTarget> = view
        .review
        .accounts
        .iter()
        .filter(|a| a.leftover > Decimal::ZERO || can_take(a))
        .map(|a| {
            let deficit = a.leftover <= Decimal::ZERO;
            SplitTarget {
                account_id: a.account_id.clone(),
                name: a.name.clone(),
                currency: a.currency.clone(),
                target: if deficit { Decimal::ZERO } else { a.leftover },
                note: if deficit {
                    Some("Deficit: nothing to split, but you can take from a pot.".into())
                } else if a.role == AccountRoleKind::Savings {
                    Some("Savings account".into())
                } else {
                    None
                },
            }
        })
        .collect();
    let categories: Vec<String> = report
        .map(|r| {
            r.category_catalog
                .entries
                .iter()
                .map(|e| e.name.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    let key = format!("{month}:{}:settle", view.period.revision);
    view! {
        <SplitEditor
            month=month.to_owned()
            pots=pots
            locations=locations
            committed=allocations.clone()
            suggestion=view.suggestion.clone()
            categories=categories
            can_create_pot=true
            goals=view.goals.clone()
            initial=if settled { allocations.clone() } else { Vec::new() }
            submit_label=if settled { "Confirm new split" } else { "Confirm split" }
            on_submit=Callback::new(move |lines| wizard.settle(lines))
            accounts=targets
            store_key=key
        />
    }
    .into_any()
}

/// Dashboard nudge: the month to close is open, or a closed month changed
/// afterwards.
#[component]
pub fn MonthClosureBanner() -> impl IntoView {
    let app = state();
    let banner = RwSignal::new(None::<(String, bool)>);
    spawn_local(async move {
        let target = month_to_close();
        let earlier_month = previous_month(&target);
        let current = load_closure(&target).await.ok();
        let earlier = load_closure(&earlier_month).await.ok();
        let changed = |v: &Option<ClosureView>| {
            v.as_ref()
                .is_some_and(|v| v.period.status == ClosureStatus::ChangedAfterClose)
        };
        let next = if changed(&earlier) {
            Some((earlier_month, true))
        } else if changed(&current) {
            Some((target, true))
        } else if current
            .as_ref()
            .is_some_and(|v| v.period.status != ClosureStatus::Closed)
        {
            Some((target, false))
        } else {
            None
        };
        let _ = banner.try_set(next);
    });
    move || {
        banner.get().map(|(month, changed)| {
            let name = month_label(&month, false);
            view! {
                <Card tone=CardTone::Muted class="mb-4 flex flex-wrap items-center justify-between gap-3 py-3 sm:py-3">
                    <p class="flex items-center gap-2 text-sm">
                        <span aria-hidden="true" class="inline-flex shrink-0 text-fg-muted"><Lucide icon=Icon::CalendarCheck size=16 /></span>
                        <strong>{format!("{name} {}", if changed { "changed after close" } else { "ready to close" })}</strong>
                    </p>
                    <Button size=Size::Sm on_click=Callback::new(move |_| {
                        WANTED_MONTH.with(|w| *w.borrow_mut() = Some(month.clone()));
                        app.go_manage(ManageSection::CloseMonth);
                    })>{if changed { "Review changes" } else { "Close month" }}</Button>
                </Card>
            }
        })
    }
}
