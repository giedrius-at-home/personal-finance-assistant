//! The Plan section, "Plans & bills" (personal-site `owner-fixed-spending.tsx`):
//! the payday, one-off plans for this or next month, and the bills with
//! their editor, automatic matching, schedule and suggested payments.

use std::collections::HashMap;

use finance_types::gateway::PlannedPaymentRequest;
use finance_types::gateway::{
    BILL_FREQUENCIES, BillCreateRequest, BillEditRequest, BillMatchRequest, BillRef, BillSaved,
    Direction, SubscriptionCategoriesRequest, SubscriptionCategoryRule,
};
use finance_types::report::Dashboard;
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::subscriptions::{
    BillLinkCandidate, ScheduledBillPayment, SubscriptionRow,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::IgnoredAny;

use crate::api::invoke;
use crate::ds::money_plan::MoneyPlanFields;
use crate::ds::sync_value;
use crate::ds::{
    Badge, Button, CONTROL, Card, CardTone, Eyebrow, Facts, FactsRow, Field, Help, Icon, Lucide,
    Money, MonthSwitcher, Size, Tone, Variant, card_class, cn,
};
use crate::pages::with_report;
use crate::{ManageSection, format, state};

/// The single argument of the write commands.
#[derive(Serialize)]
struct Request<T: Serialize> {
    request: T,
}

/// `archive_planned_payment(plan_id)`.
#[derive(Serialize)]
struct PlanIdArgs {
    plan_id: String,
}

/// Screen state that outlives a dashboard reload.
#[derive(Clone, Copy)]
struct Ui {
    app: crate::AppState,
    editing: RwSignal<Option<String>>,
    edit_busy: RwSignal<bool>,
    /// `bill:transaction` pairs being confirmed or declined.
    saving: RwSignal<Vec<String>>,
    /// Feedback per target: (text, is error).
    messages: RwSignal<HashMap<String, (String, bool)>>,
    plan_busy: RwSignal<bool>,
    plan_action: RwSignal<String>,
    removed_plans: RwSignal<Vec<String>>,
    removed_bills: RwSignal<Vec<String>>,
    linked: RwSignal<Vec<String>>,
    repeat: RwSignal<String>,
    direction: RwSignal<String>,
    removing_bill: RwSignal<Option<String>>,
    category_busy: RwSignal<Option<String>>,
    archive_id: RwSignal<Option<String>>,
    next_month: RwSignal<bool>,
    name: RwSignal<String>,
    match_category: RwSignal<String>,
    account_id: RwSignal<String>,
    amount: RwSignal<String>,
    currency: RwSignal<String>,
    due_on: RwSignal<String>,
}

impl Ui {
    fn new() -> Self {
        Self {
            app: state(),
            editing: RwSignal::new(None),
            edit_busy: RwSignal::new(false),
            saving: RwSignal::new(Vec::new()),
            messages: RwSignal::new(HashMap::new()),
            plan_busy: RwSignal::new(false),
            plan_action: RwSignal::new(String::new()),
            removed_plans: RwSignal::new(Vec::new()),
            removed_bills: RwSignal::new(Vec::new()),
            linked: RwSignal::new(Vec::new()),
            repeat: RwSignal::new("once".into()),
            direction: RwSignal::new("outflow".into()),
            removing_bill: RwSignal::new(None),
            category_busy: RwSignal::new(None),
            archive_id: RwSignal::new(None),
            next_month: RwSignal::new(false),
            name: RwSignal::new(String::new()),
            match_category: RwSignal::new(String::new()),
            account_id: RwSignal::new(String::new()),
            amount: RwSignal::new(String::new()),
            currency: RwSignal::new(String::new()),
            due_on: RwSignal::new(String::new()),
        }
    }

    fn set_message(self, target: &str, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.messages.update(|m| {
            m.insert(target.to_owned(), (text, error));
        });
    }

    /// A confirmed write: show its receipt and reload every screen.
    fn saved(self, target: &str, text: impl Into<String>) {
        self.set_message(target, text, false);
        self.app.refresh();
    }

    fn reset_form(self) {
        for field in [
            self.name,
            self.match_category,
            self.account_id,
            self.amount,
            self.due_on,
        ] {
            field.set(String::new());
        }
        self.currency.set(String::new());
    }
}

/// The receipt or error of `target`, when there is one.
fn feedback(ui: Ui, target: String) -> impl IntoView {
    move || {
        ui.messages.with(|m| {
            m.get(&target).filter(|(text, _)| !text.is_empty()).map(|(text, error)| {
                let class = cn([
                    "mt-2 break-words text-sm",
                    if *error { "text-negative-fg" } else { "text-positive-fg" },
                ]);
                view! { <p role=if *error { "alert" } else { "status" } class=class>{text.clone()}</p> }
            })
        })
    }
}

/// Like [`feedback`], inside a wrapper that only exists while there is one.
fn feedback_in(ui: Ui, target: String, class: &'static str) -> impl IntoView {
    move || {
        let has = ui
            .messages
            .with(|m| m.get(&target).is_some_and(|(text, _)| !text.is_empty()));
        has.then(|| view! { <div class=class>{feedback(ui, target.clone())}</div> })
    }
}

/// Categories of the bills' suggested payments.
fn candidate_categories(report: &Dashboard) -> impl Iterator<Item = String> + '_ {
    report
        .subscriptions
        .rows
        .iter()
        .flat_map(|r| r.link_candidates.iter().filter_map(|c| c.category.clone()))
}

/// The categories a new plan or bill can match, sorted.
pub(crate) fn plan_categories(report: &Dashboard) -> Vec<String> {
    unique_categories(
        report
            .category_catalog
            .entries
            .iter()
            .map(|e| e.name.clone())
            .chain(
                report
                    .monthly_spending
                    .mappings
                    .iter()
                    .map(|m| m.category.clone()),
            )
            .chain(candidate_categories(report))
            .chain(
                report
                    .planned_payments
                    .iter()
                    .map(|p| p.match_category.clone()),
            ),
    )
}

/// Distinct (case-insensitively) trimmed names, sorted.
fn unique_categories(categories: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut by_name: HashMap<String, String> = HashMap::new();
    let mut order = Vec::new();
    for category in categories {
        let cleaned = category.trim().to_owned();
        let key = cleaned.to_lowercase();
        if !cleaned.is_empty() && !by_name.contains_key(&key) {
            by_name.insert(key.clone(), cleaned);
            order.push(key);
        }
    }
    let mut out: Vec<String> = order
        .into_iter()
        .filter_map(|k| by_name.remove(&k))
        .collect();
    out.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    out
}

fn select_wrap(select: impl IntoView) -> impl IntoView {
    view! {
        <div class="relative w-full">
            {select}
            <span aria-hidden="true" class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

fn select_class() -> String {
    cn([CONTROL, "appearance-none pr-9"])
}

#[component]
pub fn Plan() -> impl IntoView {
    let ui = Ui::new();
    with_report(move |report| page(ui, report))
}

fn page(ui: Ui, report: &Dashboard) -> impl IntoView + use<> {
    let next = ui.next_month.get() && report.next_month_subscriptions.is_some();
    let active = if next {
        report
            .next_month_subscriptions
            .clone()
            .unwrap_or_else(|| report.subscriptions.clone())
    } else {
        report.subscriptions.clone()
    };
    let removed_bills = ui.removed_bills.get();
    let rows: Vec<SubscriptionRow> = active
        .rows
        .iter()
        .filter(|b| !removed_bills.contains(&b.bill_id))
        .cloned()
        .collect();
    let month_of = |date: &str| date.get(..7).unwrap_or(date).to_owned();
    let active_month = month_of(&active.through_date);
    let active_label = format::month_long(&active_month);
    let current_month = month_of(&report.subscriptions.through_date);
    let next_month = report
        .next_month_subscriptions
        .as_ref()
        .map_or_else(|| current_month.clone(), |n| month_of(&n.through_date));
    let active_end = format::month_end(&active_month);
    let removed_plans = ui.removed_plans.get();
    let month_start = format!("{active_month}-01");
    let plans: Vec<PlannedPaymentView> = report
        .planned_payments
        .iter()
        .filter(|p| {
            !removed_plans.contains(&p.plan_id)
                && (p.due_on.starts_with(&active_month)
                    || (!next
                        && p.pending
                        && p.status != PlanStatus::Paid
                        && p.due_on < month_start))
        })
        .cloned()
        .collect();
    let mut currencies: Vec<String> = Vec::new();
    for c in &report.net_worth.components {
        if !currencies.contains(&c.currency.code) {
            currencies.push(c.currency.code.clone());
        }
    }
    let available = plan_categories(report);
    let subscription_categories = unique_categories(
        report
            .monthly_spending
            .mappings
            .iter()
            .map(|m| m.category.clone())
            .chain(
                report
                    .subscriptions
                    .rows
                    .iter()
                    .flat_map(|r| r.category_match_categories.clone()),
            )
            .chain(candidate_categories(report)),
    );
    // Lower-cased category → (bill id, bill name).
    let mut owners: HashMap<String, (String, String)> = HashMap::new();
    for row in &report.subscriptions.rows {
        for category in &row.category_match_categories {
            owners.insert(
                category.to_lowercase(),
                (row.bill_id.clone(), row.name.clone()),
            );
        }
    }
    let accounts: Vec<(String, String)> = report
        .cash_overview
        .accounts
        .iter()
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect();
    let stale = active.stale;
    let scheduled = active.scheduled_payments.clone();
    let shared = StoredValue::new(Shared {
        active_month: active_month.clone(),
        stale,
        scheduled,
        subscription_categories,
        owners: owners.clone(),
    });

    let payday = report.cash_overview.payday_status.clone().map(|payday| {
        let body = match &payday.received_on {
            Some(day) => {
                let amounts = payday
                    .received_amounts
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        view! {
                            <span>
                                {(i > 0).then_some(" · ")}
                                <Money value=a.amount unit=a.currency.clone() />
                            </span>
                        }
                    })
                    .collect_view();
                view! {
                    <strong class="mt-1 block text-2xl font-semibold">{amounts}</strong>
                    <Facts class="mt-2" rows=vec![("Received".to_owned(), day.clone())] />
                }
                .into_any()
            }
            None => view! { <Facts class="mt-2" rows=vec![("Expected".to_owned(), payday.due_on.clone())] /> }.into_any(),
        };
        let title = if payday.received_on.is_some() {
            "Salary received"
        } else {
            "Salary not recorded yet"
        };
        let plan_fields = report.money_plan.clone().map(|plan| {
            let names: HashMap<String, String> = accounts.iter().cloned().collect();
            view! {
                <div class="mt-4 border-t border-line pt-4">
                    <Eyebrow>"WHERE THE MONEY GOES"</Eyebrow>
                    <div class="mt-3"><MoneyPlanFields plan=plan names=names /></div>
                </div>
            }
        });
        view! {
            <Card>
                <Eyebrow>"MONTHLY PAYDAY"</Eyebrow>
                <h2 class="mt-2 text-lg font-semibold">{title}</h2>
                {body}
                {plan_fields}
            </Card>
        }
    });

    let switcher_month = if next {
        next_month.clone()
    } else {
        current_month.clone()
    };
    let current_for_switch = current_month.clone();
    let on_month = Callback::new(move |month: String| {
        ui.next_month.set(month != current_for_switch);
        // The due date belongs to the month shown (the form keyed it).
        ui.due_on.set(String::new());
    });

    let no_plans = format!("No plans for {active_label}.");
    let plans_empty = plans.is_empty();
    let plan_rows = plans
        .into_iter()
        .map(|plan| plan_row(ui, plan))
        .collect_view();
    let no_bills = rows.is_empty();
    let bill_cards = rows
        .into_iter()
        .map(|bill| view! { <BillCard ui=ui bill=bill shared=shared /> })
        .collect_view();

    view! {
        <div class="grid gap-4">
            {payday}
            <div role="group" aria-label="Planning month" class="justify-self-start">
                <MonthSwitcher
                    month=Signal::derive(move || switcher_month.clone())
                    on_change=on_month
                    min=current_month.clone()
                    max=next_month.clone()
                />
            </div>
            <Card>
                <Eyebrow>"PLANS"</Eyebrow>
                <h2 class="mt-2 text-lg font-semibold">{active_label.clone()}</h2>
                {create_form(ui, &available, &owners, &accounts, &currencies, &active_month, &active_end)}
                <div class="border-t border-line">
                    {feedback(ui, "plan-removed".into())}
                    {plan_rows}
                    {plans_empty.then(|| view! { <p class="mt-3 text-sm text-fg-muted">{no_plans}</p> })}
                </div>
            </Card>
            <Card>
                <Eyebrow>"BILLS"</Eyebrow>
                <h2 class="mt-2 text-lg font-semibold">"Fixed spending"</h2>
                {stale.then(|| view! {
                    <p role="status" class="mt-2 text-sm text-warning-fg">
                        "Bill information is stale. Refresh reports before linking payments."
                    </p>
                })}
                {feedback(ui, "bill-removed".into())}
            </Card>
            {no_bills.then(|| view! { <p>"No bills this month."</p> })}
            <div class="grid gap-4">{bill_cards}</div>
        </div>
    }
}

/// What every bill card reads from the report.
struct Shared {
    active_month: String,
    stale: bool,
    scheduled: Vec<ScheduledBillPayment>,
    subscription_categories: Vec<String>,
    owners: HashMap<String, (String, String)>,
}

fn create_form(
    ui: Ui,
    available: &[String],
    owners: &HashMap<String, (String, String)>,
    accounts: &[(String, String)],
    currencies: &[String],
    active_month: &str,
    active_end: &str,
) -> impl IntoView + use<> {
    let first_currency = currencies.first().cloned().unwrap_or_default();
    let has_currencies = !currencies.is_empty();
    let currency = {
        let first = first_currency.clone();
        move || {
            let chosen = ui.currency.get();
            if chosen.is_empty() {
                first.clone()
            } else {
                chosen
            }
        }
    };
    let submit = {
        let first = first_currency.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            if ui.plan_busy.get_untracked() {
                return;
            }
            let currency = match ui.currency.get_untracked() {
                c if c.is_empty() => first.clone(),
                c => c,
            };
            let Ok(amount) = ui.amount.get_untracked().trim().parse::<Decimal>() else {
                ui.set_message("create", "Enter an amount.", true);
                return;
            };
            let repeat = ui.repeat.get_untracked();
            let name = ui.name.get_untracked().trim().to_owned();
            let match_category = ui.match_category.get_untracked().trim().to_owned();
            let due_on = ui.due_on.get_untracked();
            ui.plan_busy.set(true);
            ui.plan_action.set("create".into());
            ui.set_message("create", "", false);
            if repeat == "once" {
                let request = PlannedPaymentRequest {
                    name,
                    match_category,
                    direction: if ui.direction.get_untracked() == "inflow" {
                        Direction::Inflow
                    } else {
                        Direction::Outflow
                    },
                    amount: amount.to_string(),
                    currency,
                    due_on,
                    account_id: ui.account_id.get_untracked(),
                    reimbursement_for: None,
                    match_description: None,
                };
                spawn_local(async move {
                    match invoke::<IgnoredAny>("create_planned_payment", &Request { request }).await
                    {
                        Ok(_) => {
                            ui.saved(
                                "create",
                                "Plan saved. Matching payments will update its status.",
                            );
                            ui.archive_id.set(None);
                            ui.reset_form();
                        }
                        Err(e) => ui.set_message(
                            "create",
                            error_or(e, "Could not save planned payment."),
                            true,
                        ),
                    }
                    ui.plan_busy.set(false);
                });
            } else {
                let request = BillCreateRequest {
                    name,
                    match_category,
                    amount_min: amount,
                    amount_max: amount,
                    currency_code: currency,
                    date: due_on,
                    repeat_freq: repeat,
                    skip: 0,
                    end_date: None,
                };
                spawn_local(async move {
                    match invoke::<BillSaved>("create_bill", &Request { request }).await {
                        Ok(_) => {
                            ui.removing_bill.set(None);
                            ui.saved("create", "Recurring plan added.");
                            ui.reset_form();
                        }
                        Err(e) => ui.set_message(
                            "create",
                            error_or(e, "Could not save recurring plan."),
                            true,
                        ),
                    }
                    ui.plan_busy.set(false);
                });
            }
        }
    };
    let frequency_options = std::iter::once(("once".to_owned(), "Once".to_owned()))
        .chain(
            BILL_FREQUENCIES
                .iter()
                .map(|f| ((*f).to_owned(), (*f).to_owned())),
        )
        .map(|(value, label)| {
            let selected = value == ui.repeat.get_untracked();
            view! { <option value=value selected=selected>{label}</option> }
        })
        .collect_view();
    let category_options = available
        .iter()
        .map(|category| {
            let owned = owners.contains_key(&category.to_lowercase());
            let selected = *category == ui.match_category.get_untracked();
            view! {
                <option
                    value=category.clone()
                    selected=selected
                    disabled=move || owned && ui.repeat.get() != "once"
                >
                    {category.clone()}
                </option>
            }
        })
        .collect_view();
    let account_options = accounts
        .iter()
        .map(|(id, name)| {
            let selected = *id == ui.account_id.get_untracked();
            view! { <option value=id.clone() selected=selected>{name.clone()}</option> }
        })
        .collect_view();
    let currency_options = currencies
        .iter()
        .map(|code| view! { <option value=code.clone()>{code.clone()}</option> })
        .collect_view();
    let min = format!("{active_month}-01");
    let max = active_end.to_owned();
    view! {
        <form class="my-5 grid items-end gap-3 sm:grid-cols-2 lg:grid-cols-3" on:submit=submit>
            <Field label="Repeat" id="plan-repeat">
                {select_wrap(view! {
                    <select
                        id="plan-repeat"
                        aria-label="Plan frequency"
                        class=select_class()
                        prop:value=move || ui.repeat.get()
                        on:change=move |e| {
                            let value = event_target_value(&e);
                            if value != "once" {
                                ui.direction.set("outflow".into());
                            }
                            ui.repeat.set(value);
                        }
                    >
                        {frequency_options}
                    </select>
                })}
            </Field>
            <Field label="Type" id="plan-direction">
                {select_wrap(view! {
                    <select
                        id="plan-direction"
                        name="direction"
                        class=select_class()
                        disabled=move || ui.repeat.get() != "once"
                        prop:value=move || ui.direction.get()
                        on:change=move |e| ui.direction.set(event_target_value(&e))
                    >
                        <option value="outflow">"Payment to make"</option>
                        <option value="inflow">"Money to receive"</option>
                    </select>
                })}
            </Field>
            <Field label="Plan name" id="plan-name">
                <input
                    id="plan-name"
                    name="name"
                    required
                    maxlength="128"
                    placeholder="e.g. insurance instalment"
                    class=CONTROL
                    prop:value=move || ui.name.get()
                    on:input=move |e| ui.name.set(event_target_value(&e))
                />
            </Field>
            <Field
                label="Match category"
                id="plan-category"
                hint="Marks this plan paid when a matching transaction arrives."
            >
                {select_wrap(view! {
                    <select
                        id="plan-category"
                        name="match_category"
                        required
                        class=select_class()
                        prop:value=move || ui.match_category.get()
                        on:change=move |e| ui.match_category.set(event_target_value(&e))
                    >
                        <option value="" disabled selected=ui.match_category.get_untracked().is_empty()>
                            "Choose category"
                        </option>
                        {category_options}
                    </select>
                })}
            </Field>
            <Field label="Account" id="plan-account">
                {select_wrap(view! {
                    <select
                        id="plan-account"
                        name="account_id"
                        required
                        class=select_class()
                        prop:value=move || ui.account_id.get()
                        on:change=move |e| ui.account_id.set(event_target_value(&e))
                    >
                        <option value="" disabled selected=ui.account_id.get_untracked().is_empty()>
                            "Choose account"
                        </option>
                        {account_options}
                    </select>
                })}
            </Field>
            <Field label="Amount" id="plan-amount">
                <input
                    id="plan-amount"
                    name="amount"
                    required
                    type="number"
                    min="0.01"
                    step="0.01"
                    placeholder="0.00"
                    class=CONTROL
                    node_ref={
                        let node = NodeRef::<leptos::html::Input>::new();
                        sync_value(node, move || ui.amount.get());
                        node
                    }
                    on:input=move |e| ui.amount.set(event_target_value(&e))
                />
            </Field>
            <Field label="Currency" id="plan-currency">
                {select_wrap(view! {
                    <select
                        id="plan-currency"
                        name="currency"
                        required
                        class=select_class()
                        prop:value=currency
                        on:change=move |e| ui.currency.set(event_target_value(&e))
                    >
                        {currency_options}
                    </select>
                })}
            </Field>
            <Field label="Due date" id="plan-due">
                <input
                    id="plan-due"
                    name="due_on"
                    required
                    type="date"
                    min=min
                    max=max
                    class=CONTROL
                    prop:value=move || ui.due_on.get()
                    on:input=move |e| ui.due_on.set(event_target_value(&e))
                />
            </Field>
            <Button
                submit=true
                variant=Variant::Secondary
                disabled=Signal::derive(move || ui.plan_busy.get() || !has_currencies)
            >
                {move || {
                    if ui.plan_busy.get() && ui.plan_action.get() == "create" { "Saving…" } else { "Add plan" }
                }}
            </Button>
            {feedback_in(ui, "create".into(), "sm:col-span-2 lg:col-span-3")}
        </form>
    }
}

/// The backend's message, or `fallback` when it sent none.
fn error_or(error: String, fallback: &str) -> String {
    if error.trim().is_empty() || error == "unknown-error" {
        fallback.to_owned()
    } else {
        error
    }
}

fn archive_plan(ui: Ui, plan_id: String, name: String) {
    if ui.plan_busy.get_untracked() {
        return;
    }
    let target = format!("plan:{plan_id}");
    ui.plan_busy.set(true);
    ui.plan_action.set(target.clone());
    ui.set_message(&target, "", false);
    spawn_local(async move {
        let args = PlanIdArgs {
            plan_id: plan_id.clone(),
        };
        match invoke::<IgnoredAny>("archive_planned_payment", &args).await {
            Ok(_) => {
                ui.removed_plans.update(|r| r.push(plan_id));
                ui.saved("plan-removed", format!("{name} removed."));
                ui.archive_id.set(None);
            }
            Err(e) => ui.set_message(
                &target,
                error_or(e, "Could not save planned payment."),
                true,
            ),
        }
        ui.plan_busy.set(false);
    });
}

fn plan_row(ui: Ui, plan: PlannedPaymentView) -> impl IntoView {
    let inflow = plan.direction == Direction::Inflow;
    let detail = vec![
        (
            "Kind".to_owned(),
            if inflow { "Expected income" } else { "Payment" }.to_owned(),
        ),
        (
            "Matches".to_owned(),
            plan.match_description
                .clone()
                .unwrap_or_else(|| plan.match_category.clone()),
        ),
        (
            if plan.pending { "Card payment" } else { "Due" }.to_owned(),
            plan.due_on.clone(),
        ),
    ];
    let status = match plan.status {
        PlanStatus::Paid => format!(
            "{} {}",
            if inflow { "Received" } else { "Paid" },
            format::day_month(plan.paid_on.as_deref().unwrap_or(&plan.due_on))
        ),
        PlanStatus::Pending => "Reserved · awaiting bank posting".into(),
        PlanStatus::NeedsReview => {
            format!("{} possible matches · review needed", plan.candidate_count)
        }
        PlanStatus::Overdue => "Overdue · no matching transaction".into(),
        PlanStatus::Upcoming if inflow => "Expected".into(),
        PlanStatus::Upcoming => "Upcoming".into(),
    };
    let tone = match plan.status {
        PlanStatus::Paid => "text-positive-fg",
        PlanStatus::Overdue | PlanStatus::NeedsReview => "text-warning-fg",
        _ => "text-fg-muted",
    };
    let id = plan.plan_id.clone();
    let action = if id.starts_with("recurring-salary-") {
        view! {
            <a
                class="text-accent underline underline-offset-2"
                href="#"
                on:click=move |e| {
                    e.prevent_default();
                    ui.app.go_manage(ManageSection::Settings);
                }
            >
                "Edit salary"
            </a>
        }
        .into_any()
    } else {
        let (confirm_id, show_id, target) = (id.clone(), id.clone(), format!("plan:{id}"));
        let name = plan.name.clone();
        view! {
            {move || {
                let removing = ui.archive_id.get().as_deref() == Some(show_id.as_str());
                if removing {
                    let (id, name, target) = (confirm_id.clone(), name.clone(), target.clone());
                    view! {
                        <span class="flex gap-2">
                            <Button
                                size=Size::Sm
                                variant=Variant::Danger
                                disabled=Signal::derive(move || ui.plan_busy.get())
                                on_click=Callback::new(move |_| archive_plan(ui, id.clone(), name.clone()))
                            >
                                {move || {
                                    if ui.plan_busy.get() && ui.plan_action.get() == target { "Removing…" } else { "Confirm remove" }
                                }}
                            </Button>
                            <Button
                                size=Size::Sm
                                variant=Variant::Secondary
                                disabled=Signal::derive(move || ui.plan_busy.get())
                                on_click=Callback::new(move |_| ui.archive_id.set(None))
                            >
                                "Cancel"
                            </Button>
                        </span>
                    }
                    .into_any()
                } else {
                    let id = show_id.clone();
                    view! {
                        <Button
                            size=Size::Sm
                            variant=Variant::Ghost
                            disabled=Signal::derive(move || ui.plan_busy.get())
                            on_click=Callback::new(move |_| ui.archive_id.set(Some(id.clone())))
                        >
                            "Remove"
                        </Button>
                    }
                    .into_any()
                }
            }}
        }
        .into_any()
    };
    view! {
        <div class="grid grid-cols-[1fr_auto] items-center gap-x-4 gap-y-2 border-b border-line py-3 text-sm md:grid-cols-[minmax(0,1fr)_7rem_14rem_minmax(6rem,auto)]">
            <div class="grid min-w-0 gap-1">
                <strong>{plan.name.clone()}</strong>
                // Fixed tracks, so every row's fields line up in columns.
                <FactsRow rows=detail class="grid grid-cols-[9rem_minmax(0,1fr)_7rem] sm:max-w-xl" />
            </div>
            <strong class="text-right tabular-nums">
                <Money value=plan.amount.amount unit=plan.amount.currency.clone() neutral=true />
            </strong>
            <span class=cn(["col-span-2 md:col-span-1", tone])>{status}</span>
            {action}
            {feedback_in(ui, format!("plan:{id}"), "col-span-full")}
        </div>
    }
}

/// One bill: its amount, editor, removal, automatic matching, schedule and
/// suggested payments.
#[component]
fn BillCard(ui: Ui, bill: SubscriptionRow, shared: StoredValue<Shared>) -> impl IntoView {
    let (active_month, stale, scheduled) = shared.with_value(|s| {
        (
            s.active_month.clone(),
            s.stale,
            s.scheduled
                .iter()
                .filter(|p| p.bill_id == bill.bill_id)
                .cloned()
                .collect::<Vec<_>>(),
        )
    });
    let id = bill.bill_id.clone();
    let mut subtitle = vec![
        ("Frequency".to_owned(), bill.frequency.to_string()),
        (
            "Latest payment".to_owned(),
            bill.last_recorded_on.clone().unwrap_or_else(|| "—".into()),
        ),
    ];
    if !bill.expected_dates.is_empty() {
        subtitle.push((
            format!("Scheduled {active_month}"),
            bill.expected_dates.join(", "),
        ));
    }
    let bill = StoredValue::new(bill);
    let editing_this = {
        let id = id.clone();
        move || ui.editing.get().as_deref() == Some(id.as_str())
    };
    let removing_this = {
        let id = id.clone();
        move || ui.removing_bill.get().as_deref() == Some(id.as_str())
    };
    let remove_target = format!("bill:{id}:remove");
    let schedule = (!scheduled.is_empty()).then(|| {
        let lines = scheduled
            .into_iter()
            .map(|payment| {
                let (class, text) = match (&payment.paid_on, payment.candidate_ids.is_empty()) {
                    (Some(day), _) => (
                        "text-positive-fg",
                        format!("Paid {}", format::day_month(day)),
                    ),
                    (None, false) => ("text-fg-muted", "Payment found · review match".to_owned()),
                    (None, true) => (
                        "text-fg-muted",
                        format!(
                            "To pay · {}",
                            crate::ds::money(payment.amount.amount, &payment.amount.currency)
                        ),
                    ),
                };
                view! {
                    <p class="flex justify-between gap-4 border-b border-line py-2.5 text-sm">
                        <span>{format!("Due {}", payment.due_on)}</span>
                        <strong class=class>{text}</strong>
                    </p>
                }
            })
            .collect_view();
        view! { <div class="mt-3 border-t border-line">{lines}</div> }
    });
    let (name, amount_max, currency) =
        bill.with_value(|b| (b.name.clone(), b.amount_max, b.currency.clone()));
    let toggle_editor = {
        let id = id.clone();
        move |_| {
            ui.editing.update(|e| {
                *e = if e.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id.clone())
                };
            });
        }
    };
    let remove_controls = {
        let (id, target) = (id.clone(), remove_target.clone());
        move || {
            if removing_this() {
                let (id, target) = (id.clone(), target.clone());
                view! {
                    <div class="flex flex-wrap items-center gap-2 text-sm">
                        <span>"Remove future payments? Past transactions stay linked."</span>
                        <Button
                            size=Size::Sm
                            variant=Variant::Danger
                            disabled=Signal::derive(move || ui.plan_busy.get())
                            on_click=Callback::new(move |_| archive_bill(ui, bill, id.clone()))
                        >
                            {move || {
                                if ui.plan_busy.get() && ui.plan_action.get() == target { "Removing…" } else { "Confirm remove" }
                            }}
                        </Button>
                        <Button
                            size=Size::Sm
                            variant=Variant::Secondary
                            disabled=Signal::derive(move || ui.plan_busy.get())
                            on_click=Callback::new(move |_| ui.removing_bill.set(None))
                        >
                            "Cancel"
                        </Button>
                    </div>
                }
                .into_any()
            } else {
                let id = id.clone();
                view! {
                    <Button
                        variant=Variant::Ghost
                        disabled=Signal::derive(move || ui.plan_busy.get() || stale)
                        on_click=Callback::new(move |_| ui.removing_bill.set(Some(id.clone())))
                    >
                        "Remove recurring plan"
                    </Button>
                }
                .into_any()
            }
        }
    };
    let editing_label = editing_this.clone();
    let editor = move || editing_this().then(|| bill_editor(ui, bill));
    view! {
        <article class=cn([card_class(CardTone::Default), "min-w-0"])>
            <div class="flex flex-col items-stretch justify-between gap-4 sm:flex-row sm:items-start">
                <div>
                    <h3 class="text-lg font-semibold">{name}</h3>
                    <FactsRow class="mt-2" rows=subtitle />
                </div>
                <strong class="whitespace-nowrap text-accent">
                    <Money value=amount_max unit=currency neutral=true />
                </strong>
            </div>
            <div class="mt-3 flex flex-wrap items-center gap-2">
                <Button
                    variant=Variant::Secondary
                    disabled=Signal::derive(move || ui.edit_busy.get() || stale)
                    on_click=Callback::new(toggle_editor)
                >
                    {move || if editing_label() { "Close editor" } else { "Edit fixed spending" }}
                </Button>
                {remove_controls}
            </div>
            {feedback(ui, remove_target)}
            {editor}
            {feedback(ui, format!("bill:{id}:edit"))}
            {matching(ui, bill, shared)}
            {schedule}
            {feedback(ui, format!("bill:{id}:link"))}
            {candidates(ui, bill, stale)}
        </article>
    }
}

fn archive_bill(ui: Ui, bill: StoredValue<SubscriptionRow>, bill_id: String) {
    if ui.plan_busy.get_untracked() {
        return;
    }
    let target = format!("bill:{bill_id}:remove");
    ui.plan_busy.set(true);
    ui.plan_action.set(target.clone());
    ui.set_message(&target, "", false);
    let name = bill.with_value(|b| b.name.clone());
    spawn_local(async move {
        let request = BillRef {
            bill_id: bill_id.clone(),
        };
        match invoke::<()>("archive_bill", &Request { request }).await {
            Ok(()) => {
                ui.removing_bill.set(None);
                ui.removed_bills.update(|r| r.push(bill_id));
                ui.saved(
                    "bill-removed",
                    format!("{name} removed. Payment history is kept."),
                );
            }
            Err(e) => ui.set_message(&target, error_or(e, "Could not save recurring plan."), true),
        }
        ui.plan_busy.set(false);
    });
}

fn bill_editor(ui: Ui, bill: StoredValue<SubscriptionRow>) -> impl IntoView {
    let row = bill.get_value();
    let name = RwSignal::new(row.name.clone());
    let amount_min = RwSignal::new(row.amount_min.normalize().to_string());
    let amount_max = RwSignal::new(row.amount_max.normalize().to_string());
    let frequency = RwSignal::new(row.frequency.clone());
    let date = RwSignal::new(String::new());
    let bill_id = row.bill_id.clone();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if ui.edit_busy.get_untracked() {
            return;
        }
        let target = format!("bill:{bill_id}:edit");
        let (Ok(minimum), Ok(maximum)) = (
            amount_min.get_untracked().trim().parse::<Decimal>(),
            amount_max.get_untracked().trim().parse::<Decimal>(),
        ) else {
            ui.set_message(&target, "Enter both amounts.", true);
            return;
        };
        let date = date.get_untracked();
        let request = BillEditRequest {
            bill_id: bill_id.clone(),
            name: name.get_untracked().trim().to_owned(),
            amount_min: minimum,
            amount_max: maximum,
            repeat_freq: frequency.get_untracked(),
            date: (!date.trim().is_empty()).then(|| date.trim().to_owned()),
        };
        ui.edit_busy.set(true);
        ui.set_message(&target, "", false);
        spawn_local(async move {
            match invoke::<()>("edit_bill", &Request { request }).await {
                Ok(()) => {
                    ui.editing.set(None);
                    ui.saved(&target, "Bill updated.");
                }
                Err(e) => ui.set_message(&target, error_or(e, "Could not update bill."), true),
            }
            ui.edit_busy.set(false);
        });
    };
    let id = row.bill_id.clone();
    let [f_name, f_min, f_max, f_frequency, f_date] =
        ["name", "min", "max", "frequency", "date"].map(|suffix| format!("bill-{id}-{suffix}"));
    let frequencies = BILL_FREQUENCIES
        .iter()
        .map(|f| {
            let selected = *f == row.frequency;
            view! { <option value=*f selected=selected>{*f}</option> }
        })
        .collect_view();
    view! {
        <form class="my-5 grid items-end gap-3 sm:grid-cols-2 lg:grid-cols-3" on:submit=submit>
            <Field label="Name" id=f_name.clone()>
                <input
                    id=f_name
                    name="name"
                    required
                    maxlength="128"
                    class=CONTROL
                    prop:value=move || name.get()
                    on:input=move |e| name.set(event_target_value(&e))
                />
            </Field>
            <Field label="Minimum amount" id=f_min.clone()>
                <input
                    id=f_min
                    name="amount_min"
                    required
                    type="number"
                    min="0"
                    step="0.01"
                    class=CONTROL
                    node_ref={
                        let node = NodeRef::<leptos::html::Input>::new();
                        sync_value(node, move || amount_min.get());
                        node
                    }
                    on:input=move |e| amount_min.set(event_target_value(&e))
                />
            </Field>
            <Field label="Maximum amount" id=f_max.clone()>
                <input
                    id=f_max
                    name="amount_max"
                    required
                    type="number"
                    min="0"
                    step="0.01"
                    class=CONTROL
                    node_ref={
                        let node = NodeRef::<leptos::html::Input>::new();
                        sync_value(node, move || amount_max.get());
                        node
                    }
                    on:input=move |e| amount_max.set(event_target_value(&e))
                />
            </Field>
            <Field label="Frequency" id=f_frequency.clone()>
                {select_wrap(view! {
                    <select
                        id=f_frequency
                        name="repeat_freq"
                        class=select_class()
                        prop:value=move || frequency.get()
                        on:change=move |e| frequency.set(event_target_value(&e))
                    >
                        {frequencies}
                    </select>
                })}
            </Field>
            <Field label="New schedule start (optional)" id=f_date.clone()>
                <input
                    id=f_date
                    name="date"
                    type="date"
                    class=CONTROL
                    prop:value=move || date.get()
                    on:input=move |e| date.set(event_target_value(&e))
                />
            </Field>
            <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || ui.edit_busy.get())>
                {move || if ui.edit_busy.get() { "Saving…" } else { "Save bill" }}
            </Button>
        </form>
    }
}

/// "Automatic matching": the categories whose payments count as this bill's.
fn matching(
    ui: Ui,
    bill: StoredValue<SubscriptionRow>,
    shared: StoredValue<Shared>,
) -> impl IntoView {
    let row = bill.get_value();
    let count = row.category_match_categories.len();
    let summary = format!(
        "Automatic matching · {}",
        if !row.category_match_configured {
            "bill-name fallback".to_owned()
        } else if count == 0 {
            "off".to_owned()
        } else {
            format!("{count} categor{}", if count == 1 { "y" } else { "ies" })
        }
    );
    let (categories, owners) =
        shared.with_value(|s| (s.subscription_categories.clone(), s.owners.clone()));
    let saved_here: Vec<String> = row
        .category_match_categories
        .iter()
        .map(|c| c.to_lowercase())
        .collect();
    let selected = RwSignal::new(
        categories
            .iter()
            .filter(|c| saved_here.contains(&c.to_lowercase()))
            .cloned()
            .collect::<Vec<_>>(),
    );
    let id = row.bill_id.clone();
    let busy = {
        let id = id.clone();
        move || ui.category_busy.get().as_deref() == Some(id.as_str())
    };
    let target = format!("bill:{id}:matching");
    let submit = {
        let target = target.clone();
        let row = row.clone();
        let order = categories.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            if ui.category_busy.get_untracked().is_some() {
                return;
            }
            // In the order the boxes show, as the web form submitted them.
            let chosen = selected.get_untracked();
            let categories: Vec<String> = order
                .iter()
                .filter(|c| chosen.contains(c))
                .cloned()
                .collect();
            let request = SubscriptionCategoriesRequest {
                bill_id: row.bill_id.clone(),
                categories: categories.clone(),
                expected_revision: row.category_match_revision,
            };
            let (target, name) = (target.clone(), row.name.clone());
            ui.category_busy.set(Some(row.bill_id.clone()));
            ui.set_message(&target, "", false);
            spawn_local(async move {
                match invoke::<SubscriptionCategoryRule>(
                    "save_subscription_categories",
                    &Request { request },
                )
                .await
                {
                    Ok(_) => {
                        let n = categories.len();
                        let text = if n == 0 {
                            format!("Automatic category matching is off for {name}.")
                        } else {
                            format!(
                                "{name} will match {n} selected categor{}.",
                                if n == 1 { "y" } else { "ies" }
                            )
                        };
                        ui.saved(&target, text);
                    }
                    Err(e) => ui.set_message(
                        &target,
                        error_or(e, "Could not save automatic matching."),
                        true,
                    ),
                }
                ui.category_busy.set(None);
            });
        }
    };
    let empty = categories.is_empty();
    let boxes = categories
        .into_iter()
        .map(|category| {
            let key = category.to_lowercase();
            let owner = owners.get(&key).cloned();
            let here = saved_here.contains(&key);
            let elsewhere = owner.as_ref().filter(|(owner_id, _)| *owner_id != id).map(|(_, name)| name.clone());
            let blocked = elsewhere.is_some();
            let class = cn([
                "grid min-h-10 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-2 rounded-md border border-line bg-card-muted px-3 py-2 text-xs",
                if here { "border-accent bg-accent-soft" } else { "" },
                if blocked { "cursor-not-allowed opacity-60" } else { "cursor-pointer" },
            ]);
            let (check, toggle) = (category.clone(), category.clone());
            let busy = busy.clone();
            view! {
                <label class=class>
                    <crate::ds::Checkbox
                        label=category.clone()
                        checked=Signal::derive(move || selected.with(|s| s.contains(&check)))
                        on_change=Callback::new(move |on: bool| {
                            selected.update(|s| {
                                s.retain(|c| *c != toggle);
                                if on {
                                    s.push(toggle.clone());
                                }
                            });
                        })
                        disabled=Signal::derive(move || blocked || busy())
                    />
                    <span class="min-w-0 truncate">{category}</span>
                    {elsewhere.map(|name| view! { <small class="col-start-2 text-[0.65rem] text-fg-muted">{name}</small> })}
                </label>
            }
        })
        .collect_view();
    let busy_label = busy.clone();
    view! {
        <details class="mt-4 border-t border-line pt-3">
            <summary class="cursor-pointer text-sm font-semibold">{summary}</summary>
            <form class="grid gap-3 pt-3" on:submit=submit>
                <Help text="Choose the transaction categories that belong to this subscription. A matching amount in the billing window can then be marked paid automatically. One category can belong to only one subscription." />
                <div class="grid grid-cols-[repeat(auto-fit,minmax(145px,1fr))] gap-2">{boxes}</div>
                {empty.then(|| view! {
                    <Help text="Categorize a payment first to create matching options. You can still save this subscription with automatic category matching off." />
                })}
                <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
                    <Button submit=true variant=Variant::Secondary size=Size::Sm disabled=Signal::derive(busy)>
                        {move || if busy_label() { "Saving…" } else { "Save matching" }}
                    </Button>
                </div>
                {feedback(ui, target)}
            </form>
        </details>
    }
}

fn decide(ui: Ui, bill: StoredValue<SubscriptionRow>, candidate: BillLinkCandidate, confirm: bool) {
    let bill_id = bill.with_value(|b| b.bill_id.clone());
    let key = format!("{bill_id}:{}", candidate.transaction_id);
    if ui.saving.with_untracked(|s| s.contains(&key)) {
        return;
    }
    let target = format!("bill:{bill_id}:link");
    ui.saving.update(|s| s.push(key.clone()));
    ui.set_message(&target, "", false);
    spawn_local(async move {
        let request = BillMatchRequest {
            bill_id,
            transaction_id: candidate.transaction_id.clone(),
        };
        let command = if confirm {
            "link_bill_payment"
        } else {
            "decline_bill_match"
        };
        match invoke::<()>(command, &Request { request }).await {
            Ok(()) => {
                ui.linked.update(|l| l.push(key.clone()));
                ui.saved(
                    &target,
                    if confirm {
                        "Payment linked."
                    } else {
                        "Payment match declined."
                    },
                );
            }
            Err(e) => ui.set_message(
                &target,
                error_or(e, "Could not confirm the bill link."),
                true,
            ),
        }
        ui.saving.update(|s| s.retain(|k| *k != key));
    });
}

/// "Possible payments to link", without the ones just decided.
fn candidates(ui: Ui, bill: StoredValue<SubscriptionRow>, stale: bool) -> impl IntoView {
    move || {
        let row = bill.get_value();
        let open: Vec<BillLinkCandidate> = ui.linked.with(|linked| {
            row.link_candidates
                .iter()
                .filter(|c| !linked.contains(&format!("{}:{}", row.bill_id, c.transaction_id)))
                .cloned()
                .collect()
        });
        (!open.is_empty()).then(|| {
            let cards = open
                .into_iter()
                .map(|candidate| {
                    let key = format!("{}:{}", row.bill_id, candidate.transaction_id);
                    let differs = candidate.amount < row.amount_min || candidate.amount > row.amount_max;
                    let (unit, group_label) = (row.currency.clone(), format!("Payment match for {}", row.name));
                    let (saving_a, saving_b, saving_c) = (key.clone(), key.clone(), key);
                    let is_saving = move |k: &String| ui.saving.with(|s| s.contains(k));
                    let (confirm, decline) = (candidate.clone(), candidate.clone());
                    view! {
                        <Card tone=CardTone::Muted class="grid items-center gap-4 p-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:p-4">
                            <div class="min-w-0">
                                <b>
                                    {format!("{} · ", candidate.occurred_on)}
                                    <Money value=candidate.amount unit=unit neutral=true />
                                </b>
                                <p class="my-1 break-words text-sm">{candidate.description.clone()}</p>
                                {differs.then(|| view! { <Badge tone=Tone::Warning>"Amount differs"</Badge> })}
                            </div>
                            <div role="group" aria-label=group_label class="inline-flex flex-wrap items-center gap-2">
                                <Button
                                    disabled=Signal::derive(move || is_saving(&saving_a) || stale)
                                    on_click=Callback::new(move |_| decide(ui, bill, confirm.clone(), true))
                                >
                                    {move || if is_saving(&saving_b) { "Saving…" } else { "Confirm" }}
                                </Button>
                                <Button
                                    variant=Variant::Secondary
                                    disabled=Signal::derive(move || is_saving(&saving_c) || stale)
                                    on_click=Callback::new(move |_| decide(ui, bill, decline.clone(), false))
                                >
                                    "Decline"
                                </Button>
                            </div>
                        </Card>
                    }
                })
                .collect_view();
            view! {
                <div class="mt-4 grid gap-3">
                    <strong>"Possible payments to link"</strong>
                    {cards}
                </div>
            }
        })
    }
}
