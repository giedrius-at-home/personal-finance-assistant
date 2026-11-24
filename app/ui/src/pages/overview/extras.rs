//! One-off extra income and payments of the budget month: their totals.
//! They are added and removed in Plan.

use finance_types::gateway::Direction;
use finance_types::report::Dashboard;
use finance_types::report::budget::MonthlyBudget;
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{BarTone, ProgressBar};
use crate::ds::patterns::ROW_LIST;
use crate::ds::{
    CardTone, Eyebrow, Facts, Heading, Icon, Lucide, Metric, Money, Size, Variant, button_class,
    card_class, cn, money,
};
use crate::pages::f;
use crate::{Tab, state};

fn is_salary(category: &str) -> bool {
    category.trim().to_lowercase() == "salary"
}

#[component]
pub fn BudgetExtras(
    report: Dashboard,
    budget: MonthlyBudget,
    available: Option<Decimal>,
) -> impl IntoView {
    let month = budget.plan.month[..7].to_owned();
    let unit = budget.plan.currency.clone();
    let m = {
        let unit = unit.clone();
        move |v: Decimal| money(v, &unit)
    };
    let through = report.monthly_spending.through.clone();
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
    let totals = |direction: Direction, completed: bool| -> Decimal {
        plans
            .iter()
            .filter(|p| p.direction == direction && (p.status == PlanStatus::Paid) == completed)
            .map(|p| p.amount.amount)
            .sum()
    };
    let expected = totals(Direction::Inflow, false);

    let summaries = [Direction::Inflow, Direction::Outflow]
        .into_iter()
        .map(|direction| {
            let inflow = direction == Direction::Inflow;
            let completed = totals(direction, true);
            let outstanding = totals(direction, false);
            let total = completed + outstanding;
            view! {
                <div class=ROW_LIST>
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
                    <Facts rows=vec![
                        (if inflow { "Received" } else { "Paid" }.to_owned(), m(completed)),
                        (if inflow { "Expected" } else { "Remaining" }.to_owned(), m(outstanding)),
                    ] />
                </div>
            }
        })
        .collect_view();

    let projected = (expected > Decimal::ZERO)
        .then_some(available)
        .flatten()
        .map(|a| {
            let unit = unit.clone();
            view! {
                <Metric label="Projected available in this plan" size="text-xl" class="my-4">
                    <Money value=a.round_dp(u32::from(unit.minor_unit)) unit=unit neutral=true />
                </Metric>
            }
        });

    let app = state();
    view! {
        <section class=card_class(CardTone::Default) aria-label="Extra income and payments">
            <header class="flex flex-col items-start justify-between gap-3 sm:flex-row sm:items-center">
                <div>
                    <Eyebrow level=Heading::P>"THIS MONTH ONLY"</Eyebrow>
                    <h3 class="mb-3.5 mt-1.5 text-base font-medium">"Extra income & payments"</h3>
                </div>
                <button
                    type="button"
                    class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), "shrink-0 text-accent"])
                    on:click=move |_| app.go(Tab::Plan)
                >
                    "Add or remove in Plan "
                    <Lucide icon=Icon::ArrowRight size=14 />
                </button>
            </header>
            <div class="grid gap-6 py-4 sm:grid-cols-2">{summaries}</div>
            {projected}
        </section>
    }
}
