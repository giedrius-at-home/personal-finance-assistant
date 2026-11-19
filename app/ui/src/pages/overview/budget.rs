//! The monthly budget (`monthly-budget.tsx`, owner view): month picker,
//! totals, outlook panels, visuals, one expandable row per envelope, the
//! budget group editor (→ `save_budget_plan`) and the extras.

use std::collections::BTreeSet;
use std::str::FromStr;

use finance_types::gateway::{BudgetGroup, BudgetPlan};
use finance_types::report::Dashboard;
use finance_types::report::budget::{BudgetPurchase, BudgetRow, MonthlyBudget};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::api::invoke;
use crate::ds::charts::{BudgetBar, ProgressBar, fmt};
use crate::ds::sync_value;
use crate::ds::{
    CONTROL, Card, CardTone, Checkbox, Eyebrow, Heading, Icon, Lucide, Select, Size, Variant,
    button_class, cn, money,
};
use crate::format;
use crate::pages::{f, with_report};
use crate::{Tab, state};

use super::extras::{BudgetExtras, ExtrasState};
use super::forecast::DailyForecastPanel;
use super::month_label;
use super::outlook::SavingsOutlookPanel;
use super::visuals::{BUDGET_COLORS, BudgetVisuals};

#[wasm_bindgen(inline_js = "
export function random_uuid() {
  if (crypto.randomUUID) return crypto.randomUUID();
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 15) | 64; b[8] = (b[8] & 63) | 128;
  const h = [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
")]
extern "C" {
    fn random_uuid() -> String;
}

#[derive(Serialize)]
struct PlanArgs {
    plan: BudgetPlan,
}

/// The group being added or edited, against the plan it belongs to.
#[derive(Clone, Debug, PartialEq)]
struct Editor {
    plan: BudgetPlan,
    adding: bool,
    account_id: String,
    id: String,
    name: String,
    amount: String,
    source_groups: Vec<String>,
    categories: Vec<String>,
    transaction_ids: Vec<String>,
    /// Cut order: blank never cuts, 1 protects most.
    importance: String,
    /// The lowest the group may be cut to; blank is 0.
    floor: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    SourceGroups,
    Categories,
    Purchases,
}

impl Editor {
    fn list(&self, pick: Pick) -> &Vec<String> {
        match pick {
            Pick::SourceGroups => &self.source_groups,
            Pick::Categories => &self.categories,
            Pick::Purchases => &self.transaction_ids,
        }
    }

    fn list_mut(&mut self, pick: Pick) -> &mut Vec<String> {
        match pick {
            Pick::SourceGroups => &mut self.source_groups,
            Pick::Categories => &mut self.categories,
            Pick::Purchases => &mut self.transaction_ids,
        }
    }

    fn name_taken(&self) -> bool {
        let name = self.name.trim().to_lowercase();
        self.plan
            .groups
            .iter()
            .any(|g| g.id != self.id && g.name.to_lowercase() == name)
    }

    /// `/^\d+(\.\d+)?$/` with no more decimals than the currency has.
    fn valid_amount(&self) -> bool {
        let text = self.amount.as_str();
        let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
        !whole.is_empty()
            && !fraction.is_empty()
            && whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
            && Decimal::from_str(text)
                .is_ok_and(|d| d.normalize().scale() <= u32::from(self.plan.currency.minor_unit))
    }

    /// Blank, or a whole number from 1 to 255.
    fn parsed_importance(&self) -> Result<Option<u8>, ()> {
        match self.importance.trim() {
            "" => Ok(None),
            text => match text.parse::<u8>() {
                Ok(n) if n >= 1 => Ok(Some(n)),
                _ => Err(()),
            },
        }
    }

    /// Blank, or a non-negative amount.
    fn parsed_floor(&self) -> Result<Option<Decimal>, ()> {
        match self.floor.trim() {
            "" => Ok(None),
            text => match Decimal::from_str(text) {
                Ok(d) if d >= Decimal::ZERO => Ok(Some(d)),
                _ => Err(()),
            },
        }
    }

    fn valid_ranking(&self) -> bool {
        self.parsed_importance().is_ok() && self.parsed_floor().is_ok()
    }

    /// The other group of this plan that already uses `value`.
    fn assigned(&self, pick: Pick, value: &str) -> Option<String> {
        self.plan
            .groups
            .iter()
            .find(|g| {
                g.id != self.id
                    && match pick {
                        Pick::SourceGroups => &g.source_groups,
                        Pick::Categories => &g.categories,
                        Pick::Purchases => &g.transaction_ids,
                    }
                    .iter()
                    .any(|v| v == value)
            })
            .map(|g| g.name.clone())
    }
}

/// The panel's own state; it outlives report refreshes so notices stay.
#[derive(Clone, Copy, Debug)]
struct PanelState {
    selection: RwSignal<String>,
    editor: RwSignal<Option<Editor>>,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    notice: RwSignal<String>,
    confirm_remove: RwSignal<bool>,
    add_button: NodeRef<html::Button>,
    extras: ExtrasState,
    /// The spending trajectory's group (`all` for every group).
    trajectory: RwSignal<String>,
}

impl PanelState {
    fn edit(&self, plan: &BudgetPlan, group: Option<&BudgetGroup>) {
        self.error.set(String::new());
        self.notice.set(String::new());
        self.confirm_remove.set(false);
        self.editor.set(Some(Editor {
            plan: plan.clone(),
            adding: group.is_none(),
            account_id: group.and_then(|g| g.account_id.clone()).unwrap_or_default(),
            id: group.map_or_else(random_uuid, |g| g.id.clone()),
            name: group.map(|g| g.name.clone()).unwrap_or_default(),
            amount: group.map(|g| g.amount.to_string()).unwrap_or_default(),
            source_groups: group.map(|g| g.source_groups.clone()).unwrap_or_default(),
            categories: group.map(|g| g.categories.clone()).unwrap_or_default(),
            transaction_ids: group.map(|g| g.transaction_ids.clone()).unwrap_or_default(),
            importance: group
                .and_then(|g| g.importance)
                .map(|n| n.to_string())
                .unwrap_or_default(),
            floor: group
                .and_then(|g| g.floor)
                .map(|f| f.to_string())
                .unwrap_or_default(),
        }));
    }

    fn close(&self) {
        self.editor.set(None);
        self.confirm_remove.set(false);
        self.error.set(String::new());
        if let Some(button) = self.add_button.get_untracked() {
            let _ = button.focus();
        }
    }

    fn save(self, remove: bool) {
        let Some(editor) = self.editor.get_untracked() else {
            return;
        };
        if self.busy.get_untracked() {
            return;
        }
        let minor = u32::from(editor.plan.currency.minor_unit);
        let amount = if remove {
            Decimal::ZERO
        } else {
            let mut d = Decimal::from_str(&editor.amount)
                .unwrap_or_default()
                .round_dp(minor);
            d.rescale(minor);
            d
        };
        let group = BudgetGroup {
            id: editor.id.clone(),
            name: editor.name.trim().to_owned(),
            amount,
            source_groups: editor.source_groups.clone(),
            categories: editor.categories.clone(),
            transaction_ids: editor.transaction_ids.clone(),
            account_id: (!editor.account_id.is_empty()).then(|| editor.account_id.clone()),
            importance: editor.parsed_importance().unwrap_or_default(),
            floor: editor.parsed_floor().unwrap_or_default().map(|mut f| {
                f = f.round_dp(minor);
                f.rescale(minor);
                f
            }),
        };
        let mut plan = editor.plan.clone();
        if remove {
            plan.groups.retain(|g| g.id != group.id);
        } else if editor.adding {
            plan.groups.push(group);
        } else {
            for g in &mut plan.groups {
                if g.id == group.id {
                    *g = group.clone();
                }
            }
        }
        let month = month_label(&plan.month);
        self.busy.set(true);
        self.error.set(String::new());
        let app = state();
        spawn_local(async move {
            match invoke::<BudgetPlan>("save_budget_plan", &PlanArgs { plan }).await {
                Ok(_) => {
                    self.editor.set(None);
                    self.confirm_remove.set(false);
                    self.notice.set(if remove {
                        "Budget group removed. Its spending remains visible.".into()
                    } else {
                        format!("Saved for {month}.")
                    });
                    app.refresh();
                    if let Some(button) = self.add_button.get_untracked() {
                        let _ = button.focus();
                    }
                }
                Err(e) => self.error.set(if e.is_empty() {
                    "Could not save this budget.".into()
                } else {
                    e
                }),
            }
            self.busy.set(false);
        });
    }
}

#[component]
pub fn MonthlyBudgetPanel() -> impl IntoView {
    let s = PanelState {
        selection: RwSignal::new(String::new()),
        editor: RwSignal::new(None),
        busy: RwSignal::new(false),
        error: RwSignal::new(String::new()),
        notice: RwSignal::new(String::new()),
        confirm_remove: RwSignal::new(false),
        add_button: NodeRef::new(),
        extras: ExtrasState::new(),
        trajectory: RwSignal::new("all".to_owned()),
    };
    with_report(move |report| {
        let report = StoredValue::new(report.clone());
        // Only the month picker re-renders the panel; editing is fine-grained.
        move || {
            let selection = s.selection.get();
            report.with_value(|r| panel(r, &selection, s))
        }
    })
}

fn key(budget: &MonthlyBudget) -> String {
    format!("{}:{}", budget.plan.month, budget.plan.currency.code)
}

fn panel(report: &Dashboard, selection: &str, s: PanelState) -> AnyView {
    let budgets = &report.monthly_budgets;
    let Some(budget) = budgets
        .iter()
        .find(|b| key(b) == selection)
        .or_else(|| budgets.first())
    else {
        return view! {
            <Card class="grid gap-2">
                <h2 class="text-lg font-semibold">"Monthly budget"</h2>
                <p class="text-sm text-fg-muted">"Monthly budgets are unavailable. Refresh after the finance service is updated."</p>
            </Card>
        }
        .into_any();
    };
    let plan = budget.plan.clone();
    let unit = plan.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let current_month = format!(
        "{}-01",
        &report.monthly_spending.through[..7.min(report.monthly_spending.through.len())]
    );
    let is_current = plan.month == current_month;
    let allocation = report
        .monthly_allocation
        .as_ref()
        .and_then(|a| a.currencies.iter().find(|c| c.currency.code == unit.code))
        .cloned();
    let savings = allocation.as_ref().map(|a| {
        if is_current {
            a.current_wiggle_room
        } else if a.next_shortfall > Decimal::ZERO {
            -a.next_shortfall
        } else {
            a.next_wiggle_room
        }
    });

    // Choices for the editor.
    let groups: BTreeSet<String> = report
        .time_trends
        .currencies
        .iter()
        .flat_map(|c| c.groups.iter().filter_map(|g| g.group.clone()))
        .chain(report.custom_spending_groups.iter().cloned())
        .chain(
            plan.groups
                .iter()
                .flat_map(|g| g.source_groups.iter().cloned()),
        )
        .collect();
    let categories: BTreeSet<String> = report
        .monthly_spending
        .mappings
        .iter()
        .map(|m| m.category.clone())
        .chain(report.monthly_spending.currencies.iter().flat_map(|c| {
            c.groups
                .iter()
                .flat_map(|g| g.stores.iter().filter_map(|s| s.category.clone()))
        }))
        .chain(
            plan.groups
                .iter()
                .flat_map(|g| g.categories.iter().cloned()),
        )
        .collect();
    let accounts: Vec<(String, String)> = report
        .cash_overview
        .accounts
        .iter()
        .map(|a| (a.account_id.clone(), a.name.clone()))
        .collect();

    let options: Vec<(String, String)> = budgets
        .iter()
        .map(|b| {
            (
                key(b),
                format!("{} · {}", month_label(&b.plan.month), b.plan.currency.code),
            )
        })
        .collect();
    let picked = RwSignal::new(key(budget));
    Effect::new(move |first: Option<()>| {
        let value = picked.get();
        if first.is_some() {
            s.selection.set(value);
            s.notice.set(String::new());
            s.error.set(String::new());
            s.extras.reset();
        }
    });

    let savings_label = if savings.is_some_and(|v| v < Decimal::ZERO) {
        "Funding shortfall"
    } else if is_current {
        "Available to save"
    } else {
        "Unallocated next month"
    };
    let totals = [
        ("Planned", m(budget.planned)),
        ("Spent", m(budget.spent)),
        (savings_label, savings.map_or("—".to_owned(), &m)),
    ]
    .into_iter()
    .map(|(label, value)| {
        view! {
            <div class="grid gap-1.5">
                <span class="text-xs text-fg-muted">{label}</span>
                <strong class="text-base font-semibold tabular-nums sm:text-2xl">{value}</strong>
            </div>
        }
    })
    .collect_view();

    // Expected income is shown apart so the headline never counts money that has not arrived.
    let income_notes = allocation.as_ref().map(|a| {
        if is_current {
            let expected = (a.current_expected_income > Decimal::ZERO).then(|| {
                view! {
                    <p>{format!(
                        "+{} if the expected extra income arrives, {} in total. Overdue income needs a new expected date.",
                        m(a.current_expected_income),
                        m(a.current_planned_wiggle_room),
                    )}</p>
                }
            });
            let by_month_end = a.current_wiggle_room.max(Decimal::ZERO) + a.savings_balance;
            view! {
                <div class="grid gap-1 px-5 pt-4 text-xs leading-relaxed text-fg-muted sm:px-7">
                    {expected}
                    <p>{format!(
                        "{} already in savings; {} by month end if you save the rest and spend within plan.",
                        m(a.savings_balance),
                        m(by_month_end),
                    )}</p>
                </div>
            }
            .into_any()
        } else {
            view! {
                <p class="px-5 pt-4 text-xs leading-relaxed text-fg-muted sm:px-7">
                    {format!(
                        "Includes {} expected extra income, including reimbursements. Received income is already in your bank balance. Overdue income needs a new expected date.",
                        m(a.next_expected_income),
                    )}
                </p>
            }
            .into_any()
        }
    });

    let code = unit.code.clone();
    let daily = is_current
        .then(|| view! { <DailyForecastPanel report=report.clone() currency_code=code.clone() /> });
    let editing = move || s.busy.get() || s.editor.with(Option::is_some);
    let plan_for_add = plan.clone();
    let add_button = view! {
        <button
            type="button"
            node_ref=s.add_button
            class=button_class(Variant::Primary, Size::Md)
            disabled=editing
            on:click=move |_| s.edit(&plan_for_add, None)
        >
            <Lucide icon=Icon::Plus size=16 />
            " Add budget group"
        </button>
    };

    let rows = {
        let mut rows = vec![budget_row(budget, &budget.fixed, Kind::Fixed, s)];
        rows.extend(
            budget
                .groups
                .iter()
                .map(|row| budget_row(budget, row, Kind::Group, s)),
        );
        if budget.extras.planned > Decimal::ZERO || budget.extras.spent > Decimal::ZERO {
            rows.push(budget_row(budget, &budget.extras, Kind::Extras, s));
        }
        rows.push(budget_row(budget, &budget.unplanned, Kind::Unplanned, s));
        rows
    };

    let editor_form = editor_form(
        budget.purchases.clone(),
        groups.into_iter().collect(),
        categories.into_iter().collect(),
        accounts,
        s,
    );

    view! {
        <section class="mb-6 overflow-hidden rounded-xl border border-border bg-card-strong text-fg glass-card" aria-label="Monthly budget">
            <header class="flex flex-col items-start justify-between gap-5 p-5 sm:flex-row sm:items-center sm:px-7 sm:py-6">
                <div>
                    <Eyebrow level=Heading::P>"YOUR PLAN"</Eyebrow>
                    <h2 class="mt-1 text-2xl font-semibold tracking-tight">"Monthly budget"</h2>
                </div>
                <div class="flex w-full flex-wrap items-center gap-3 sm:w-auto">
                    <div class="min-w-48 flex-1 sm:flex-none">
                        <Select
                            label="Budget month and currency"
                            value=picked
                            options=options
                            disabled=Signal::derive(editing)
                        />
                    </div>
                    {add_button}
                </div>
            </header>
            <div class="grid grid-cols-3 gap-3 border-y border-border bg-card-muted px-5 py-4 sm:gap-6 sm:px-7 sm:py-5">
                {totals}
            </div>
            {income_notes}
            <div class="grid gap-4 p-5 sm:px-7">
                <SavingsOutlookPanel report=report.clone() currency_code=code.clone() />
                {daily}
            </div>
            <BudgetVisuals budget=budget.clone() allocation=allocation.clone() current_month=current_month.clone() selection=s.trajectory />
            {move || {
                let notice = s.notice.get();
                (!notice.is_empty()).then(|| view! { <p class="mx-5 my-4 text-sm sm:mx-7" role="status">{notice}</p> })
            }}
            {move || {
                let error = s.error.get();
                (!error.is_empty())
                    .then(|| view! { <p class="mx-5 my-4 text-sm text-negative-fg sm:mx-7" role="alert">{error}</p> })
            }}
            {editor_form}
            <div
                class="hidden grid-cols-[minmax(0,1.8fr)_repeat(3,minmax(105px,1fr))] gap-4 border-b border-border px-7 py-4 text-xs text-fg-muted sm:grid [&>:not(:first-child)]:text-right"
                aria-hidden="true"
            >
                <span>"Budget group"</span>
                <span>"Planned"</span>
                <span>"Spent"</span>
                <span>"Remaining"</span>
            </div>
            {rows}
            <BudgetExtras report=report.clone() budget=budget.clone() available=savings state=s.extras />
            <footer class="px-5 py-4 text-xs leading-relaxed text-fg-muted sm:px-7">
                "Budgets reserve money for the month. Spending includes refunds; transfers are excluded."
            </footer>
        </section>
    }
    .into_any()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fixed,
    Group,
    Unplanned,
    Extras,
}

fn budget_row(budget: &MonthlyBudget, row: &BudgetRow, kind: Kind, s: PanelState) -> AnyView {
    let app = state();
    let plan = budget.plan.clone();
    let unit = plan.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let over = kind == Kind::Group && row.remaining < Decimal::ZERO;
    let group = plan.groups.iter().find(|g| g.id == row.id).cloned();
    let width = if row.planned > Decimal::ZERO {
        (f(row.spent) / f(row.planned) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let color = match kind {
        Kind::Fixed => "var(--ds-chart-fixed)".to_owned(),
        Kind::Unplanned => "var(--ds-chart-limit)".to_owned(),
        _ => {
            let index = plan.groups.iter().position(|g| g.id == row.id).unwrap_or(0);
            BUDGET_COLORS[index % BUDGET_COLORS.len()].to_owned()
        }
    };
    let category_total = row
        .categories
        .iter()
        .map(|c| f(c.spent).max(0.0))
        .sum::<f64>()
        .max(1.0);
    let target = if kind == Kind::Unplanned {
        None
    } else {
        Some(row.id.as_str())
    };
    let purchases: Vec<BudgetPurchase> = budget
        .purchases
        .iter()
        .filter(|p| p.budget_group_id.as_deref() == target)
        .cloned()
        .collect();
    let subtitle = match kind {
        Kind::Fixed => "Bills & recurring payments".to_owned(),
        Kind::Extras => "One-off payments outside group allowances".into(),
        Kind::Unplanned if !row.expected.is_empty() => {
            "Outside your planned groups · planned is spent so far plus the expected rest".into()
        }
        Kind::Unplanned => "Outside your planned groups".into(),
        Kind::Group => match &group {
            Some(g) if g.source_groups.iter().any(|n| *n != row.name) => {
                g.source_groups.join(" · ")
            }
            _ => "Monthly allowance".into(),
        },
    };
    let cell = "sm:text-right sm:tabular-nums before:mb-1 before:block before:text-xs before:font-normal before:text-fg-muted before:content-[attr(data-label)] sm:before:hidden";
    // The unplanned row has a plan only when the outlook estimates one.
    let no_plan = kind == Kind::Unplanned && row.expected.is_empty();
    let planned = if no_plan {
        "—".to_owned()
    } else {
        m(row.planned)
    };
    let remaining = if no_plan {
        "—".to_owned()
    } else if over {
        format!("{} over", m(row.remaining.abs()))
    } else {
        m(row.remaining)
    };
    let progress = (kind == Kind::Group).then(|| {
        let color = color.clone();
        view! {
            <span class="mt-2.5 block h-1 max-w-44 overflow-hidden rounded-pill bg-soft">
                <i
                    class=if over { "block h-full rounded-pill bg-negative" } else { "block h-full rounded-pill " }
                    style:width=format!("{width}%")
                    style:background=(!over).then_some(color)
                ></i>
            </span>
        }
    });
    let edit = group.clone().map(|g| {
        let editing = move || s.busy.get() || s.editor.with(Option::is_some);
        let plan = plan.clone();
        view! {
            <div class="flex items-center justify-between gap-3 text-sm text-fg-muted">
                <span>"Spending by category"</span>
                <button
                    type="button"
                    class=button_class(Variant::Secondary, Size::Sm)
                    disabled=editing
                    on:click=move |_| s.edit(&plan, Some(&g))
                >
                    <Lucide icon=Icon::Pencil size=14 />
                    " Edit group"
                </button>
            </div>
        }
    });
    let used = (group.is_some() && row.planned > Decimal::ZERO).then(|| {
        let unit = unit.clone();
        view! {
            <BudgetBar
                class="mt-3"
                label="Budget used"
                spent=f(row.spent)
                limit=f(row.planned)
                format=fmt(move |n| money(Decimal::try_from(n).unwrap_or_default(), &unit))
            />
        }
    });
    let categories = if row.categories.is_empty() {
        view! {
            <p class="mt-2 text-xs text-fg-muted">
                {if kind == Kind::Unplanned { "All recorded spending is covered by your groups." } else { "No spending recorded yet." }}
            </p>
        }
        .into_any()
    } else {
        view! {
            <ul class="mb-3 mt-2 list-none p-0">
                {row
                    .categories
                    .iter()
                    .map(|c| {
                        let detail = format!(
                            "{} transaction{}{}",
                            c.count,
                            if c.count == 1 { "" } else { "s" },
                            if c.spent < Decimal::ZERO { " · Refund" } else { "" },
                        );
                        view! {
                            <li class="flex justify-between gap-5 border-b border-border py-2.5 text-sm last:border-b-0">
                                <span class="min-w-0 max-w-md flex-1">
                                    <ProgressBar
                                        label=c.category.clone().unwrap_or_else(|| "Uncategorized".into())
                                        value=f(c.spent).max(0.0)
                                        max=category_total
                                    />
                                    <small class="mt-1 block text-xs text-fg-muted">{detail}</small>
                                </span>
                                <strong class="whitespace-nowrap text-right font-medium tabular-nums">{m(c.spent)}</strong>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        }
        .into_any()
    };
    let expected = (!row.expected.is_empty()).then(|| {
        view! {
            <ul class="mb-3 mt-2 list-none p-0" aria-label="Estimates still to pay">
                {row
                    .expected
                    .iter()
                    .map(|(label, amount)| {
                        view! {
                            <li class="flex justify-between gap-5 border-b border-border py-2.5 text-sm last:border-b-0">
                                <span>
                                    {label.clone()}
                                    <small class="mt-1 block text-xs text-fg-muted">"Estimate from past months"</small>
                                </span>
                                <strong class="whitespace-nowrap font-medium tabular-nums">{m(*amount)}</strong>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        }
    });
    let bills_link = (kind == Kind::Fixed).then(|| {
        view! {
            <a
                class="mt-3 block text-sm underline underline-offset-2"
                href="?view=fixed-spending"
                on:click=move |e| {
                    e.prevent_default();
                    app.go(Tab::Plan);
                }
            >
                "Manage bills & recurring payments"
            </a>
        }
    });
    let purchase_list = (!purchases.is_empty()).then(|| {
        let count = purchases.len();
        view! {
            <details class="mt-3">
                <summary class="cursor-pointer text-sm">{format!("Purchases ({count})")}</summary>
                <ul class="mb-3 mt-2 list-none p-0">
                    {purchases
                        .into_iter()
                        .map(|p| {
                            view! {
                                <li class="flex justify-between gap-5 border-b border-border py-2.5 text-sm last:border-b-0">
                                    <span>
                                        {p.description.clone()}
                                        <small class="mt-1 block text-xs text-fg-muted">{p.occurred_on.clone()}</small>
                                    </span>
                                    <strong class="whitespace-nowrap font-medium tabular-nums">{m(p.amount)}</strong>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </details>
        }
    });
    let history = budget
        .history_months
        .iter()
        .enumerate()
        .map(|(i, month)| {
            view! {
                <span class="grid gap-2 text-xs">
                    <small class="text-fg-muted">{format::month_short(&month[..7.min(month.len())])}</small>
                    <strong class="font-medium tabular-nums">{m(row.history.get(i).copied().unwrap_or_default())}</strong>
                </span>
            }
        })
        .collect_view();
    view! {
        <details class="group border-b border-border">
            <summary class="grid cursor-pointer list-none grid-cols-3 gap-x-2 gap-y-4 px-5 py-4 text-sm hover:bg-card-muted focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring sm:grid-cols-[minmax(0,1.8fr)_repeat(3,minmax(105px,1fr))] sm:items-center sm:gap-4 sm:px-7 [&::-webkit-details-marker]:hidden">
                <span class="col-span-3 flex min-w-0 items-center gap-2.5 sm:col-span-1">
                    <Lucide icon=Icon::ChevronDown size=16 class="-rotate-90 text-fg-muted group-open:rotate-0" />
                    <span class="min-w-0 flex-1">
                        <strong class="font-semibold">
                            <i class="mr-2 inline-block size-2 rounded-[2px] align-middle" style:background=color></i>
                            {row.name.clone()}
                        </strong>
                        <small class="mt-1 block text-xs leading-snug text-fg-muted">{subtitle}</small>
                        {progress}
                    </span>
                </span>
                <span data-label="Planned" class=cell>{planned}</span>
                <span data-label="Spent" class=cell>{m(row.spent)}</span>
                <span data-label="Remaining" class=cn(["font-semibold", cell, if over { "text-negative-fg" } else { "" }])>{remaining}</span>
            </summary>
            <div class="px-5 pb-5 pt-1.5 sm:pl-[54px] sm:pr-7">
                {edit}
                {used}
                {categories}
                {expected}
                {bills_link}
                {purchase_list}
                <details class="mt-3">
                    <summary class="cursor-pointer text-sm">"Spending history"</summary>
                    <p class="mt-2 text-xs text-fg-muted">"Using this group's current category selection."</p>
                    <div class="mt-3.5 grid grid-cols-3 gap-3 sm:grid-cols-6">{history}</div>
                </details>
            </div>
        </details>
    }
    .into_any()
}

const CHOICE: &str = "flex items-start gap-2 rounded-md border border-border p-2 text-xs has-[button:disabled]:opacity-45";
const CHOICE_HINT: &str = "mt-1 block text-[0.68rem] text-fg-muted";

/// One checkbox choice of the editor; `other` names the group that already has it.
fn choice(
    pick: Pick,
    value: String,
    label: String,
    hint: Option<String>,
    other: Option<String>,
    with_title: bool,
    s: PanelState,
) -> impl IntoView {
    let checked = {
        let value = value.clone();
        Signal::derive(move || {
            s.editor
                .with(|e| e.as_ref().is_some_and(|e| e.list(pick).contains(&value)))
        })
    };
    let toggle = Callback::new(move |_: bool| {
        s.editor.update(|e| {
            if let Some(e) = e {
                let list = e.list_mut(pick);
                if let Some(i) = list.iter().position(|v| *v == value) {
                    list.remove(i);
                } else {
                    list.push(value.clone());
                }
            }
        });
    });
    let disabled = other.is_some();
    let title = other
        .as_ref()
        .filter(|_| with_title)
        .map(|o| format!("Already in {o}"));
    let small = match (hint, &other) {
        (Some(h), Some(o)) => Some(format!("{h} · In {o}")),
        (Some(h), None) => Some(h),
        (None, Some(o)) => Some(format!("In {o}")),
        (None, None) => None,
    };
    view! {
        <label class=CHOICE title=title>
            <span class="mt-0.5 flex">
                <Checkbox checked=checked on_change=toggle disabled=Signal::derive(move || disabled || s.busy.get()) />
            </span>
            <span>{label}{small.map(|t| view! { <small class=CHOICE_HINT>{t}</small> })}</span>
        </label>
    }
}

fn editor_form(
    purchases: Vec<BudgetPurchase>,
    groups: Vec<String>,
    categories: Vec<String>,
    accounts: Vec<(String, String)>,
    s: PanelState,
) -> impl IntoView {
    let data = StoredValue::new((purchases, groups, categories, accounts));
    view! {
        <Show when=move || s.editor.with(Option::is_some)>
            {move || {
                let Some(editor) = s.editor.get_untracked() else {
                    return ().into_any();
                };
                let (purchases, groups, categories, accounts) = data.get_value();
                let unit = editor.plan.currency.clone();
                let title = if editor.adding { "Add budget group" } else { "Edit budget group" };
                let month = month_label(&editor.plan.month);
                let step = Decimal::new(1, u32::from(unit.minor_unit)).to_string();
                let field = move |f: fn(&Editor) -> String| {
                    move || s.editor.with(|e| e.as_ref().map(f).unwrap_or_default())
                };
                let set = move |f: fn(&mut Editor, String)| {
                    move |ev: leptos::ev::Event| {
                        let v = event_target_value(&ev);
                        s.editor.update(|e| {
                            if let Some(e) = e {
                                f(e, v);
                            }
                        });
                    }
                };
                let name_taken = move || s.editor.with(|e| e.as_ref().is_some_and(Editor::name_taken));
                let can_save = move || {
                    s.editor.with(|e| {
                        e.as_ref().is_some_and(|e| e.valid_amount() && e.valid_ranking() && !e.name.trim().is_empty() && !e.name_taken())
                    })
                };
                let group_choices = groups
                    .into_iter()
                    .map(|name| {
                        let other = editor.assigned(Pick::SourceGroups, &name);
                        choice(Pick::SourceGroups, name.clone(), name, None, other, true, s)
                    })
                    .collect_view();
                let category_choices = categories
                    .into_iter()
                    .map(|name| {
                        let other = editor.assigned(Pick::Categories, &name);
                        choice(Pick::Categories, name.clone(), name, None, other, false, s)
                    })
                    .collect_view();
                let purchase_choices = if purchases.is_empty() {
                    view! { <p>"No purchases recorded for this month yet."</p> }.into_any()
                } else {
                    view! {
                        <div class="grid max-h-64 gap-2 overflow-y-auto">
                            {purchases
                                .into_iter()
                                .map(|p| {
                                    let other = editor.assigned(Pick::Purchases, &p.id);
                                    let hint = format!("{} · {}", p.occurred_on, money(p.amount, &unit));
                                    choice(Pick::Purchases, p.id.clone(), p.description.clone(), Some(hint), other, false, s)
                                })
                                .collect_view()}
                        </div>
                    }
                    .into_any()
                };
                let account_options = accounts
                    .into_iter()
                    .map(|(id, name)| {
                        let selected = id == editor.account_id;
                        view! { <option value=id selected=selected>{name}</option> }
                    })
                    .collect_view();
                let remove_button = (!editor.adding).then(|| {
                    view! {
                        <button
                            type="button"
                            class=button_class(Variant::Danger, Size::Md)
                            on:click=move |_| s.confirm_remove.set(true)
                        >
                            <Lucide icon=Icon::Trash2 size=14 />
                            " Remove"
                        </button>
                    }
                });
                view! {
                    <Card tone=CardTone::Muted class="mx-3 my-4 sm:mx-7 sm:my-5">
                        <form
                            aria-label=title
                            on:submit=move |e| {
                                e.prevent_default();
                                if can_save() {
                                    s.save(false);
                                }
                            }
                        >
                            <div class="flex items-center justify-between gap-3">
                                <h3 class="text-base font-semibold">{title}</h3>
                                <button
                                    type="button"
                                    aria-label="Close budget editor"
                                    class=button_class(Variant::Ghost, Size::Icon)
                                    disabled=move || s.busy.get()
                                    on:click=move |_| s.close()
                                >
                                    <Lucide icon=Icon::X size=18 />
                                </button>
                            </div>
                            <fieldset disabled=move || s.busy.get() class="m-0 min-w-0 border-0 p-0">
                                <div class="my-4 grid gap-4 sm:grid-cols-[2fr_1fr_1fr]">
                                    <div class="grid gap-1.5">
                                        <label for="budget-group-name" class="text-sm font-medium text-fg">"Name"</label>
                                        <input
                                            id="budget-group-name"
                                            class=CONTROL
                                            autofocus=true
                                            required=true
                                            maxlength="64"
                                            placeholder="e.g. Clothing"
                                            prop:value=field(|e| e.name.clone())
                                            on:input=set(|e, v| e.name = v)
                                        />
                                    </div>
                                    <div class="grid gap-1.5">
                                        <label for="budget-group-amount" class="text-sm font-medium text-fg">
                                            {format!("Planned amount ({})", unit.code)}
                                        </label>
                                        <input
                                            id="budget-group-amount"
                                            class=CONTROL
                                            type="number"
                                            required=true
                                            min="0"
                                            step=step.clone()
                                            placeholder="0.00"
                                            node_ref={
                                                let node = NodeRef::<html::Input>::new();
                                                sync_value(node, field(|e| e.amount.clone()));
                                                node
                                            }
                                            on:input=set(|e, v| e.amount = v)
                                        />
                                    </div>
                                    <div class="grid gap-1.5">
                                        <label for="budget-group-account" class="text-sm font-medium text-fg">"Account"</label>
                                        <div class="relative w-full">
                                            <select
                                                id="budget-group-account"
                                                class=cn([CONTROL, "appearance-none pr-9"])
                                                on:change=set(|e, v| e.account_id = v)
                                            >
                                                <option value="" selected=editor.account_id.is_empty()>"Choose account"</option>
                                                {account_options}
                                            </select>
                                            <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                                                <Lucide icon=Icon::ChevronDown size=16 />
                                            </span>
                                        </div>
                                    </div>
                                </div>
                                                <div class="mb-4 grid gap-4 sm:grid-cols-2">
                                    <div class="grid gap-1.5">
                                        <label for="budget-group-importance" class="text-sm font-medium text-fg">"Cut order"</label>
                                        <input
                                            id="budget-group-importance"
                                            class=CONTROL
                                            type="number"
                                            min="1"
                                            max="255"
                                            step="1"
                                            placeholder="Never cut"
                                            aria-describedby="budget-group-importance-hint"
                                            node_ref={
                                                let node = NodeRef::<html::Input>::new();
                                                sync_value(node, field(|e| e.importance.clone()));
                                                node
                                            }
                                            on:input=set(|e, v| e.importance = v)
                                        />
                                        <p id="budget-group-importance-hint" class="text-xs text-fg-muted">
                                            "Blank = never cut, 1 = protect most. Wishes cut the highest number first."
                                        </p>
                                    </div>
                                    <div class="grid gap-1.5">
                                        <label for="budget-group-floor" class="text-sm font-medium text-fg">
                                            {format!("Lowest ({})", unit.code)}
                                        </label>
                                        <input
                                            id="budget-group-floor"
                                            class=CONTROL
                                            type="number"
                                            min="0"
                                            step=step
                                            placeholder="0.00"
                                            aria-describedby="budget-group-floor-hint"
                                            node_ref={
                                                let node = NodeRef::<html::Input>::new();
                                                sync_value(node, field(|e| e.floor.clone()));
                                                node
                                            }
                                            on:input=set(|e, v| e.floor = v)
                                        />
                                        <p id="budget-group-floor-hint" class="text-xs text-fg-muted">
                                            "A wish never cuts this group below this amount."
                                        </p>
                                    </div>
                                </div>
                                <Show when=name_taken>
                                    <p role="alert" class="text-sm text-negative-fg">"That name is already used this month."</p>
                                </Show>
                                <span class="text-sm">"Include spending from"</span>
                                <p class="text-xs text-fg-muted">"Bills stay under Fixed expenses."</p>
                                <div class="my-3 grid max-h-60 grid-cols-2 gap-2.5 overflow-y-auto sm:grid-cols-3">{group_choices}</div>
                                <details class="mt-4 text-sm">
                                    <summary class="cursor-pointer">"Choose individual categories"</summary>
                                    <p class="text-fg-muted">"A selected category uses this budget instead of its broader group."</p>
                                    <div class="my-3 grid max-h-60 grid-cols-2 gap-2.5 overflow-y-auto sm:grid-cols-3">{category_choices}</div>
                                </details>
                                <details class="mt-4 text-sm">
                                    <summary class="cursor-pointer">"Choose specific purchases"</summary>
                                    <p class="text-fg-muted">"Selected purchases use this budget instead of their category's budget."</p>
                                    {purchase_choices}
                                </details>
                                <div class="mt-5 flex flex-col items-start justify-between gap-3 border-t border-border pt-4 sm:flex-row sm:items-center">
                                    <small class="text-xs text-fg-muted">{format!("For {month} only.")}</small>
                                    <div class="flex flex-wrap items-center gap-1.5 sm:gap-3">
                                        {remove_button}
                                        <button type="button" class=button_class(Variant::Secondary, Size::Md) on:click=move |_| s.close()>
                                            "Cancel"
                                        </button>
                                        <button type="submit" class=button_class(Variant::Primary, Size::Md) disabled=move || !can_save()>
                                            {move || if s.busy.get() { "Saving…" } else { "Save group" }}
                                        </button>
                                    </div>
                                </div>
                                <Show when=move || s.confirm_remove.get()>
                                    <div class="mt-3.5 flex flex-wrap items-center gap-2 text-sm">
                                        <span>"Remove this group? Its purchases will remain in your spending."</span>
                                        <button type="button" class=button_class(Variant::Danger, Size::Sm) on:click=move |_| s.save(true)>
                                            "Remove group"
                                        </button>
                                        <button
                                            type="button"
                                            class=button_class(Variant::Secondary, Size::Sm)
                                            on:click=move |_| s.confirm_remove.set(false)
                                        >
                                            "Keep group"
                                        </button>
                                    </div>
                                </Show>
                            </fieldset>
                        </form>
                    </Card>
                }
                .into_any()
            }}
        </Show>
    }
}
