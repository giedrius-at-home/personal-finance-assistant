//! Where this and next month's money goes (Python domain/monthly_allocation.py).
//!
//! Booked cash funds the current month; a salary received this month is kept
//! aside for the next one, together with the income still expected.

use std::collections::{BTreeSet, HashMap, HashSet};

use finance_types::gateway::{Direction, SavingsGoalRecord, SpendingLimit};
use finance_types::report::Currency;
use finance_types::report::UnitMoney;
use finance_types::report::allocation::{
    BankAllocation, CurrencyAllocation, GroupAllocation, MonthlyAllocation,
};
use finance_types::report::budget::{BudgetRow, MonthlyBudget};
use finance_types::report::cash::PaydayStatus;
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::reporting::GoalEvaluation;
use finance_types::report::subscriptions::SubscriptionReport;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::Ctx;
use crate::closure::day_of;
use crate::model::Account;
use crate::period::{plan_month, shift_month};
use crate::reporting::py_div;
use crate::text::casefold;

/// A spending limit's use so far (a spending nudge), for owners without budgets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupUsage {
    pub currency: Currency,
    pub group: String,
    pub spent: Decimal,
    pub projected_month_end: Decimal,
}

/// Everything the allocation reads.
#[derive(Clone, Copy, Debug)]
pub struct AllocationInputs<'a> {
    /// The cash overview's last day.
    pub through_date: Date,
    pub accounts: &'a [Account],
    pub goals: &'a [SavingsGoalRecord],
    /// Empty unless the owner asked to fund goals from the monthly budget.
    pub goal_evaluations: &'a [GoalEvaluation],
    pub payday_status: Option<&'a PaydayStatus>,
    pub current_subscriptions: &'a SubscriptionReport,
    pub next_subscriptions: &'a SubscriptionReport,
    pub planned_payments: &'a [PlannedPaymentView],
    /// Only used without `budgets`.
    pub spending_limits: &'a [SpendingLimit],
    pub spending_nudges: &'a [GroupUsage],
    pub budgets: Option<&'a [MonthlyBudget]>,
    pub savings_account_ids: &'a HashSet<String>,
    /// The running budget month; the calendar month of `through_date` without it.
    pub current_month: Option<Date>,
    /// Goals held in an excluded savings account.
    pub savings_goal_ids: &'a HashSet<String>,
    /// The local day the current month began; salary counts by calendar
    /// month without it.
    pub period_start: Option<Date>,
}

/// Savings-role account ids and the goals they hold, from the owner's
/// confirmed account roles.
pub fn savings_roles(ctx: &Ctx) -> (HashSet<String>, HashSet<String>) {
    let savings = ctx
        .snap
        .account_roles
        .values()
        .filter(|r| r.role == "savings");
    let accounts = savings.clone().map(|r| r.account_id.clone()).collect();
    let goals = savings
        .filter_map(|r| r.goal_id.clone())
        .filter(|g| !g.is_empty())
        .collect();
    (accounts, goals)
}

fn is_salary(payment: &PlannedPaymentView) -> bool {
    casefold(payment.match_category.trim()) == "salary"
}

fn money_totals(items: &[UnitMoney]) -> HashMap<Currency, Decimal> {
    let mut totals: HashMap<Currency, Decimal> = HashMap::new();
    for money in items {
        *totals.entry(money.currency.clone()).or_default() += money.amount;
    }
    totals
}

fn add(totals: &mut HashMap<Currency, Decimal>, currency: &Currency, amount: Decimal) {
    *totals.entry(currency.clone()).or_default() += amount;
}

fn budget_rows(rows: &[BudgetRow]) -> Vec<GroupAllocation> {
    rows.iter()
        .map(|row| GroupAllocation {
            group: row.name.clone(),
            limit: row.planned,
            spent: row.spent,
            remaining: row.reserved,
            projected_month_end: row.spent,
        })
        .collect()
}

/// Income other than salary booked this budget month into accounts that are
/// not savings: money from elsewhere, not the salary, that this month's
/// spending room already holds. Paid salary plans and salary-category
/// income are left out; money back (refunds) is spending, not income.
pub fn extra_income(
    transactions: &[crate::model::Tx],
    payments: &[PlannedPaymentView],
    savings_account_ids: &HashSet<String>,
    month_of: impl Fn(&crate::model::Tx) -> Date,
    current_month: Date,
) -> HashMap<Currency, Decimal> {
    let salary_ids: HashSet<&str> = payments
        .iter()
        .filter(|p| p.status == PlanStatus::Paid && is_salary(p))
        .filter_map(|p| p.transaction_id.as_deref())
        .collect();
    let mut out: HashMap<Currency, Decimal> = HashMap::new();
    for t in transactions {
        if t.kind != crate::model::Kind::Inflow
            || salary_ids.contains(t.transaction_id.as_str())
            || savings_account_ids.contains(&t.account_id)
            || t.category
                .as_deref()
                .is_some_and(|c| casefold(c.trim()).contains("salary"))
            || month_of(t) != current_month
        {
            continue;
        }
        add(&mut out, &t.currency, t.amount);
    }
    out
}

/// Keep a newly received salary outside the current month's envelopes.
pub fn monthly_allocation(inputs: &AllocationInputs) -> MonthlyAllocation {
    let through_date = inputs.through_date;
    let current_month = inputs
        .current_month
        .unwrap_or_else(|| through_date.first_of_month());
    let in_current_period = |day: Date| match inputs.period_start {
        None => day.first_of_month() == current_month,
        Some(start) => day >= start,
    };
    let funded_month = shift_month(current_month, 1);

    let mut balances: HashMap<Currency, Decimal> = HashMap::new();
    let mut savings_totals: HashMap<Currency, Decimal> = HashMap::new();
    let mut banks: HashMap<Currency, Vec<BankAllocation>> = HashMap::new();
    for account in inputs.accounts {
        let reserved: Decimal = inputs
            .planned_payments
            .iter()
            .filter(|p| {
                p.pending
                    && p.status != PlanStatus::Paid
                    && p.direction == Direction::Outflow
                    && p.account_id.as_deref() == Some(account.account_id.as_str())
                    && &p.amount.currency == account.currency()
                    && day_of(&p.due_on).is_some_and(|d| d <= through_date)
            })
            .map(|p| p.amount.amount)
            .sum();
        // Savings accounts are not spendable, so they never fund the envelopes.
        let savings = inputs.savings_account_ids.contains(&account.account_id);
        let included = account.include_net_worth && !savings;
        let booked = account.balance.amount;
        banks
            .entry(account.currency().clone())
            .or_default()
            .push(BankAllocation {
                account_id: account.account_id.clone(),
                name: account.name.clone(),
                booked,
                reserved,
                available: booked - reserved,
                included,
            });
        if included {
            add(&mut balances, &account.balance.currency, booked);
        } else if savings {
            add(&mut savings_totals, &account.balance.currency, booked);
        }
    }
    let mut protected: HashMap<Currency, Decimal> = HashMap::new();
    for goal in inputs.goals {
        // A goal held in an excluded savings account is already out of the pot.
        if !inputs.savings_goal_ids.contains(&goal.goal.goal_id) {
            add(
                &mut protected,
                &goal.protected_amount.currency,
                goal.protected_amount.amount,
            );
        }
    }

    let mut salary: HashMap<Currency, Decimal> = HashMap::new();
    let mut salary_received_on = None;
    if let Some(status) = inputs.payday_status
        && let Some(received_on) = status.received_on.as_deref().and_then(day_of)
        && in_current_period(received_on)
    {
        salary_received_on = Some(received_on);
        for amount in &status.received_amounts {
            add(&mut salary, &amount.currency, amount.amount);
        }
    }
    if salary.is_empty() {
        let paid_income: Vec<(&PlannedPaymentView, Date)> = inputs
            .planned_payments
            .iter()
            .filter(|p| p.direction == Direction::Inflow && is_salary(p))
            .filter(|p| p.status == PlanStatus::Paid)
            .filter_map(|p| Some((p, p.paid_on.as_deref().and_then(day_of)?)))
            .filter(|(_, paid_on)| in_current_period(*paid_on))
            .collect();
        for (p, _) in &paid_income {
            add(&mut salary, &p.amount.currency, p.amount.amount);
        }
        if let Some(latest) = paid_income.iter().map(|(_, d)| *d).max() {
            salary_received_on = Some(latest);
        }
    }
    let salary_of = |c: &Currency| salary.get(c).copied().unwrap_or_default();

    let current_fixed = money_totals(&inputs.current_subscriptions.remaining_costs);
    let next_fixed = money_totals(&inputs.next_subscriptions.expected_costs);
    let mut current_income: HashMap<Currency, Decimal> = HashMap::new();
    let mut next_income: HashMap<Currency, Decimal> = HashMap::new();
    let mut expected_salary: HashMap<Currency, Decimal> = HashMap::new();
    for p in inputs.planned_payments {
        let Some(due_on) = day_of(&p.due_on) else {
            continue;
        };
        if p.direction != Direction::Inflow
            || p.status != PlanStatus::Upcoming
            || due_on < through_date
        {
            continue;
        }
        let currency = &p.amount.currency;
        let month = plan_month(due_on, current_month, inputs.period_start);
        if is_salary(p) {
            // This month's payday funds next month. A paid salary is already reserved.
            if month == current_month && salary_of(currency).is_zero() {
                add(&mut expected_salary, currency, p.amount.amount);
            }
        } else if month == current_month {
            add(&mut current_income, currency, p.amount.amount);
        } else if month == funded_month {
            add(&mut next_income, currency, p.amount.amount);
        }
    }
    let mut current_plans: HashMap<Currency, Decimal> = HashMap::new();
    let mut next_plans: HashMap<Currency, Decimal> = HashMap::new();
    for p in inputs.planned_payments {
        let Some(due_on) = day_of(&p.due_on) else {
            continue;
        };
        if p.direction != Direction::Outflow || p.status == PlanStatus::Paid {
            continue;
        }
        let month = plan_month(due_on, current_month, inputs.period_start);
        if month == current_month || (p.pending && due_on < current_month) {
            add(&mut current_plans, &p.amount.currency, p.amount.amount);
        } else if month == funded_month {
            add(&mut next_plans, &p.amount.currency, p.amount.amount);
        }
    }

    let mut monthly_goals: HashMap<Currency, Decimal> = HashMap::new();
    for evaluation in inputs.goal_evaluations {
        let monthly = py_div(
            evaluation.required_weekly_contribution * Decimal::from(52),
            Decimal::from(12),
        );
        add(
            &mut monthly_goals,
            &evaluation.currency,
            evaluation.remaining.min(monthly),
        );
    }

    let mut group_rows: HashMap<Currency, Vec<GroupAllocation>> = HashMap::new();
    let mut group_limits: HashMap<Currency, Decimal> = HashMap::new();
    let mut group_remaining: HashMap<Currency, Decimal> = HashMap::new();
    if inputs.budgets.is_none() {
        for saved in inputs.spending_limits {
            let currency = &saved.amount.currency;
            let nudge = inputs
                .spending_nudges
                .iter()
                .find(|n| &n.currency == currency && n.group == saved.group);
            let spent = nudge.map(|n| n.spent).unwrap_or_default();
            let remaining = (saved.amount.amount - spent).max(Decimal::ZERO);
            add(&mut group_limits, currency, saved.amount.amount);
            add(&mut group_remaining, currency, remaining);
            group_rows
                .entry(currency.clone())
                .or_default()
                .push(GroupAllocation {
                    group: saved.group.clone(),
                    limit: saved.amount.amount,
                    spent,
                    remaining,
                    projected_month_end: nudge.map(|n| n.projected_month_end).unwrap_or_default(),
                });
        }
    }
    let mut next_group_rows: HashMap<Currency, Vec<GroupAllocation>> = HashMap::new();
    for budget in inputs.budgets.unwrap_or_default() {
        let currency = &budget.plan.currency;
        let month = day_of(&budget.plan.month);
        if month == Some(current_month) {
            group_remaining.insert(currency.clone(), budget.reserved);
            group_rows.insert(currency.clone(), budget_rows(&budget.groups));
        } else if month == Some(funded_month) {
            // Only that month's plans reserve next month's money.
            group_limits.insert(currency.clone(), budget.reserved);
            next_group_rows.insert(currency.clone(), budget_rows(&budget.groups));
        }
    }

    let currencies: BTreeSet<&Currency> = [
        &balances,
        &protected,
        &salary,
        &current_fixed,
        &next_fixed,
        &current_plans,
        &next_plans,
        &monthly_goals,
        &group_limits,
        &group_remaining,
        &current_income,
        &next_income,
        &expected_salary,
    ]
    .into_iter()
    .flat_map(HashMap::keys)
    .collect();
    let get = |totals: &HashMap<Currency, Decimal>, c: &Currency| {
        totals.get(c).copied().unwrap_or_default()
    };
    let currencies = currencies
        .into_iter()
        .map(|c| {
            let current_funds = get(&balances, c) - get(&protected, c) - get(&salary, c);
            let current_commitments =
                get(&current_fixed, c) + get(&current_plans, c) + get(&group_remaining, c);
            let current_wiggle = current_funds - current_commitments;
            let next_commitments = get(&next_fixed, c)
                + get(&monthly_goals, c)
                + get(&next_plans, c)
                + get(&group_limits, c);
            let next_funds = get(&salary, c) + get(&expected_salary, c) + get(&next_income, c);
            let next_wiggle = next_funds - next_commitments;
            let mut groups = group_rows.get(c).cloned().unwrap_or_default();
            let next_rows = if inputs.budgets.is_some() {
                next_group_rows.get(c).cloned().unwrap_or_default()
            } else {
                groups.clone()
            };
            groups.sort_by(|a, b| a.group.cmp(&b.group));
            let mut bank_rows = banks.get(c).cloned().unwrap_or_default();
            bank_rows.sort_by_cached_key(|b| casefold(&b.name));
            CurrencyAllocation {
                currency: c.clone(),
                current_month: current_month.to_string(),
                funded_month: funded_month.to_string(),
                bank_balance: get(&balances, c),
                protected_savings: get(&protected, c),
                salary_reserved: get(&salary, c),
                salary_received_on: salary_received_on.map(|d| d.to_string()),
                current_month_funds: current_funds,
                current_fixed_remaining: get(&current_fixed, c),
                current_plans_remaining: get(&current_plans, c),
                current_groups_remaining: get(&group_remaining, c),
                current_wiggle_room: current_wiggle,
                salary_at_risk: (-current_wiggle).max(Decimal::ZERO),
                next_fixed: get(&next_fixed, c),
                next_goals: get(&monthly_goals, c),
                next_plans: get(&next_plans, c),
                next_groups: get(&group_limits, c),
                next_wiggle_room: next_wiggle.max(Decimal::ZERO),
                next_shortfall: (-next_wiggle).max(Decimal::ZERO),
                groups,
                banks: bank_rows,
                next_group_rows: next_rows,
                current_expected_income: get(&current_income, c),
                next_expected_income: get(&next_income, c),
                expected_salary: get(&expected_salary, c),
                next_planned_funds: next_funds,
                current_planned_wiggle_room: current_wiggle + get(&current_income, c),
                savings_balance: get(&savings_totals, c),
                current_extra_income: Decimal::ZERO,
            }
        })
        .collect();
    MonthlyAllocation {
        through_date: through_date.to_string(),
        currencies,
    }
}

#[cfg(test)]
mod extra_income_tests {
    use super::*;
    use crate::model::Kind;

    #[test]
    fn only_non_salary_income_into_spending_accounts_counts() {
        let base = crate::budget::tests::tx("gift", "1100");
        let income =
            |id: &str, amount: &str, category: Option<&str>, account: &str| crate::model::Tx {
                transaction_id: id.into(),
                kind: Kind::Inflow,
                amount: amount.parse().unwrap(),
                category: category.map(Into::into),
                account_id: account.into(),
                ..base.clone()
            };
        let rows = vec![
            income("gift", "1100", Some("Gifts"), "daily"),
            income("pay", "3000", None, "daily"),
            income("wage", "50", Some("Salary"), "daily"),
            income("saved", "70", None, "savings"),
            crate::model::Tx {
                kind: Kind::Refund,
                ..income("tikkie", "15", None, "daily")
            },
        ];
        let paid_salary: PlannedPaymentView = serde_json::from_value(serde_json::json!({
            "plan_id": "recurring-salary", "name": "Salary", "match_category": "Salary",
            "direction": "inflow", "amount": {"amount": "3000", "currency": {"code": "EUR", "minor_unit": 2}},
            "due_on": "2026-09-25", "created_at": "2026-09-01T00:00:00Z", "reimbursement_for": null,
            "pending": false, "account_id": null, "match_description": null, "status": "paid",
            "paid_on": "2026-09-25", "transaction_id": "pay", "candidate_count": 0,
            "candidates": [], "by_hand": false
        }))
        .unwrap();
        let month = jiff::civil::date(2026, 9, 1);
        let found = extra_income(
            &rows,
            &[paid_salary],
            &HashSet::from(["savings".to_owned()]),
            |t| t.effective_on().first_of_month(),
            month,
        );
        assert_eq!(found.get(&Currency::eur()), Some(&Decimal::from(1100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn eur(amount: &str) -> UnitMoney {
        UnitMoney {
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
        }
    }

    fn account(id: &str, amount: &str) -> Account {
        Account {
            account_id: id.into(),
            ledger_id: 1,
            name: id.into(),
            balance: eur(amount),
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    fn empty() -> SubscriptionReport {
        SubscriptionReport {
            fetched_at: None,
            through_date: "2026-09-28".into(),
            stale: false,
            rows: Vec::new(),
            expected_costs: Vec::new(),
            remaining_costs: Vec::new(),
            scheduled_payments: Vec::new(),
            unmatched_count: 0,
            evidence_start: "2026-09-28".into(),
            category_matched_names: Vec::new(),
        }
    }

    fn income(status: PlanStatus, due_on: &str) -> PlannedPaymentView {
        let paid = status == PlanStatus::Paid;
        PlannedPaymentView {
            plan_id: "sale".into(),
            name: "Sold a bicycle".into(),
            match_category: "Other income".into(),
            direction: Direction::Inflow,
            amount: eur("100"),
            due_on: due_on.into(),
            created_at: "2026-09-28T12:00:00Z".into(),
            reimbursement_for: None,
            pending: false,
            account_id: None,
            match_description: None,
            status,
            paid_on: paid.then(|| due_on.into()),
            transaction_id: paid.then(|| "sale".into()),
            candidate_count: 0,
            candidates: Vec::new(),
            by_hand: false,
        }
    }

    fn project(
        accounts: &[Account],
        payments: &[PlannedPaymentView],
        through: Date,
        savings: &[&str],
    ) -> CurrencyAllocation {
        let empty = empty();
        let savings_ids: HashSet<String> = savings.iter().map(|s| (*s).to_owned()).collect();
        let no_goals = HashSet::new();
        monthly_allocation(&AllocationInputs {
            through_date: through,
            accounts,
            goals: &[],
            goal_evaluations: &[],
            payday_status: None,
            current_subscriptions: &empty,
            next_subscriptions: &empty,
            planned_payments: payments,
            spending_limits: &[],
            spending_nudges: &[],
            budgets: None,
            savings_account_ids: &savings_ids,
            current_month: None,
            savings_goal_ids: &no_goals,
            period_start: None,
        })
        .currencies
        .remove(0)
    }

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    #[test]
    fn a_plan_due_after_an_early_close_counts_in_the_running_month() {
        // October closed on the 28th: from the 29th the budget runs November.
        let rent = PlannedPaymentView {
            direction: Direction::Outflow,
            ..income(PlanStatus::Upcoming, "2026-10-30")
        };
        let empty = empty();
        let (none, accounts) = (HashSet::new(), [account("bank", "1000")]);
        let row = monthly_allocation(&AllocationInputs {
            through_date: date(2026, 10, 29),
            accounts: &accounts,
            goals: &[],
            goal_evaluations: &[],
            payday_status: None,
            current_subscriptions: &empty,
            next_subscriptions: &empty,
            planned_payments: std::slice::from_ref(&rent),
            spending_limits: &[],
            spending_nudges: &[],
            budgets: None,
            savings_account_ids: &none,
            current_month: Some(date(2026, 11, 1)),
            savings_goal_ids: &none,
            period_start: Some(date(2026, 10, 29)),
        })
        .currencies
        .remove(0);
        assert_eq!(row.current_plans_remaining, d(100));
    }

    #[test]
    fn extra_income_is_not_salary_or_counted_twice() {
        let today = date(2026, 9, 28);
        let bank = [account("bank", "1000")];
        for status in [PlanStatus::Upcoming, PlanStatus::Paid] {
            let upcoming = status == PlanStatus::Upcoming;
            let row = project(&bank, &[income(status, "2026-09-28")], today, &[]);
            assert_eq!(row.salary_reserved, d(0));
            assert_eq!(row.salary_received_on, None);
            assert_eq!(row.current_month_funds, d(1000));
            assert_eq!(row.current_wiggle_room, d(1000));
            assert_eq!(
                row.current_expected_income,
                d(if upcoming { 100 } else { 0 })
            );
            assert_eq!(
                row.current_planned_wiggle_room,
                d(if upcoming { 1100 } else { 1000 })
            );
            assert_eq!(row.next_groups, d(0));
        }
        let extra = income(PlanStatus::Upcoming, "2026-09-30");
        let pending = project(&bank, std::slice::from_ref(&extra), today, &[]);
        let settled = project(
            &[account("bank", "1100")],
            &[PlannedPaymentView {
                status: PlanStatus::Paid,
                ..extra.clone()
            }],
            today,
            &[],
        );
        assert_eq!(pending.current_planned_wiggle_room, d(1100));
        assert_eq!(settled.current_planned_wiggle_room, d(1100));
        assert_eq!(settled.current_expected_income, d(0));

        // Upcoming reimbursements fund their due month, never recurring salary.
        let refund = PlannedPaymentView {
            due_on: "2026-10-03".into(),
            reimbursement_for: Some("purchase".into()),
            ..extra.clone()
        };
        let salary = PlannedPaymentView {
            plan_id: "salary".into(),
            match_category: "Salary".into(),
            amount: eur("3042.27"),
            ..extra.clone()
        };
        let next = project(&bank, &[refund, salary], today, &[]);
        let expected: Decimal = "3142.27".parse().unwrap();
        assert_eq!(next.next_expected_income, d(100));
        assert_eq!(next.expected_salary, "3042.27".parse().unwrap());
        assert_eq!(next.next_planned_funds, expected);
        assert_eq!(next.next_wiggle_room, expected);
        assert_eq!(next.current_planned_wiggle_room, d(1000));
        assert_eq!(next.salary_reserved, d(0));

        // Late or disputed deposits cannot silently become available funds.
        for status in [PlanStatus::Overdue, PlanStatus::NeedsReview] {
            let late = PlannedPaymentView {
                status,
                ..extra.clone()
            };
            assert_eq!(
                project(&bank, &[late], today, &[]).current_expected_income,
                d(0)
            );
        }
        let later = project(&bank, &[extra], date(2026, 10, 1), &[]);
        assert_eq!(later.current_expected_income, d(0));
    }

    #[test]
    fn savings_accounts_are_reported_apart_from_spendable_funds() {
        let accounts = [account("bank", "300.00"), account("vault", "522.09")];
        let row = project(&accounts, &[], date(2026, 10, 7), &["vault"]);
        assert_eq!(row.current_wiggle_room, "300.00".parse().unwrap());
        assert_eq!(row.savings_balance, "522.09".parse().unwrap());
        assert!(
            !row.banks
                .iter()
                .find(|b| b.account_id == "vault")
                .unwrap()
                .included
        );
    }

    #[test]
    fn received_salary_funds_next_month_and_budgets_reserve_envelopes() {
        let accounts = [account("bank", "4000")];
        let status = PaydayStatus {
            due_on: "2026-09-25".into(),
            next_expected_on: "2026-10-25".into(),
            received_on: Some("2026-09-25".into()),
            received_amounts: vec![eur("3000")],
            transaction_count: 1,
        };
        let row = |planned: i64, reserved: i64| BudgetRow {
            id: "groceries".into(),
            name: "Groceries".into(),
            planned: d(planned),
            spent: d(planned - reserved),
            remaining: d(reserved),
            reserved: d(reserved),
            scheduled: d(0),
            categories: Vec::new(),
            history: Vec::new(),
            daily_spending: Vec::new(),
            elapsed_days: 0,
            expected: Vec::new(),
            to_events: Vec::new(),
            week: None,
        };
        let budget = |month: &str, reserved: i64| MonthlyBudget {
            plan: crate::budget::default_budget(month.parse().unwrap(), &Currency::eur()),
            fixed: row(0, 0),
            groups: vec![row(300, reserved)],
            unplanned: row(0, 0),
            extras: row(0, 0),
            purchases: Vec::new(),
            history_months: Vec::new(),
            overruns: Vec::new(),
            recurring_extras: None,
            reserved: d(reserved),
            planned: d(300),
            spent: d(300 - reserved),
            events: Vec::new(),
        };
        let budgets = [budget("2026-09-01", 100), budget("2026-10-01", 300)];
        let mut current = empty();
        current.remaining_costs = vec![eur("50")];
        let mut next = empty();
        next.expected_costs = vec![eur("1000")];
        let none = HashSet::new();
        let allocation = monthly_allocation(&AllocationInputs {
            through_date: date(2026, 9, 28),
            accounts: &accounts,
            goals: &[],
            goal_evaluations: &[],
            payday_status: Some(&status),
            current_subscriptions: &current,
            next_subscriptions: &next,
            planned_payments: &[],
            spending_limits: &[],
            spending_nudges: &[],
            budgets: Some(&budgets),
            savings_account_ids: &none,
            current_month: Some(date(2026, 9, 1)),
            savings_goal_ids: &none,
            period_start: Some(date(2026, 9, 1)),
        });
        let eur_row = &allocation.currencies[0];
        assert_eq!(eur_row.salary_reserved, d(3000));
        assert_eq!(eur_row.salary_received_on.as_deref(), Some("2026-09-25"));
        assert_eq!(eur_row.current_month_funds, d(1000));
        assert_eq!(eur_row.current_groups_remaining, d(100));
        assert_eq!(eur_row.current_wiggle_room, d(850));
        assert_eq!(eur_row.next_groups, d(300));
        assert_eq!(eur_row.next_wiggle_room, d(1700));
        assert_eq!(eur_row.groups[0].remaining, d(100));
        assert_eq!(eur_row.next_group_rows[0].remaining, d(300));
    }
}
