//! One-off extra income and payments of the budget month
//! (`budget-extras.tsx`): totals, the add form (→ `create_planned_payment`)
//! and the open entries with remove (→ `archive_planned_payment`).

use finance_types::gateway::{Direction, PlannedPayment, PlannedPaymentRequest};
use finance_types::report::Dashboard;
use finance_types::report::budget::MonthlyBudget;
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::invoke;
use crate::ds::charts::{BarTone, ProgressBar};
use crate::ds::sync_value;
use crate::ds::{
    CONTROL, Eyebrow, Heading, Icon, Lucide, Money, Size, Variant, button_class, cn, money,
};
use crate::format;
use crate::pages::f;
use crate::{AppState, state};

#[derive(Serialize)]
struct CreateArgs {
    request: PlannedPaymentRequest,
}

#[derive(Serialize)]
struct ArchiveArgs {
    plan_id: String,
}

/// The extras' state; it lives with the budget panel so a notice survives
/// the refresh that follows a save.
#[derive(Clone, Copy, Debug)]
pub struct ExtrasState {
    direction: RwSignal<Option<Direction>>,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    notice: RwSignal<String>,
    remove: RwSignal<Option<String>>,
    name: RwSignal<String>,
    amount: RwSignal<String>,
    account_id: RwSignal<String>,
    match_category: RwSignal<String>,
    due_on: RwSignal<String>,
    reimbursement_for: RwSignal<String>,
}

impl ExtrasState {
    pub fn new() -> Self {
        Self {
            direction: RwSignal::new(None),
            busy: RwSignal::new(false),
            error: RwSignal::new(String::new()),
            notice: RwSignal::new(String::new()),
            remove: RwSignal::new(None),
            name: RwSignal::new(String::new()),
            amount: RwSignal::new(String::new()),
            account_id: RwSignal::new(String::new()),
            match_category: RwSignal::new(String::new()),
            due_on: RwSignal::new(String::new()),
            reimbursement_for: RwSignal::new(String::new()),
        }
    }

    /// Forget everything (another month was picked).
    pub fn reset(&self) {
        self.direction.set(None);
        self.error.set(String::new());
        self.notice.set(String::new());
        self.remove.set(None);
    }

    fn open(&self, direction: Direction, default_due: &str) {
        self.direction.set(Some(direction));
        self.error.set(String::new());
        for field in [
            self.name,
            self.amount,
            self.account_id,
            self.match_category,
            self.reimbursement_for,
        ] {
            field.set(String::new());
        }
        self.due_on.set(default_due.to_owned());
    }

    fn finish(self, app: AppState, result: Result<(), String>, archived: bool) {
        match result {
            Ok(()) => {
                self.direction.set(None);
                self.remove.set(None);
                self.notice.set(if archived {
                    "Entry removed. Recorded transactions are kept.".into()
                } else {
                    "One-off entry saved.".into()
                });
                app.refresh();
            }
            Err(e) => self.error.set(if e.is_empty() {
                "Could not save this entry.".into()
            } else {
                e
            }),
        }
        self.busy.set(false);
    }

    fn start(&self) -> bool {
        if self.busy.get_untracked() {
            return false;
        }
        self.busy.set(true);
        self.error.set(String::new());
        self.notice.set(String::new());
        true
    }

    fn archive(self, plan_id: String) {
        if !self.start() {
            return;
        }
        let app = state();
        spawn_local(async move {
            let result = invoke::<()>("archive_planned_payment", &ArchiveArgs { plan_id }).await;
            self.finish(app, result, true);
        });
    }

    fn create(self, request: PlannedPaymentRequest) {
        if !self.start() {
            return;
        }
        let app = state();
        spawn_local(async move {
            let result =
                invoke::<PlannedPayment>("create_planned_payment", &CreateArgs { request })
                    .await
                    .map(|_| ());
            self.finish(app, result, false);
        });
    }
}

fn is_salary(category: &str) -> bool {
    category.trim().to_lowercase() == "salary"
}

#[component]
pub fn BudgetExtras(
    report: Dashboard,
    budget: MonthlyBudget,
    available: Option<Decimal>,
    state: ExtrasState,
) -> impl IntoView {
    let s = state;
    let month = budget.plan.month[..7].to_owned();
    let unit = budget.plan.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let through = report.monthly_spending.through.clone();
    let month_end = format::month_end(&month);
    let is_current = budget.plan.month == format!("{}-01", &through[..7]);
    let plans: Vec<PlannedPaymentView> = report
        .planned_payments
        .iter()
        .filter(|p| {
            !(p.direction == Direction::Inflow && is_salary(&p.match_category))
                && p.amount.currency.code == unit.code
                && (p.due_on.starts_with(&month)
                    || (is_current
                        && p.pending
                        && p.status != PlanStatus::Paid
                        && p.due_on < budget.plan.month))
        })
        .cloned()
        .collect();
    let mut categories: Vec<String> = report
        .category_catalog
        .entries
        .iter()
        .map(|e| e.name.clone())
        .chain(
            report
                .monthly_spending
                .mappings
                .iter()
                .map(|e| e.category.clone()),
        )
        .chain(
            report
                .planned_payments
                .iter()
                .map(|p| p.match_category.clone()),
        )
        .collect();
    categories.sort();
    categories.dedup();
    let totals = |direction: Direction, completed: bool| -> Decimal {
        plans
            .iter()
            .filter(|p| p.direction == direction && (p.status == PlanStatus::Paid) == completed)
            .map(|p| p.amount.amount)
            .sum()
    };
    let expected = totals(Direction::Inflow, false);
    let default_due = if through.starts_with(&month) {
        through.clone()
    } else {
        budget.plan.month.clone()
    };

    let summaries = [Direction::Inflow, Direction::Outflow]
        .into_iter()
        .map(|direction| {
            let inflow = direction == Direction::Inflow;
            let completed = totals(direction, true);
            let outstanding = totals(direction, false);
            let total = completed + outstanding;
            view! {
                <div class="grid gap-2">
                    <span class=cn([
                        "flex items-center gap-1.5 text-sm",
                        if inflow { "text-positive-fg" } else { "text-warning-fg" },
                    ])>
                        <Lucide icon=if inflow { Icon::ArrowDownLeft } else { Icon::ArrowUpRight } size=16 />
                        {if inflow { "Extra income" } else { "Extra payments" }}
                    </span>
                    <strong class="text-xl font-medium tabular-nums">{m(total)}</strong>
                    <ProgressBar
                        label=if inflow { "Received" } else { "Paid" }
                        value=f(completed)
                        max=f(total)
                        tone=if inflow { BarTone::Positive } else { BarTone::Warning }
                    />
                    <small class="text-xs text-fg-muted">
                        {format!(
                            "{} {} · {} {}",
                            m(completed),
                            if inflow { "received" } else { "paid" },
                            m(outstanding),
                            if inflow { "expected" } else { "remaining" },
                        )}
                    </small>
                </div>
            }
        })
        .collect_view();

    let projected = (expected > Decimal::ZERO)
        .then_some(available)
        .flatten()
        .map(|a| {
            view! {
                <div class="my-3 grid gap-4 rounded-md bg-card-muted p-4 text-xs text-fg-muted sm:grid-cols-3">
                    <span>
                        "Projected available in this plan"
                        <Money
                            value=a.round_dp(u32::from(unit.minor_unit))
                            unit=unit.clone()
                            neutral=true
                            class="mt-1.5 block text-base font-medium text-fg"
                        />
                    </span>
                    <span>"Upcoming extra income is already included in the allocation above."</span>
                </div>
            }
        });

    let open: Vec<PlannedPaymentView> = plans
        .into_iter()
        .filter(|p| p.status != PlanStatus::Paid)
        .collect();
    let entries = if open.is_empty() {
        view! { <p class="text-xs text-fg-muted">"No extra income or payments planned this month."</p> }.into_any()
    } else {
        view! {
            <ul class="mt-3 list-none p-0">
                {open.into_iter().map(|p| entry(p, s)).collect_view()}
            </ul>
        }
        .into_any()
    };

    let accounts: Vec<(String, String)> = report
        .cash_overview
        .accounts
        .iter()
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect();
    let reimbursable: Vec<(String, String)> = budget
        .purchases
        .iter()
        .filter(|p| p.amount > Decimal::ZERO)
        .map(|p| (p.id.clone(), format!("{} · {}", p.description, m(p.amount))))
        .collect();
    let form_data = StoredValue::new((
        accounts,
        categories,
        reimbursable,
        unit.clone(),
        budget.plan.month.clone(),
        month_end,
    ));
    let list_id = format!("extra-categories-{}", unit.code);
    let form = move || {
        let direction = s.direction.get()?;
        let (accounts, categories, reimbursable, unit, min_due, max_due) = form_data.get_value();
        let inflow = direction == Direction::Inflow;
        let step = Decimal::new(1, u32::from(unit.minor_unit)).to_string();
        let title = if inflow {
            "Add extra income"
        } else {
            "Add extra payment"
        };
        let code = unit.code.clone();
        let list_id = list_id.clone();
        let reimbursement = inflow.then(|| {
            view! {
                <div class="grid gap-1.5">
                    <label for="extra-reimbursement" class="text-sm font-medium text-fg">"Reimbursement for"</label>
                    <div class="relative w-full">
                        <select
                            id="extra-reimbursement"
                            aria-describedby="extra-reimbursement-hint"
                            class=cn([CONTROL, "appearance-none pr-9"])
                            on:change=move |e| s.reimbursement_for.set(event_target_value(&e))
                        >
                            <option value="" selected=true>"General income"</option>
                            {reimbursable
                                .into_iter()
                                .map(|(id, label)| view! { <option value=id>{label}</option> })
                                .collect_view()}
                        </select>
                        <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                            <Lucide icon=Icon::ChevronDown size=16 />
                        </span>
                    </div>
                    <p id="extra-reimbursement-hint" class="text-xs text-fg-muted">
                        "Optional. Once received, this reduces the purchase's net spending."
                    </p>
                </div>
            }
        });
        Some(view! {
            <form
                class="mt-4 rounded-lg border border-border p-5"
                aria-label=title
                on:submit=move |e| {
                    e.prevent_default();
                    let category = s.match_category.get_untracked();
                    if inflow && is_salary(&category) {
                        s.error.set("Salary is managed in Plan. Choose a category for extra income.".into());
                        return;
                    }
                    let reimbursement = s.reimbursement_for.get_untracked();
                    s.create(PlannedPaymentRequest {
                        name: s.name.get_untracked().trim().to_owned(),
                        match_category: category.trim().to_owned(),
                        direction,
                        amount: s.amount.get_untracked(),
                        currency: code.clone(),
                        due_on: s.due_on.get_untracked(),
                        account_id: s.account_id.get_untracked(),
                        reimbursement_for: (inflow && !reimbursement.is_empty()).then_some(reimbursement),
                        match_description: None,
                    });
                }
            >
                <div class="flex items-center justify-between gap-3">
                    <h3 class="text-base font-semibold">{title}</h3>
                    <button
                        type="button"
                        aria-label="Close extra entry"
                        class=button_class(Variant::Ghost, Size::IconSm)
                        disabled=move || s.busy.get()
                        on:click=move |_| s.direction.set(None)
                    >
                        <Lucide icon=Icon::X size=16 />
                    </button>
                </div>
                <fieldset disabled=move || s.busy.get() class="m-0 min-w-0 border-0 p-0">
                    <div class="my-4 grid gap-4 sm:grid-cols-2">
                        <div class="grid gap-1.5">
                            <label for="extra-name" class="text-sm font-medium text-fg">"Name"</label>
                            <input
                                id="extra-name"
                                class=CONTROL
                                autofocus=true
                                required=true
                                maxlength="128"
                                placeholder=if inflow { "e.g. Sold a bicycle" } else { "e.g. Bicycle repair" }
                                prop:value=move || s.name.get()
                                on:input=move |e| s.name.set(event_target_value(&e))
                            />
                        </div>
                        <div class="grid gap-1.5">
                            <label for="extra-amount" class="text-sm font-medium text-fg">{format!("Amount ({})", unit.code)}</label>
                            <input
                                id="extra-amount"
                                class=CONTROL
                                type="number"
                                required=true
                                min=step.clone()
                                step=step
                                placeholder="0.00"
                                node_ref={
                                    let node = NodeRef::<leptos::html::Input>::new();
                                    sync_value(node, move || s.amount.get());
                                    node
                                }
                                on:input=move |e| s.amount.set(event_target_value(&e))
                            />
                        </div>
                        <div class="grid gap-1.5">
                            <label for="extra-account" class="text-sm font-medium text-fg">"Account"</label>
                            <div class="relative w-full">
                                <select
                                    id="extra-account"
                                    required=true
                                    class=cn([CONTROL, "appearance-none pr-9"])
                                    on:change=move |e| s.account_id.set(event_target_value(&e))
                                >
                                    <option value="" disabled=true selected=true>"Choose account"</option>
                                    {accounts
                                        .into_iter()
                                        .map(|(id, name)| view! { <option value=id>{name}</option> })
                                        .collect_view()}
                                </select>
                                <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                                    <Lucide icon=Icon::ChevronDown size=16 />
                                </span>
                            </div>
                        </div>
                        <div class="grid gap-1.5">
                            <label for="extra-category" class="text-sm font-medium text-fg">"Transaction category"</label>
                            <input
                                id="extra-category"
                                class=CONTROL
                                list=list_id.clone()
                                required=true
                                maxlength="128"
                                placeholder="Choose or type a category"
                                prop:value=move || s.match_category.get()
                                on:input=move |e| s.match_category.set(event_target_value(&e))
                            />
                        </div>
                        <datalist id=list_id>
                            {categories.into_iter().map(|c| view! { <option value=c></option> }).collect_view()}
                        </datalist>
                        <div class="grid gap-1.5">
                            <label for="extra-due" class="text-sm font-medium text-fg">
                                {if inflow { "Expected date" } else { "Due date" }}
                            </label>
                            <input
                                id="extra-due"
                                class=CONTROL
                                type="date"
                                required=true
                                min=min_due
                                max=max_due
                                prop:value=move || s.due_on.get()
                                on:input=move |e| s.due_on.set(event_target_value(&e))
                            />
                        </div>
                        {reimbursement}
                    </div>
                    <p class="text-xs text-fg-muted">
                        {format!(
                            "One-off only. A bank transaction with the same category and amount updates this entry to {}.",
                            if inflow { "received" } else { "paid" },
                        )}
                    </p>
                    <div class="mt-5 flex flex-col items-start justify-between gap-3 border-t border-border pt-4 sm:flex-row sm:items-center">
                        <small class="text-xs text-fg-muted">
                            {if inflow {
                                "Expected income stays separate until received."
                            } else {
                                "Payments reserve money without duplicating a matching budget allowance."
                            }}
                        </small>
                        <button type="submit" class=button_class(Variant::Primary, Size::Md)>
                            {move || if s.busy.get() { "Saving…" } else { "Save entry" }}
                        </button>
                    </div>
                </fieldset>
            </form>
        })
    };

    let opener = move |direction: Direction| {
        let due = default_due.clone();
        move |_| s.open(direction, &due)
    };
    let locked = move || s.busy.get() || s.direction.with(Option::is_some);
    view! {
        <section class="border-b border-border p-5 sm:px-7 sm:py-6" aria-label="Extra income and payments">
            <header class="flex flex-col items-start justify-between gap-3 sm:flex-row sm:items-center">
                <div>
                    <Eyebrow level=Heading::P>"THIS MONTH ONLY"</Eyebrow>
                    <h3 class="mb-3.5 mt-1.5 text-base font-medium">"Extra income & payments"</h3>
                </div>
                <div class="flex flex-wrap gap-2">
                    <button
                        type="button"
                        class=button_class(Variant::Secondary, Size::Sm)
                        disabled=locked
                        on:click=opener(Direction::Inflow)
                    >
                        <Lucide icon=Icon::Plus size=14 />
                        " Extra income"
                    </button>
                    <button
                        type="button"
                        class=button_class(Variant::Secondary, Size::Sm)
                        disabled=locked
                        on:click=opener(Direction::Outflow)
                    >
                        <Lucide icon=Icon::Plus size=14 />
                        " Extra payment"
                    </button>
                </div>
            </header>
            <div class="grid gap-6 py-4 sm:grid-cols-2">{summaries}</div>
            {projected}
            {move || {
                let error = s.error.get();
                (!error.is_empty()).then(|| view! { <p role="alert" class="my-3 text-sm text-negative-fg">{error}</p> })
            }}
            {move || {
                let notice = s.notice.get();
                (!notice.is_empty()).then(|| view! { <p role="status" class="my-3 text-sm">{notice}</p> })
            }}
            {form}
            {entries}
        </section>
    }
}

fn entry(plan: PlannedPaymentView, s: ExtrasState) -> impl IntoView {
    let inflow = plan.direction == Direction::Inflow;
    let unit = plan.amount.currency.clone();
    let status = match plan.status {
        PlanStatus::NeedsReview => "Check possible matches in Plan",
        PlanStatus::Overdue => "Overdue",
        _ if inflow => "Expected",
        _ => "Upcoming",
    };
    let detail = format!("{} · {status}", format::day_month(&plan.due_on));
    let amount = format!(
        "{} {}",
        if inflow { "+" } else { "−" },
        money(plan.amount.amount, &unit)
    );
    let id = plan.plan_id.clone();
    let confirming = {
        let id = id.clone();
        move || s.remove.get().as_deref() == Some(id.as_str())
    };
    let label = format!("Remove {}", plan.name);
    view! {
        <li class="flex flex-wrap items-center gap-3 border-t border-border py-3 text-sm">
            <span class=if inflow { "text-positive-fg" } else { "text-warning-fg" }>
                <Lucide icon=if inflow { Icon::ArrowDownLeft } else { Icon::ArrowUpRight } size=16 />
            </span>
            <span class="min-w-0 flex-1">
                <strong class="font-medium">{plan.name.clone()}</strong>
                <small class="mt-1 block text-xs text-fg-muted">{detail}</small>
            </span>
            <strong class="font-medium tabular-nums">{amount}</strong>
            {move || {
                let id = id.clone();
                if confirming() {
                    view! {
                        <div class="flex gap-2">
                            <button
                                type="button"
                                class=button_class(Variant::Danger, Size::Sm)
                                disabled=move || s.busy.get()
                                on:click=move |_| s.archive(id.clone())
                            >
                                "Confirm remove"
                            </button>
                            <button
                                type="button"
                                class=button_class(Variant::Secondary, Size::Sm)
                                disabled=move || s.busy.get()
                                on:click=move |_| s.remove.set(None)
                            >
                                "Keep"
                            </button>
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <button
                            type="button"
                            aria-label=label.clone()
                            class=button_class(Variant::Ghost, Size::IconSm)
                            disabled=move || s.busy.get()
                            on:click=move |_| s.remove.set(Some(id.clone()))
                        >
                            <Lucide icon=Icon::X size=14 />
                        </button>
                    }
                    .into_any()
                }
            }}
        </li>
    }
}
