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
use crate::ds::patterns::{
    ADD_FORM, BADGE_TIGHT, BOARD_SECTION, BOARD_TITLE, BUTTON_GROUP, CARD_NAME, DATE, Disclosure,
    DisclosureKind, FEEDBACK_LINE, FIGURE, HINT, INLINE_TOOLS, LIST_STACK, META_LINE, MORE_DETAILS,
    MUTED, NAME_ROW, PANEL_TITLE, PAYMENT_ROW, ROW_LIST, SECTION_GRID, SELECT_CHEVRON, SELECT_WRAP,
    TRUNCATE, WARNING_LINE,
};
use crate::ds::sync_value;
use crate::ds::{
    BOARD, BOARD_CARD, Badge, BoardColumn, Button, CONTROL, Card, CardTone, Eyebrow, Facts,
    FactsRow, Field, Help, Icon, Lucide, Money, MonthSwitcher, Popup, Size, Tone, Variant,
    card_class, cn,
};
use crate::layout::{Initial, PageView, PanelDef};
use crate::pages::payment_pick::{Ask, PaymentPicker};
use crate::pages::{report_is, with_report};
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
    /// The purchase an expected income pays back, when it does.
    reimbursement_for: RwSignal<String>,
    /// The plan whose box is open over the page.
    open_plan: RwSignal<Option<String>>,
    /// The plan being dragged, and whether it is one paid by hand.
    dragging: RwSignal<Option<(String, bool)>>,
    /// The plan a payment is being picked for.
    ask: RwSignal<Option<Ask>>,
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
            reimbursement_for: RwSignal::new(String::new()),
            open_plan: RwSignal::new(None),
            dragging: RwSignal::new(None),
            ask: RwSignal::new(None),
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
            self.reimbursement_for,
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
                    FEEDBACK_LINE,
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
        <div class=SELECT_WRAP>
            {select}
            <span aria-hidden="true" class=SELECT_CHEVRON>
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

fn select_class() -> String {
    cn([CONTROL, "appearance-none pr-9"])
}

/// Plan's panels, in their first-run order.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "plan.salary",
        title: "Salary and where the money goes",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || with_report(salary).into_any(),
        summary: None,
        shown: Some(|| report_is(|r| r.cash_overview.payday_status.is_some())),
    },
    PanelDef {
        id: "plan.payments",
        title: "Payments",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: false,
        render: || with_month(payments).into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "plan.add",
        title: "Add a payment or income",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: || with_month(add).into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "plan.bills",
        title: "Bills",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: || with_month(bills).into_any(),
        summary: None,
        shown: None,
    },
];

const PAGE: &str = "plan";

/// The page state and the shown month, by context; the plan box and the
/// payment picker open over the page, outside the panels.
#[component]
pub fn Plan() -> impl IntoView {
    let ui = Ui::new();
    let month = Memo::new(move |_| {
        ui.app.report.data.with(|d| match d {
            Some(Ok(report)) => Some(Shared::new(ui, report)),
            _ => None,
        })
    });
    provide_context(ui);
    provide_context(month);
    view! {
        <PageView tab="plan" tab_name="Plan" page=PAGE />
        {move || {
            let id = ui.open_plan.get()?;
            let plan = month.with(|m| m.as_ref()?.plans.iter().find(|p| p.plan_id == id).cloned())?;
            Some(plan_popup(ui, plan))
        }}
        <PaymentPicker
            ask=ui.ask
            busy=ui.plan_busy
            on_pick=Callback::new(move |(ask, row): (Ask, finance_types::report::review::SearchRow)| {
                tie_plan(ui, ask.id, Some(row.transaction_id));
            })
            otherwise="Newest first, the ones of the same amount on top. No payment yet? Close this: the box stays where it is."
        />
    }
}

/// `f` over the page state and the shown month, once the report has loaded.
fn with_month(f: fn(Ui, &Shared, &Dashboard) -> AnyView) -> impl IntoView {
    let ui = expect_context::<Ui>();
    let month = expect_context::<Memo<Option<Shared>>>();
    with_report(move |report| month.with(|m| m.as_ref().map(|m| f(ui, m, report))))
}

/// Account id → name pairs.
fn accounts(report: &Dashboard) -> Vec<(String, String)> {
    report
        .cash_overview
        .accounts
        .iter()
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect()
}

/// The salary card; it reads only the report, so it may sit on any tab.
fn salary(report: &Dashboard) -> impl IntoView + use<> {
    report.cash_overview.payday_status.clone().map(|payday| {
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
                    <Facts class="mt-2" rows=vec![("On".to_owned(), format::day_month(day))] />
                }
                .into_any()
            }
            None => view! { <Facts class="mt-2" rows=vec![("Expected".to_owned(), format::day_month(&payday.due_on))] /> }.into_any(),
        };
        let title = if payday.received_on.is_some() {
            "Received"
        } else {
            "Not in yet"
        };
        let plan_fields = report.money_plan.clone().map(|plan| {
            let names: HashMap<String, String> = accounts(report).into_iter().collect();
            view! {
                <div class="mt-4 border-t border-line pt-4">
                    <Eyebrow>"WHERE THE MONEY GOES"</Eyebrow>
                    <div class="mt-3"><MoneyPlanFields plan=plan names=names /></div>
                </div>
            }
        });
        view! {
            <Card>
                <Eyebrow>"SALARY"</Eyebrow>
                <h2 class=PANEL_TITLE>{title}</h2>
                {body}
                {plan_fields}
            </Card>
        }
    })
}

/// The month switcher, then the payments board.
fn payments(ui: Ui, s: &Shared, report: &Dashboard) -> AnyView {
    let active_month = &s.active_month;
    let today = &s.today;
    let mut boxes: Vec<Boxed> = s
        .scheduled
        .iter()
        // A bill of nothing (a card with no charge) is not a payment.
        .filter(|payment| payment.paid_on.is_some() || payment.amount.amount > Decimal::ZERO)
        .map(|payment| {
            let column = bill_column(
                payment.paid_on.is_some(),
                !payment.candidate_ids.is_empty(),
                &payment.due_on,
                today,
            );
            let day = payment
                .paid_on
                .clone()
                .unwrap_or_else(|| payment.due_on.clone());
            let head = box_head(
                payment.name.clone(),
                crate::ds::money(payment.amount.amount, &payment.amount.currency),
                (Tone::Outline, "Bill"),
                format::day_month(&day),
            );
            // A bill opens in place: its dates and matches; it is edited under Bills.
            let mut rows = vec![("Due".to_owned(), format::day_month(&payment.due_on))];
            rows.push(match &payment.paid_on {
                Some(on) => ("Paid on".to_owned(), format::day_month(on)),
                None => ("Paid".to_owned(), "Not yet".to_owned()),
            });
            if payment.paid_on.is_none() && !payment.candidate_ids.is_empty() {
                rows.push((
                    "Possible payments".to_owned(),
                    payment.candidate_ids.len().to_string(),
                ));
            }
            let view = view! {
                <Disclosure kind=DisclosureKind::Card class=BOARD_CARD summary=head>
                    <Facts class="mt-1" rows=rows />
                    <p class=HINT>"Change it under Bills below."</p>
                </Disclosure>
            }
            .into_any();
            (column, day, payment.amount.amount, view)
        })
        .collect();

    let switcher_month = if s.next {
        s.next_month.clone()
    } else {
        s.current_month.clone()
    };
    let current_for_switch = s.current_month.clone();
    let on_month = Callback::new(move |month: String| {
        ui.next_month.set(month != current_for_switch);
        // The due date belongs to the month shown (the form keyed it).
        ui.due_on.set(String::new());
    });

    // The events with a share of the shown month.
    for event in &report.events {
        let Some(month) = event
            .months
            .iter()
            .find(|m| m.month.starts_with(active_month.as_str()))
        else {
            continue;
        };
        let Some(dates) = event.row.wish.event.as_ref() else {
            continue;
        };
        let unit = &event.row.wish.currency;
        let head = box_head(
            event.row.wish.name.clone(),
            crate::ds::money(month.share, unit),
            (Tone::Accent, "Event"),
            format::day_month(&dates.starts_on),
        );
        let rows = vec![
            ("Until".to_owned(), format::day_month(&dates.ends_on)),
            ("Spent".to_owned(), crate::ds::money(month.spent, unit)),
        ];
        let view = view! {
            <Disclosure kind=DisclosureKind::Card class=BOARD_CARD summary=head>
                <Facts class="mt-1" rows=rows />
            </Disclosure>
        }
        .into_any();
        boxes.push((Column::Due, dates.starts_on.clone(), month.share, view));
    }
    boxes.extend(s.plans.iter().map(|plan| plan_card(ui, plan)));
    let plans = StoredValue::new(s.plans.clone());
    let ask_for = move |id: &str| {
        plans.with_value(|all| {
            all.iter().find(|p| p.plan_id == id).map(|p| Ask {
                id: p.plan_id.clone(),
                name: p.name.clone(),
                currency: p.amount.currency.code.clone(),
                incoming: p.direction == Direction::Inflow,
                amount: p.amount.amount,
                taken: None,
            })
        })
    };
    boxes.sort_by(|a, b| a.1.cmp(&b.1));
    let board_unit = report
        .monthly_allocation
        .as_ref()
        .and_then(|m| m.currencies.first())
        .map(|a| a.currency.clone());
    let board = COLUMNS
        .into_iter()
        .map(|(column, title, tone)| {
            let (mut total, mut cards) = (Decimal::ZERO, Vec::new());
            let mut rest = Vec::new();
            for item in std::mem::take(&mut boxes) {
                if item.0 == column {
                    total += item.2;
                    cards.push(item.3);
                } else {
                    rest.push(item);
                }
            }
            boxes = rest;
            let count = cards.len();
            // Late and To check only show when something is in them.
            if count == 0 && matches!(column, Column::Late | Column::Check) {
                return ().into_any();
            }
            // What is paid matters least: the latest few, the rest folded.
            if column == Column::Done {
                cards.reverse();
                if cards.len() > PAID_SHOWN {
                    let older = cards.split_off(PAID_SHOWN);
                    let label = format!("{} earlier", older.len());
                    cards.push(
                        view! {
                            <Disclosure class=ROW_LIST summary_class="px-1 text-xs text-fg-muted" summary=label>
                                <div class="mt-2 grid gap-2">{older}</div>
                            </Disclosure>
                        }
                        .into_any(),
                    );
                }
            }
            let total = board_unit
                .as_ref()
                .filter(|_| count > 0)
                .map(|unit| crate::ds::money(total, unit));
            // A box goes to Paid by naming its payment, and one paid by
            // hand goes back to waiting; nothing else can be moved.
            let takes = move |tied: bool| match column {
                Column::Done => !tied,
                Column::Due => tied,
                Column::Late | Column::Check => false,
            };
            let accepts =
                Signal::derive(move || ui.dragging.get().is_some_and(|(_, tied)| takes(tied)));
            let on_drop = Callback::new(move |()| {
                let Some((id, tied)) = ui.dragging.get_untracked() else {
                    return;
                };
                if tied {
                    tie_plan(ui, id, None);
                } else {
                    ui.ask.set(ask_for(&id));
                }
            });
            match total {
                Some(total) => view! {
                    <BoardColumn title=title tone=tone count=count total=total accepts=accepts on_drop=on_drop>{cards}</BoardColumn>
                }
                .into_any(),
                None => view! {
                    <BoardColumn title=title tone=tone count=count accepts=accepts on_drop=on_drop>{cards}</BoardColumn>
                }
                .into_any(),
            }
        })
        .collect_view();

    view! {
        <div class=cn([card_class(CardTone::Default), SECTION_GRID])>
            <div role="group" aria-label="Planning month" class="justify-self-start">
                <MonthSwitcher
                    month=Signal::derive(move || switcher_month.clone())
                    on_change=on_month
                    min=s.current_month.clone()
                    max=s.next_month.clone()
                />
            </div>
            <section aria-label="Payments board" class=BOARD_SECTION>
                <div>
                    <Eyebrow>"PAYMENTS"</Eyebrow>
                    <h2 class=BOARD_TITLE>{s.active_label.clone()}</h2>
                </div>
                <p class=HINT>"Open a box for its details. Drag a payment to Paid to link the bank payment."</p>
                {feedback(ui, "board".into())}
                <div class=BOARD>{board}</div>
            </section>
        </div>
    }
    .into_any()
}

/// The form that adds a plan or a recurring bill.
fn add(ui: Ui, s: &Shared, _: &Dashboard) -> AnyView {
    let form = create_form(
        ui,
        &s.available,
        &s.owners,
        &s.accounts,
        &s.currencies,
        &s.active_month,
        &s.active_end,
        s.reimbursable.clone(),
    );
    view! {
        <Card>
            <Eyebrow>"NEW"</Eyebrow>
            <h2 class=PANEL_TITLE>"Add a payment or income"</h2>
            {form}
            {feedback(ui, "plan-removed".into())}
        </Card>
    }
    .into_any()
}

/// The shown month's bills.
fn bills(ui: Ui, s: &Shared, _: &Dashboard) -> AnyView {
    let shared = StoredValue::new(s.clone());
    let bill_cards = s
        .rows
        .iter()
        .map(|bill| view! { <BillCard ui=ui bill=bill.clone() shared=shared /> })
        .collect_view();
    view! {
        <section aria-label="Bills" class=cn([card_class(CardTone::Default), BOARD_SECTION])>
            <div>
                <Eyebrow>"EVERY MONTH"</Eyebrow>
                <h2 class=BOARD_TITLE>"Bills"</h2>
            </div>
            {s.stale.then(|| view! {
                <p role="status" class=WARNING_LINE>"Out of date. Refresh first."</p>
            })}
            {feedback(ui, "bill-removed".into())}
            {s.rows.is_empty().then(|| view! { <p class=MUTED>"No bills this month."</p> })}
            <div class="grid items-start gap-3 sm:grid-cols-2 xl:grid-cols-4">{bill_cards}</div>
        </section>
    }
    .into_any()
}

/// The shown month as the panels read it, rebuilt when the report, the
/// month or a removal changes.
#[derive(Clone, PartialEq)]
struct Shared {
    /// Next month is shown.
    next: bool,
    active_month: String,
    active_label: String,
    active_end: String,
    current_month: String,
    next_month: String,
    stale: bool,
    today: String,
    scheduled: Vec<ScheduledBillPayment>,
    /// The shown bills, less the ones just removed.
    rows: Vec<SubscriptionRow>,
    /// The shown plans, less the ones just removed.
    plans: Vec<PlannedPaymentView>,
    /// What an expected income may pay back: the shown month's purchases.
    reimbursable: Vec<(String, String)>,
    currencies: Vec<String>,
    available: Vec<String>,
    accounts: Vec<(String, String)>,
    subscription_categories: Vec<String>,
    /// Lower-cased category → (bill id, bill name).
    owners: HashMap<String, (String, String)>,
}

impl Shared {
    fn new(ui: Ui, report: &Dashboard) -> Self {
        let next = ui.next_month.get() && report.next_month_subscriptions.is_some();
        let active = if next {
            report
                .next_month_subscriptions
                .as_ref()
                .unwrap_or(&report.subscriptions)
        } else {
            &report.subscriptions
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
        let current_month = month_of(&report.subscriptions.through_date);
        let next_month = report
            .next_month_subscriptions
            .as_ref()
            .map_or_else(|| current_month.clone(), |n| month_of(&n.through_date));
        let reimbursable: Vec<(String, String)> = report
            .monthly_budgets
            .iter()
            .filter(|b| b.plan.month.starts_with(&active_month))
            .flat_map(|b| {
                b.purchases
                    .iter()
                    .filter(|p| p.amount > Decimal::ZERO)
                    .map(|p| {
                        let amount = crate::ds::money(p.amount, &b.plan.currency);
                        (p.id.clone(), format!("{} · {amount}", p.description))
                    })
            })
            .collect();
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
        let mut owners: HashMap<String, (String, String)> = HashMap::new();
        for row in &report.subscriptions.rows {
            for category in &row.category_match_categories {
                owners.insert(
                    category.to_lowercase(),
                    (row.bill_id.clone(), row.name.clone()),
                );
            }
        }
        Self {
            next,
            active_label: format::month_long(&active_month),
            active_end: format::month_end(&active_month),
            stale: active.stale,
            today: report.generated_at.get(..10).unwrap_or_default().to_owned(),
            scheduled: active.scheduled_payments.clone(),
            rows,
            plans,
            reimbursable,
            currencies,
            available: plan_categories(report),
            accounts: accounts(report),
            subscription_categories,
            owners,
            active_month,
            current_month,
            next_month,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_form(
    ui: Ui,
    available: &[String],
    owners: &HashMap<String, (String, String)>,
    accounts: &[(String, String)],
    currencies: &[String],
    active_month: &str,
    active_end: &str,
    reimbursable: Vec<(String, String)>,
) -> impl IntoView + use<> {
    let reimbursable = StoredValue::new(reimbursable);
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
            // Say what is missing instead of only moving the focus.
            let missing = if ui.name.get_untracked().trim().is_empty() {
                Some("Enter a name.")
            } else if ui.match_category.get_untracked().trim().is_empty() {
                Some("Choose a category.")
            } else if ui.account_id.get_untracked().is_empty() {
                Some("Choose an account.")
            } else {
                None
            };
            if let Some(text) = missing {
                ui.set_message("create", text, true);
                return;
            }
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
                    reimbursement_for: Some(ui.reimbursement_for.get_untracked())
                        .filter(|id| !id.is_empty() && ui.direction.get_untracked() == "inflow"),
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
        <form class=ADD_FORM novalidate on:submit=submit>
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
            {move || {
                (ui.repeat.get() == "once" && ui.direction.get() == "inflow").then(|| view! {
                    <Field
                        label="Pays back"
                        id="plan-reimbursement"
                        hint="Optional. Once received, it lowers that purchase's spending."
                    >
                        {select_wrap(view! {
                            <select
                                id="plan-reimbursement"
                                class=select_class()
                                prop:value=move || ui.reimbursement_for.get()
                                on:change=move |e| ui.reimbursement_for.set(event_target_value(&e))
                            >
                                <option value="">"Nothing, general income"</option>
                                {reimbursable
                                    .get_value()
                                    .into_iter()
                                    .map(|(id, label)| {
                                        let selected = id == ui.reimbursement_for.get_untracked();
                                        view! { <option value=id selected=selected>{label}</option> }
                                    })
                                    .collect_view()}
                            </select>
                        })}
                    </Field>
                })
            }}
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

/// Where a payment stands, as a board column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    Due,
    Late,
    Check,
    Done,
}

const COLUMNS: [(Column, &str, Tone); 4] = [
    (Column::Due, "To pay", Tone::Accent),
    (Column::Late, "Late", Tone::Negative),
    (Column::Check, "To check", Tone::Neutral),
    (Column::Done, "Paid", Tone::Positive),
];

fn plan_column(status: PlanStatus) -> Column {
    match status {
        PlanStatus::Upcoming | PlanStatus::Pending => Column::Due,
        PlanStatus::Overdue => Column::Late,
        PlanStatus::NeedsReview => Column::Check,
        PlanStatus::Paid => Column::Done,
    }
}

/// A bill payment: paid, found but not linked yet, late once its day has
/// passed, else still to pay.
fn bill_column(paid: bool, found: bool, due_on: &str, today: &str) -> Column {
    if paid {
        Column::Done
    } else if found {
        Column::Check
    } else if due_on < today {
        Column::Late
    } else {
        Column::Due
    }
}

/// How many paid boxes the board shows before folding the rest.
const PAID_SHOWN: usize = 4;

/// One box on the payments board: `(column, day for ordering, amount, box)`.
type Boxed = (Column, String, Decimal, AnyView);

/// The head of a box: name, amount, a kind tag and the day.
fn box_head(name: String, amount: String, tag: (Tone, &'static str), day: String) -> impl IntoView {
    view! {
        <span class=NAME_ROW>
            <strong class=CARD_NAME>{name}</strong>
            <strong class=FIGURE>{amount}</strong>
        </span>
        <span class=META_LINE>
            <Badge tone=tag.0 class=BADGE_TIGHT>{tag.1}</Badge>
            <span class="whitespace-nowrap">{day}</span>
        </span>
    }
}

/// `tie_planned_payment(plan_id, transaction_id)`.
#[derive(Serialize)]
struct TieArgs {
    plan_id: String,
    transaction_id: Option<String>,
}

/// Tie a payment to a plan by hand, or (`None`) leave it to matching again.
fn tie_plan(ui: Ui, plan_id: String, transaction_id: Option<String>) {
    if ui.plan_busy.get_untracked() {
        return;
    }
    ui.plan_busy.set(true);
    ui.plan_action.set("tie".into());
    ui.set_message("board", "", false);
    let tied = transaction_id.is_some();
    spawn_local(async move {
        let args = TieArgs {
            plan_id,
            transaction_id,
        };
        match invoke::<IgnoredAny>("tie_planned_payment", &args).await {
            Ok(_) => {
                ui.ask.set(None);
                ui.saved(
                    "board",
                    if tied {
                        "Marked paid."
                    } else {
                        "Waiting for its payment again."
                    },
                );
            }
            Err(e) => {
                ui.ask.set(None);
                ui.set_message("board", error_or(e, "Could not save."), true);
            }
        }
        ui.plan_busy.set(false);
    });
}

/// A plan the owner may move by hand: salary is generated, not stored.
fn stored(plan: &PlannedPaymentView) -> bool {
    !plan.plan_id.starts_with("recurring-")
}

/// A plan as a box on the board: click opens it over the page; an unpaid
/// one can be dragged to Paid, one paid by hand back to To pay.
fn plan_card(ui: Ui, plan: &PlannedPaymentView) -> Boxed {
    let inflow = plan.direction == Direction::Inflow;
    let column = plan_column(plan.status);
    let day = plan.paid_on.clone().unwrap_or_else(|| plan.due_on.clone());
    let paid = plan.status == PlanStatus::Paid;
    let by_hand = plan.by_hand;
    let movable = stored(plan) && (!paid || by_hand);
    let tag = if inflow {
        (Tone::Positive, "Income")
    } else {
        (Tone::Neutral, "One-off")
    };
    let amount = crate::ds::money(plan.amount.amount, &plan.amount.currency);
    let head = box_head(
        plan.name.clone(),
        if inflow { format!("+{amount}") } else { amount },
        tag,
        format::day_month(&day),
    );
    let (open_id, drag_id) = (plan.plan_id.clone(), plan.plan_id.clone());
    let view = view! {
        <button
            type="button"
            class=cn([
                BOARD_CARD,
                "text-left transition-colors hover:border-accent",
                if movable { "cursor-grab active:cursor-grabbing" } else { "" },
            ])
            draggable=movable.to_string()
            on:dragstart=move |e| {
                if let Some(data) = e.data_transfer() {
                    let _ = data.set_data("text/plain", &drag_id);
                }
                ui.dragging.set(Some((drag_id.clone(), by_hand)));
            }
            on:dragend=move |_| ui.dragging.set(None)
            on:click=move |_| {
                ui.archive_id.set(None);
                ui.open_plan.set(Some(open_id.clone()));
            }
        >
            {head}
        </button>
    }
    .into_any();
    // Income is not part of what a column still costs.
    let counted = if inflow {
        Decimal::ZERO
    } else {
        plan.amount.amount
    };
    (column, day, counted, view)
}

/// A plan opened over the page: what it is, how it stands, and what can be
/// done with it.
fn plan_popup(ui: Ui, plan: PlannedPaymentView) -> impl IntoView {
    let inflow = plan.direction == Direction::Inflow;
    let paid = plan.status == PlanStatus::Paid;
    let amount = crate::ds::money(plan.amount.amount, &plan.amount.currency);
    let standing = match plan.status {
        PlanStatus::Paid if inflow => "Received",
        PlanStatus::Paid => "Paid",
        PlanStatus::Pending => "Reserved on the card",
        PlanStatus::NeedsReview => "To check",
        PlanStatus::Overdue => "Late",
        PlanStatus::Upcoming if inflow => "Expected",
        PlanStatus::Upcoming => "To pay",
    };
    let mut detail = vec![
        (
            "Amount".to_owned(),
            if inflow { format!("+{amount}") } else { amount },
        ),
        ("Standing".to_owned(), standing.to_owned()),
        (
            if paid { "Planned for" } else { "Due" }.to_owned(),
            format::day_month(&plan.due_on),
        ),
        (
            "Group".to_owned(),
            plan.match_description
                .clone()
                .unwrap_or_else(|| plan.match_category.clone()),
        ),
    ];
    if let Some(day) = &plan.paid_on {
        detail.push((
            if inflow { "Received on" } else { "Paid on" }.to_owned(),
            format::day_month(day),
        ));
    }
    if plan.by_hand {
        detail.push(("Payment".to_owned(), "Picked by hand".to_owned()));
    }
    let id = plan.plan_id.clone();
    // The payments that could be it: naming one settles the plan.
    let candidates = (plan.status == PlanStatus::NeedsReview).then(|| {
        let rows = plan
            .candidates
            .iter()
            .map(|c| {
                let (plan_id, tx) = (id.clone(), c.transaction_id.clone());
                let value = crate::ds::money(c.amount.amount, &c.amount.currency);
                view! {
                    <li class=PAYMENT_ROW>
                        <span class=DATE>{format::date(&c.occurred_on)}</span>
                        <span class=TRUNCATE title=c.description.clone()>{c.description.clone()}</span>
                        <span class="tabular-nums">{value}</span>
                        <Button
                            size=Size::Sm
                            variant=Variant::Secondary
                            disabled=Signal::derive(move || ui.plan_busy.get())
                            on_click=Callback::new(move |_| tie_plan(ui, plan_id.clone(), Some(tx.clone())))
                        >
                            "This one"
                        </Button>
                    </li>
                }
            })
            .collect_view();
        view! {
            <div>
                <Eyebrow>"COULD BE"</Eyebrow>
                <ul class="mt-1">{rows}</ul>
            </div>
        }
    });
    let salary = !stored(&plan);
    let pick = (!salary && !paid).then(|| {
        let ask = Ask {
            id: id.clone(),
            name: plan.name.clone(),
            currency: plan.amount.currency.code.clone(),
            incoming: inflow,
            amount: plan.amount.amount,
            taken: None,
        };
        view! {
            <Button
                size=Size::Sm
                variant=Variant::Secondary
                on_click=Callback::new(move |_| ui.ask.set(Some(ask.clone())))
            >
                {if inflow { "Pick the money received" } else { "Pick the payment" }}
            </Button>
        }
    });
    let untie = plan.by_hand.then(|| {
        let plan_id = id.clone();
        view! {
            <Button
                size=Size::Sm
                variant=Variant::Secondary
                disabled=Signal::derive(move || ui.plan_busy.get())
                on_click=Callback::new(move |_| tie_plan(ui, plan_id.clone(), None))
            >
                "Not this payment"
            </Button>
        }
    });
    let action = if salary {
        view! {
            <Button
                size=Size::Sm
                variant=Variant::Ghost
                on_click=Callback::new(move |_| {
                    ui.open_plan.set(None);
                    ui.app.go_manage(ManageSection::Settings);
                })
            >
                "Edit salary"
            </Button>
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
                        <span class=BUTTON_GROUP>
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
    let close = Callback::new(move |()| {
        ui.archive_id.set(None);
        ui.open_plan.set(None);
    });
    view! {
        <Popup title=plan.name.clone() on_close=close>
            <Facts rows=detail class="max-w-none" />
            {candidates}
            <div class="flex flex-wrap items-center justify-end gap-2">{pick}{untie}{action}</div>
            {feedback(ui, "board".into())}
            {feedback(ui, format!("plan:{id}"))}
        </Popup>
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
    let _ = &active_month;
    let frequency = bill.frequency.to_string();
    let days = |dates: &[String]| {
        dates
            .iter()
            .map(|d| format::day_month(d))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let next_day = days(&bill.expected_dates);
    let mut subtitle = vec![(
        "Last paid".to_owned(),
        bill.last_recorded_on
            .as_deref()
            .map_or_else(|| "—".into(), format::day_month),
    )];
    if !bill.expected_dates.is_empty() {
        subtitle.push(("This month".to_owned(), next_day.clone()));
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
    // A card bill owes what its statement used, not the bill's own amount.
    let owed = scheduled
        .iter()
        .find(|p| p.paid_on.is_none())
        .map(|p| p.amount.amount);
    let schedule = (!scheduled.is_empty()).then(|| {
        let lines = scheduled
            .into_iter()
            .map(|payment| {
                let (class, text) = match (&payment.paid_on, payment.candidate_ids.is_empty()) {
                    (Some(day), _) => (
                        "text-positive-fg",
                        format!("Paid {}", format::day_month(day)),
                    ),
                    (None, false) => ("text-fg-muted", "Found, check below".to_owned()),
                    (None, true) => (
                        "text-fg-muted",
                        format!(
                            "To pay {}",
                            crate::ds::money(payment.amount.amount, &payment.amount.currency)
                        ),
                    ),
                };
                view! {
                    <p class="flex justify-between gap-4 border-b border-line py-2.5 text-sm">
                        <span>{format::day_month(&payment.due_on)}</span>
                        <strong class=class>{text}</strong>
                    </p>
                }
            })
            .collect_view();
        view! { <div class="mt-3 border-t border-line">{lines}</div> }
    });
    let (name, amount_max, currency) = bill.with_value(|b| {
        (
            b.name.clone(),
            owed.unwrap_or(b.amount_max),
            b.currency.clone(),
        )
    });
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
                    <div class=INLINE_TOOLS>
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
                        "Remove"
                    </Button>
                }
                .into_any()
            }
        }
    };
    let editing_label = editing_this.clone();
    let editor = move || editing_this().then(|| bill_editor(ui, bill));
    view! {
        <Disclosure
            kind=DisclosureKind::Card
            class=cn([BOARD_CARD, "open:col-span-full"])
            summary=view! {
                <span class=NAME_ROW>
                    <h3 class=CARD_NAME>{name}</h3>
                    <strong class=FIGURE>
                        <Money value=amount_max unit=currency neutral=true />
                    </strong>
                </span>
                <span class=META_LINE>
                    <span>{frequency}</span>
                    <span class="whitespace-nowrap">{next_day}</span>
                </span>
            }
        >
            <FactsRow class="mt-2" rows=subtitle />
            <div class="mt-3 flex flex-wrap items-center gap-2">
                <Button
                    variant=Variant::Secondary
                    disabled=Signal::derive(move || ui.edit_busy.get() || stale)
                    on_click=Callback::new(toggle_editor)
                >
                    {move || if editing_label() { "Close" } else { "Edit" }}
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
        </Disclosure>
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
        <form class=ADD_FORM on:submit=submit>
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
                    <span class=TRUNCATE>{category}</span>
                    {elsewhere.map(|name| view! { <small class="col-start-2 text-eyebrow text-fg-muted">{name}</small> })}
                </label>
            }
        })
        .collect_view();
    let busy_label = busy.clone();
    view! {
        <Disclosure class=MORE_DETAILS summary_class="text-sm font-semibold" summary=summary>
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
        </Disclosure>
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
                <div class=LIST_STACK>
                    <strong>"Possible payments to link"</strong>
                    {cards}
                </div>
            }
        })
    }
}

#[cfg(test)]
mod board_tests {
    use super::*;

    #[test]
    fn a_bill_payment_lands_in_the_column_of_its_state() {
        let today = "2026-11-20";
        assert_eq!(bill_column(true, true, "2026-11-01", today), Column::Done);
        assert_eq!(bill_column(false, true, "2026-11-01", today), Column::Check);
        assert_eq!(bill_column(false, false, "2026-11-19", today), Column::Late);
        assert_eq!(bill_column(false, false, "2026-11-20", today), Column::Due);
        assert_eq!(plan_column(PlanStatus::Pending), Column::Due);
        assert_eq!(plan_column(PlanStatus::Overdue), Column::Late);
    }
}
