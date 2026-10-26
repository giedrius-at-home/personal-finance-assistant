//! Monthly budget plans against actual spending (Python domain/monthly_budget.py).
//!
//! Envelopes sit on top of the unchanged categories: a purchase belongs to the
//! group that names it, else its category, else its reviewed spending group.

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::gateway::{BudgetGroup, BudgetPlan, Direction, GroupMapping};
use finance_types::report::Currency;
use finance_types::report::budget::{BudgetCategory, BudgetPurchase, BudgetRow, MonthlyBudget};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use finance_types::report::subscriptions::SubscriptionReport;
use jiff::Zoned;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::Ctx;
use crate::groups::{
    fixed_category_names, fixed_transaction_ids, group_index, without_settlements,
};
use crate::model::{Kind, Tx};
use crate::period::{PeriodResolver, shift_month};
use crate::personal::apply_personal_spending;
use crate::plans::net_spending_transactions;

/// The months a budget's history covers: five before it and the month itself.
const HISTORY_MONTHS: i32 = 6;

/// The spending every budget reads: bulk-payment settlements removed, linked
/// reimbursements netted, then owner-marked shared spending applied (the
/// dashboard's `spending_transactions`).
pub fn spending_transactions(ctx: &Ctx, payments: &[PlannedPaymentView]) -> Vec<Tx> {
    let settled = without_settlements(&ctx.snap.transactions, &ctx.snap.details);
    let netted = net_spending_transactions(&settled, payments);
    apply_personal_spending(netted, &ctx.snap.details)
}

/// Transactions the budget counts as fixed spending (Python `budget_fixed_ids`).
pub fn budget_fixed_ids(ctx: &Ctx, spending: &[Tx]) -> HashSet<String> {
    let snap = ctx.snap;
    let categories =
        fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    fixed_transaction_ids(spending, &categories)
}

fn group(id: &str, name: &str, amount: i64, source_groups: &[&str]) -> BudgetGroup {
    BudgetGroup {
        id: id.to_owned(),
        name: name.to_owned(),
        amount: Decimal::from(amount),
        source_groups: source_groups.iter().map(|g| (*g).to_owned()).collect(),
        categories: Vec::new(),
        transaction_ids: Vec::new(),
        account_id: None,
    }
}

/// The plan of a month the owner never planned: groceries and food out in
/// euros, nothing in other currencies.
pub fn default_budget(month: Date, currency: &Currency) -> BudgetPlan {
    let groups = if currency.code == "EUR" {
        vec![
            group("groceries", "Groceries", 300, &["Groceries"]),
            group(
                "food-out",
                "Food & drinks out",
                100,
                &[
                    "Coffee",
                    "Drinks",
                    "Eating out",
                    "Takeaway",
                    "Delivery",
                    "Snacks",
                ],
            ),
        ]
    } else {
        Vec::new()
    };
    BudgetPlan {
        month: month.to_string(),
        currency: currency.clone(),
        revision: 0,
        groups,
    }
}

/// The plan group a purchase counts in: an explicit purchase overrides its
/// category, which overrides a broad spending group.
pub fn budget_owner<'p>(
    groups: &'p [BudgetGroup],
    category: Option<&str>,
    source_group: Option<&str>,
    transaction_id: Option<&str>,
) -> Option<&'p str> {
    let find = |pick: &dyn Fn(&BudgetGroup) -> bool| {
        groups.iter().find(|g| pick(g)).map(|g| g.id.as_str())
    };
    transaction_id
        .and_then(|id| find(&|g| g.transaction_ids.iter().any(|t| t == id)))
        .or_else(|| category.and_then(|c| find(&|g| g.categories.iter().any(|x| x == c))))
        .or_else(|| source_group.and_then(|s| find(&|g| g.source_groups.iter().any(|x| x == s))))
}

/// What every month's budget of one build shares.
#[derive(Clone, Copy, Debug)]
pub struct BudgetInputs<'a> {
    pub transactions: &'a [Tx],
    pub mappings: &'a [GroupMapping],
    /// The build moment in the owner's time zone; later bookings are ignored.
    pub now: &'a Zoned,
    pub fixed_ids: &'a HashSet<String>,
    pub payments: &'a [PlannedPaymentView],
    /// Closure-aware months; calendar months of the effective date without it.
    pub resolver: Option<&'a PeriodResolver>,
    /// The running budget month; today's calendar month without it.
    pub current_month: Option<Date>,
}

fn signed(t: &Tx) -> Decimal {
    if t.kind == Kind::Refund {
        -t.amount
    } else {
        t.amount
    }
}

fn days_between(from: Date, to: Date) -> i64 {
    i64::from((to - from).get_days())
}

fn parse_date(text: &str) -> Option<Date> {
    text.get(..10)?.parse().ok()
}

/// Budget one month. `period_start` is the local day a closed month's cutoff
/// hands over to the plan's month (its first day by default).
pub fn monthly_budget(
    plan: &BudgetPlan,
    subscriptions: &SubscriptionReport,
    period_start: Option<Date>,
    inputs: &BudgetInputs,
) -> MonthlyBudget {
    let now = inputs.now;
    let today = now.date();
    let month = parse_date(&plan.month).unwrap_or_else(|| today.first_of_month());
    let current_month = inputs
        .current_month
        .unwrap_or_else(|| today.first_of_month());
    let start = period_start.unwrap_or(month);
    let month_end = month.last_of_month();
    let period_days = (days_between(start, month_end) + 1).max(0);
    let elapsed_days =
        usize::try_from(period_days.min((days_between(start, today) + 1).max(0))).unwrap_or(0);
    let mapping = group_index(inputs.mappings);
    let source_group = |category: Option<&str>| -> Option<&str> {
        mapping.get(category.unwrap_or("")).copied().flatten()
    };
    let history_months: Vec<Date> = (1 - HISTORY_MONTHS..=0)
        .map(|offset| shift_month(month, offset))
        .collect();
    let mut history: HashMap<String, Vec<Decimal>> = HashMap::new();
    let mut rows: HashMap<String, Vec<&Tx>> = HashMap::new();
    let mut purchases = Vec::new();
    let reimbursement_origins: HashMap<&str, &str> = inputs
        .payments
        .iter()
        .filter(|p| p.status == PlanStatus::Paid)
        .filter_map(|p| {
            Some((
                p.transaction_id.as_deref()?,
                p.reimbursement_for.as_deref()?,
            ))
        })
        .filter(|(_, origin)| !origin.is_empty())
        .collect();
    let paid_ids: HashSet<Option<&str>> = inputs
        .payments
        .iter()
        .filter(|p| p.direction == Direction::Outflow && p.status == PlanStatus::Paid)
        .map(|p| p.transaction_id.as_deref())
        .collect();
    for t in inputs.transactions {
        if !matches!(t.kind, Kind::Outflow | Kind::Refund)
            || t.currency != plan.currency
            || t.occurred_at.timestamp() > now.timestamp()
        {
            continue;
        }
        let on = t.effective_on();
        let tx_month = match inputs.resolver {
            Some(resolver) => resolver.month_of(t),
            None => on.first_of_month(),
        };
        let Some(index) = history_months.iter().position(|m| *m == tx_month) else {
            continue;
        };
        let owner = if inputs.fixed_ids.contains(&t.transaction_id) {
            Some("fixed")
        } else {
            let id = reimbursement_origins
                .get(t.transaction_id.as_str())
                .copied()
                .unwrap_or(&t.transaction_id);
            budget_owner(
                &plan.groups,
                t.category.as_deref(),
                source_group(t.category.as_deref()),
                Some(id),
            )
        };
        let key = owner.unwrap_or(if paid_ids.contains(&Some(t.transaction_id.as_str())) {
            "extras"
        } else {
            "unplanned"
        });
        let amount = signed(t);
        history
            .entry(key.to_owned())
            .or_insert_with(|| vec![Decimal::ZERO; history_months.len()])[index] += amount;
        if tx_month != month {
            continue;
        }
        rows.entry(key.to_owned()).or_default().push(t);
        if key != "fixed" {
            let description = [t.description.as_deref(), t.category.as_deref()]
                .into_iter()
                .flatten()
                .find(|s| !s.is_empty())
                .unwrap_or("Purchase");
            purchases.push(BudgetPurchase {
                id: t.transaction_id.clone(),
                description: description.to_owned(),
                category: t.category.clone(),
                amount,
                occurred_on: on.to_string(),
                budget_group_id: (key != "unplanned").then(|| key.to_owned()),
            });
        }
    }

    let mut extra_pending = Decimal::ZERO;
    let mut pending: HashMap<&str, Decimal> = HashMap::new();
    for p in inputs.payments {
        let Some(due_on) = parse_date(&p.due_on) else {
            continue;
        };
        if p.direction != Direction::Outflow
            || p.status == PlanStatus::Paid
            || p.amount.currency != plan.currency
            || !(due_on.first_of_month() == month
                || (p.pending && due_on < month && month == current_month))
        {
            continue;
        }
        let category = p.match_category.as_str();
        match budget_owner(
            &plan.groups,
            Some(category),
            source_group(Some(category)),
            None,
        ) {
            Some(owner) => *pending.entry(owner).or_default() += p.amount.amount,
            None => extra_pending += p.amount.amount,
        }
    }

    let last = |key: &str| {
        history
            .get(key)
            .and_then(|h| h.last().copied())
            .unwrap_or_default()
    };
    let row = |key: &str, name: &str, planned: Decimal, extra_pending: Decimal| {
        let items = rows.get(key).map(Vec::as_slice).unwrap_or_default();
        // Python groups in first-seen order, then sorts stably by name.
        let mut order: Vec<Option<&str>> = Vec::new();
        let mut by_category: HashMap<Option<&str>, Vec<&Tx>> = HashMap::new();
        for t in items {
            let category = t.category.as_deref();
            by_category
                .entry(category)
                .or_insert_with(|| {
                    order.push(category);
                    Vec::new()
                })
                .push(t);
        }
        order.sort_by_key(|c| c.unwrap_or(""));
        let categories: Vec<BudgetCategory> = order
            .into_iter()
            .map(|category| {
                let txs = &by_category[&category];
                BudgetCategory {
                    category: category.map(str::to_owned),
                    source_group: source_group(category).map(str::to_owned),
                    spent: txs.iter().map(|t| signed(t)).sum(),
                    count: txs.len(),
                }
            })
            .collect();
        let spent: Decimal = categories.iter().map(|c| c.spent).sum();
        let mut daily = vec![Decimal::ZERO; elapsed_days];
        for t in items {
            // Assigned to this month but dated before its start (a late
            // arrival after the previous cutoff): count it on the first day.
            let index = usize::try_from(days_between(start, t.effective_on()).max(0)).unwrap_or(0);
            if let Some(day) = daily.get_mut(index) {
                *day += signed(t);
            }
        }
        let pending_here = pending.get(key).copied().unwrap_or_default();
        BudgetRow {
            id: key.to_owned(),
            name: name.to_owned(),
            planned,
            spent,
            remaining: planned - spent,
            reserved: (planned - spent - pending_here).max(Decimal::ZERO),
            scheduled: if key == "extras" {
                extra_pending
            } else {
                pending_here.min((planned - spent).max(Decimal::ZERO))
            },
            categories,
            history: history
                .get(key)
                .cloned()
                .unwrap_or_else(|| vec![Decimal::ZERO; history_months.len()]),
            daily_spending: daily,
            elapsed_days,
        }
    };

    let fixed_remaining: Decimal = subscriptions
        .remaining_costs
        .iter()
        .filter(|m| m.currency == plan.currency)
        .map(|m| m.amount)
        .sum();
    let fixed = row(
        "fixed",
        "Fixed expenses",
        last("fixed") + fixed_remaining,
        extra_pending,
    );
    let groups: Vec<BudgetRow> = plan
        .groups
        .iter()
        .map(|g| row(&g.id, &g.name, g.amount, extra_pending))
        .collect();
    extra_pending += groups
        .iter()
        .map(|g| {
            let pending_here = pending.get(g.id.as_str()).copied().unwrap_or_default();
            (pending_here - g.remaining.max(Decimal::ZERO)).max(Decimal::ZERO)
        })
        .sum::<Decimal>();
    let extras = row(
        "extras",
        "Extra payments",
        last("extras") + extra_pending,
        extra_pending,
    );
    let unplanned = row(
        "unplanned",
        "Unplanned spending",
        Decimal::ZERO,
        extra_pending,
    );
    purchases.sort_by(|a, b| (&b.occurred_on, &b.id).cmp(&(&a.occurred_on, &a.id)));
    let groups_planned: Decimal = groups.iter().map(|g| g.planned).sum();
    let groups_spent: Decimal = groups.iter().map(|g| g.spent).sum();
    MonthlyBudget {
        plan: plan.clone(),
        reserved: groups.iter().map(|g| g.reserved).sum(),
        planned: fixed.planned + extras.planned + groups_planned,
        spent: fixed.spent + extras.spent + (unplanned.spent + groups_spent),
        fixed,
        groups,
        unplanned,
        extras,
        purchases,
        history_months: history_months.iter().map(Date::to_string).collect(),
    }
}

/// The current and next budget month for every currency the owner holds or
/// spends, from the saved plan or the default one (the dashboard's
/// `monthly_budgets`). The subscription reports are the calendar months that
/// fund each budget month (`budget_calendar_offset` and the one after).
pub fn monthly_budgets(
    ctx: &Ctx,
    spending: &[Tx],
    payments: &[PlannedPaymentView],
    current_subscriptions: &SubscriptionReport,
    next_subscriptions: &SubscriptionReport,
) -> Vec<MonthlyBudget> {
    let snap = ctx.snap;
    let saved: HashMap<(Date, &Currency), &BudgetPlan> = snap
        .budget_plans
        .iter()
        .filter_map(|p| Some(((parse_date(&p.month)?, &p.currency), p)))
        .collect();
    let mut currencies: BTreeMap<&str, &Currency> = BTreeMap::new();
    for currency in snap
        .accounts
        .iter()
        .map(|a| a.currency())
        .chain(spending.iter().map(|t| &t.currency))
    {
        currencies.entry(currency.code.as_str()).or_insert(currency);
    }
    let fixed_ids = budget_fixed_ids(ctx, spending);
    let current_month = ctx.current_month();
    let inputs = BudgetInputs {
        transactions: spending,
        mappings: &snap.group_mappings,
        now: &ctx.now,
        fixed_ids: &fixed_ids,
        payments,
        resolver: Some(&ctx.resolver),
        current_month: Some(current_month),
    };
    [
        (current_month, current_subscriptions),
        (shift_month(current_month, 1), next_subscriptions),
    ]
    .into_iter()
    .flat_map(|(month, subscriptions)| {
        let (inputs, saved) = (&inputs, &saved);
        currencies.values().map(move |currency| {
            let plan = saved
                .get(&(month, *currency))
                .map_or_else(|| default_budget(month, currency), |p| (*p).clone());
            monthly_budget(
                &plan,
                subscriptions,
                Some(ctx.resolver.period_start_date(month)),
                inputs,
            )
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::UnitMoney;
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    fn now() -> Zoned {
        "2026-09-10T12:00:00+00:00[UTC]".parse().unwrap()
    }

    fn month() -> Date {
        date(2026, 9, 1)
    }

    fn tx(id: &str, amount: &str) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 1,
            account_id: "daily".into(),
            counterparty_account_id: None,
            kind: Kind::Outflow,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: now(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: Some("Jumbo".into()),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn with_category(t: Tx, category: Option<&str>) -> Tx {
        Tx {
            category: category.map(Into::into),
            ..t
        }
    }

    fn payment(id: &str, category: &str, amount: &str, status: PlanStatus) -> PlannedPaymentView {
        PlannedPaymentView {
            plan_id: id.into(),
            name: id.into(),
            match_category: category.into(),
            direction: Direction::Outflow,
            amount: UnitMoney {
                amount: amount.parse().unwrap(),
                currency: Currency::eur(),
            },
            due_on: "2026-09-01".into(),
            created_at: "2026-09-10T12:00:00Z".into(),
            reimbursement_for: None,
            pending: false,
            account_id: None,
            match_description: None,
            status,
            paid_on: None,
            transaction_id: None,
            candidate_count: 0,
            candidates: Vec::new(),
        }
    }

    fn paid(p: PlannedPaymentView, tx_id: &str) -> PlannedPaymentView {
        PlannedPaymentView {
            status: PlanStatus::Paid,
            transaction_id: Some(tx_id.into()),
            paid_on: Some("2026-09-10".into()),
            ..p
        }
    }

    fn empty_subscriptions() -> SubscriptionReport {
        SubscriptionReport {
            fetched_at: None,
            through_date: "2026-09-10".into(),
            stale: false,
            rows: Vec::new(),
            expected_costs: Vec::new(),
            remaining_costs: Vec::new(),
            scheduled_payments: Vec::new(),
            unmatched_count: 0,
            evidence_start: "2026-09-01".into(),
            category_matched_names: Vec::new(),
        }
    }

    fn mappings() -> Vec<GroupMapping> {
        [
            ("Jumbo", "Groceries"),
            ("Cafe", "Coffee"),
            ("Restaurant", "Eating out"),
        ]
        .into_iter()
        .map(|(category, group)| GroupMapping {
            category: category.into(),
            group: Some(group.into()),
            revision: 1,
        })
        .collect()
    }

    fn report(
        plan: Option<BudgetPlan>,
        transactions: &[Tx],
        payments: &[PlannedPaymentView],
        fixed: &[&str],
    ) -> MonthlyBudget {
        let fixed_ids: HashSet<String> = fixed.iter().map(|s| (*s).to_owned()).collect();
        let mappings = mappings();
        let now = now();
        let plan = plan.unwrap_or_else(|| default_budget(month(), &Currency::eur()));
        monthly_budget(
            &plan,
            &empty_subscriptions(),
            None,
            &BudgetInputs {
                transactions,
                mappings: &mappings,
                now: &now,
                fixed_ids: &fixed_ids,
                payments,
                resolver: None,
                current_month: None,
            },
        )
    }

    fn extra_group(id: &str, amount: i64) -> BudgetGroup {
        BudgetGroup {
            id: id.into(),
            name: id.into(),
            amount: Decimal::from(amount),
            source_groups: Vec::new(),
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
        }
    }

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    #[test]
    fn combines_allowances_refunds_fixed_unplanned_and_purchase_overrides() {
        let mut plan = default_budget(month(), &Currency::eur());
        plan.groups.push(BudgetGroup {
            transaction_ids: vec!["dinner".into()],
            ..extra_group("event", 30)
        });
        let transactions = [
            tx("groceries", "50"),
            with_category(tx("coffee", "10"), Some("Cafe")),
            Tx {
                kind: Kind::Refund,
                ..with_category(tx("refund", "4"), Some("Cafe"))
            },
            with_category(tx("dinner", "25"), Some("Restaurant")),
            tx("bill", "15"),
            with_category(tx("unknown", "7"), None),
            Tx {
                kind: Kind::Transfer,
                ..tx("transfer", "100")
            },
            Tx {
                currency: Currency::new("USD"),
                ..tx("usd", "100")
            },
        ];
        let view = report(Some(plan), &transactions, &[], &["bill"]);
        let spent: Vec<Decimal> = view.groups.iter().map(|g| g.spent).collect();
        assert_eq!(spent, [d(50), d(6), d(25)]);
        assert_eq!(view.fixed.spent, d(15));
        assert_eq!(view.unplanned.spent, d(7));
        assert_eq!(view.spent, d(103));
        let food = &view.groups[1];
        assert_eq!(food.elapsed_days, 10);
        assert_eq!(food.daily_spending.iter().sum::<Decimal>(), food.spent);
        assert_eq!(food.daily_spending.last(), Some(&d(6)));
        assert_eq!(food.remaining, d(94));
        assert_eq!(food.categories[0].count, 2);
        assert_eq!(view.reserved, d(349));
        assert_eq!(view.purchases.len(), 5);
    }

    #[test]
    fn explicit_category_overrides_broad_group_and_shows_overspending() {
        let mut plan = default_budget(month(), &Currency::eur());
        plan.groups.push(BudgetGroup {
            categories: vec!["Jumbo".into()],
            ..extra_group("special", 10)
        });
        let view = report(Some(plan), &[tx("one", "15")], &[], &[]);
        assert_eq!(view.groups[0].spent, d(0));
        assert_eq!(view.groups[2].spent, d(15));
        assert_eq!(view.groups[2].remaining, d(-5));
        assert_eq!(view.groups[2].reserved, d(0));
        assert_eq!(view.spent, d(15));
    }

    #[test]
    fn pending_purchase_does_not_reserve_its_allowance_twice() {
        let order = PlannedPaymentView {
            pending: true,
            ..payment("order", "Jumbo", "50", PlanStatus::Pending)
        };
        let view = report(None, &[], std::slice::from_ref(&order), &[]);
        assert_eq!(view.groups[0].remaining, d(300));
        assert_eq!(view.groups[0].reserved, d(250));
        assert_eq!(view.reserved + order.amount.amount, d(400));
        let view = report(None, &[tx("order", "50")], &[paid(order, "order")], &[]);
        assert_eq!(view.reserved, d(350));
    }

    #[test]
    fn one_off_payment_moves_from_reserved_to_paid_without_double_counting() {
        let repair = payment("repair", "Repair", "80", PlanStatus::Upcoming);
        let pending = report(None, &[], std::slice::from_ref(&repair), &[]);
        assert_eq!(pending.extras.planned, d(80));
        assert_eq!(pending.extras.remaining, d(80));
        assert_eq!(pending.extras.scheduled, d(80));
        assert_eq!(pending.planned, d(480));
        let view = report(
            None,
            &[with_category(tx("repair", "80"), Some("Repair"))],
            &[paid(repair.clone(), "repair")],
            &[],
        );
        assert_eq!(view.extras.spent, d(80));
        assert_eq!(view.extras.remaining, d(0));
        assert_eq!(view.unplanned.spent, d(0));
        assert_eq!(view.spent, d(80));
        assert_eq!(view.purchases[0].budget_group_id.as_deref(), Some("extras"));
        let october = default_budget(date(2026, 10, 1), &Currency::eur());
        let next = report(Some(october), &[], &[repair], &[]);
        assert_eq!(next.extras.planned, d(0));
    }

    #[test]
    fn pending_payment_above_allowance_adds_only_the_excess_to_the_plan() {
        let order = payment("large-order", "Jumbo", "350", PlanStatus::Upcoming);
        let view = report(None, &[], &[order], &[]);
        assert_eq!(view.groups[0].reserved, d(0));
        assert_eq!(view.groups[0].scheduled, d(300));
        assert_eq!(view.extras.scheduled, d(50));
        assert_eq!(view.extras.planned, d(50));
        assert_eq!(view.planned, d(450));
        assert_eq!(view.reserved, d(100));
    }

    #[test]
    fn purchase_after_early_cutoff_lands_in_the_next_month() {
        let periods = vec![serde_json::json!({
            "month": "2026-09-01",
            "cutoff_at": "2026-09-30T09:00:00Z",
        })];
        let bought: Zoned = "2026-09-30T11:00:00+00:00[UTC]".parse().unwrap();
        let now: Zoned = "2026-09-30T12:00:00+00:00[UTC]".parse().unwrap();
        let resolver = PeriodResolver::new(
            &periods,
            HashMap::from([("late".to_owned(), bought.timestamp())]),
            TimeZone::UTC,
        );
        let october = resolver.current(&now);
        assert_eq!(october, date(2026, 10, 1));
        let purchase = Tx {
            occurred_at: bought,
            ..tx("late", "5")
        };
        let mappings = mappings();
        let fixed_ids = HashSet::new();
        let plan = default_budget(october, &Currency::eur());
        for start in [None, Some(resolver.period_start_date(october))] {
            let budget = monthly_budget(
                &plan,
                &empty_subscriptions(),
                start,
                &BudgetInputs {
                    transactions: std::slice::from_ref(&purchase),
                    mappings: &mappings,
                    now: &now,
                    fixed_ids: &fixed_ids,
                    payments: &[],
                    resolver: Some(&resolver),
                    current_month: Some(october),
                },
            );
            assert_eq!(budget.groups[0].spent, d(5));
            if start.is_some() {
                assert_eq!(budget.groups[0].elapsed_days, 1);
                assert_eq!(budget.groups[0].daily_spending, [d(5)]);
            }
        }
    }

    #[test]
    fn owner_prefers_purchase_then_category_then_group() {
        let mut plan = default_budget(month(), &Currency::eur());
        plan.groups.push(BudgetGroup {
            categories: vec!["Jumbo".into()],
            transaction_ids: vec!["t1".into()],
            ..extra_group("special", 10)
        });
        let owner = |c, g, t| budget_owner(&plan.groups, c, g, t);
        assert_eq!(
            owner(Some("Lidl"), Some("Groceries"), Some("t1")),
            Some("special")
        );
        assert_eq!(
            owner(Some("Jumbo"), Some("Groceries"), None),
            Some("special")
        );
        assert_eq!(
            owner(Some("Lidl"), Some("Groceries"), None),
            Some("groceries")
        );
        assert_eq!(owner(Some("Lidl"), None, None), None);
        assert!(
            default_budget(month(), &Currency::new("USD"))
                .groups
                .is_empty()
        );
    }
}
