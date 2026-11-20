//! Planned one-off payments and their matching transactions (Python planned_payments.py, budget_inputs.py, recurring_income.py).

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{Direction, OwnerSettings, PlannedPayment};
use finance_types::report::plans::{PlanMatchCandidate, PlanStatus, PlannedPaymentView};
use finance_types::report::{Currency, UnitMoney};
use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use rust_decimal::Decimal;

use crate::Ctx;
use crate::load::parse_instant;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::text::casefold;

/// Planned purchases a month closure created from its plan lines.
pub const CLOSURE_PLAN_PREFIX: &str = "closure-";
/// A planned purchase may cost a little more than the money set aside for it…
pub const CLOSURE_PLAN_TOLERANCE: Decimal = Decimal::from_parts(110, 0, 0, false, 2);
/// …but a much smaller purchase in the same category is not that purchase.
pub const CLOSURE_PLAN_MINIMUM: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// Plans written when a wish is marked bought (`wish-<wish id>`): matched on
/// account, amount and date only, whatever category the payment has.
pub const WISH_PLAN_PREFIX: &str = "wish-";
/// Monthly salary plans generated from owner settings rather than entered by hand.
pub const RECURRING_PLAN_PREFIX: &str = "recurring-";
/// An owner-entered plan may be paid early, but never long before it is due.
pub const EARLY_PAYMENT_LIMIT_DAYS: i64 = 21;

/// Saved planned payments plus recurring salary, matched to the build's
/// transactions (Python `planned_payment_views_for`).
pub fn planned_payment_views(ctx: &Ctx) -> Vec<PlannedPaymentView> {
    planned_payment_views_since(ctx, None)
}

/// [`planned_payment_views`] with salary plans from `salary_since` on, for a
/// closure reviewed after its month ended.
pub fn planned_payment_views_since(
    ctx: &Ctx,
    salary_since: Option<Date>,
) -> Vec<PlannedPaymentView> {
    let snap = ctx.snap;
    let raw: HashMap<&str, &Tx> = snap
        .raw_transactions
        .iter()
        .map(|t| (t.transaction_id.as_str(), t))
        .collect();
    // Owner-marked shared spending changes the category; the plan may still
    // name the category the bank row had.
    let original_categories: HashMap<String, Option<String>> = snap
        .transactions
        .iter()
        .filter(|t| {
            snap.details
                .get(&t.source_key())
                .is_some_and(|d| !d.values.personal_expense_role.is_empty())
        })
        .filter_map(|t| {
            raw.get(t.transaction_id.as_str())
                .map(|o| (t.transaction_id.clone(), o.category.clone()))
        })
        .collect();
    reconcile_planned_payments(
        &with_recurring_salary(
            &snap.planned_payments,
            &snap.settings,
            &ctx.now,
            salary_since,
        ),
        &snap.archived_payments,
        &snap.transactions,
        &ctx.now,
        &original_categories,
    )
}

/// Salary plans from this month on (or from `since` when earlier), unless
/// the owner entered that month's salary by hand. Never invents transactions.
pub fn with_recurring_salary(
    plans: &[PlannedPayment],
    settings: &OwnerSettings,
    now: &Zoned,
    since: Option<Date>,
) -> Vec<PlannedPayment> {
    let mut out = plans.to_vec();
    let (Some(salary), Some(payday)) = (settings.monthly_salary_eur, settings.payday_day) else {
        return out;
    };
    let zone = TimeZone::get(&settings.timezone).unwrap_or_else(|_| now.time_zone().clone());
    let current = now.with_time_zone(zone).date().first_of_month();
    let month = since.map_or(current, |s| s.first_of_month().min(current));
    let months = (i32::from(current.year()) - i32::from(month.year())) * 12
        + i32::from(current.month())
        - i32::from(month.month())
        + 4;
    let existing: HashSet<Date> = plans
        .iter()
        .filter(|p| {
            p.direction == Direction::Inflow
                && casefold(p.match_category.trim()) == "salary"
                && p.amount.currency.code == "EUR"
        })
        .filter_map(|p| due_on(p).map(Date::first_of_month))
        .collect();
    let created_at = now
        .timestamp()
        .display_with_offset(now.offset())
        .to_string();
    for offset in 0..months {
        let start = shift_month(month, offset);
        if existing.contains(&start) {
            continue;
        }
        let day = i8::try_from(payday)
            .unwrap_or(i8::MAX)
            .min(start.days_in_month());
        out.push(PlannedPayment {
            plan_id: format!("{RECURRING_PLAN_PREFIX}salary-{start}"),
            name: "Monthly salary".into(),
            match_category: "Salary".into(),
            direction: Direction::Inflow,
            amount: UnitMoney {
                amount: salary,
                currency: Currency::eur(),
            },
            due_on: start.with().day(day).build().unwrap_or(start).to_string(),
            created_at: created_at.clone(),
            reimbursement_for: None,
            pending: false,
            account_id: settings.salary_account_id.clone(),
            match_description: None,
        });
    }
    out
}

fn due_on(plan: &PlannedPayment) -> Option<Date> {
    plan.due_on.get(..10)?.parse().ok()
}

fn is_purchase(plan: &PlannedPayment) -> bool {
    plan.plan_id.starts_with(CLOSURE_PLAN_PREFIX)
}

/// Letters and digits, casefolded, joined by single spaces (Python `[^\W_]+`).
fn tokens(text: &str) -> String {
    casefold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

struct Reconciler<'a> {
    zone: TimeZone,
    original_categories: &'a HashMap<String, Option<String>>,
    evidence: Vec<&'a Tx>,
    /// Payments a bill already claims: never offered to a plan as a
    /// match, but one that fits shows the plan was paid through the bill.
    billed: Vec<&'a Tx>,
}

impl<'a> Reconciler<'a> {
    fn matches_identity(&self, plan: &PlannedPayment, t: &Tx) -> bool {
        // A planned purchase may be paid from any account.
        if let Some(account) = &plan.account_id
            && !is_purchase(plan)
            && &t.account_id != account
        {
            return false;
        }
        if plan.plan_id.starts_with(WISH_PLAN_PREFIX) {
            // The purchase itself was recorded; any category may hold it.
            return true;
        }
        if let Some(wanted) = &plan.match_description {
            // Merchant evidence takes precedence over broad accounting categories.
            let wanted = tokens(wanted);
            let description = tokens(t.description.as_deref().unwrap_or(""));
            return !wanted.is_empty()
                && format!(" {description} ").contains(&format!(" {wanted} "));
        }
        let target = casefold(plan.match_category.trim());
        let original = self
            .original_categories
            .get(&t.transaction_id)
            .and_then(Option::as_deref);
        [t.category.as_deref(), original]
            .into_iter()
            .flatten()
            .any(|c| casefold(c.trim()) == target)
    }

    fn matches_amount(plan: &PlannedPayment, t: &Tx) -> bool {
        let same_currency = t.currency == plan.amount.currency;
        if same_currency && t.amount == plan.amount.amount {
            return true;
        }
        if is_purchase(plan) {
            // What a purchase costs may differ a little from what was set aside.
            return same_currency
                && plan.amount.amount * CLOSURE_PLAN_MINIMUM <= t.amount
                && t.amount <= plan.amount.amount * CLOSURE_PLAN_TOLERANCE;
        }
        // Salary varies month to month; the category is the evidence.
        plan.direction == Direction::Inflow
            && casefold(plan.match_category.trim()) == "salary"
            && same_currency
    }

    /// (early start, normal start, end); the early stretch may be empty.
    fn window(&self, plan: &PlannedPayment, due: Date) -> (Date, Date, Date) {
        if is_purchase(plan) {
            // A planned purchase is due some time in its month.
            let start = due.first_of_month();
            let end = shift_month(start, 1)
                .checked_add(jiff::Span::new().days(6))
                .unwrap_or(due);
            return (start, start, end);
        }
        let days = |n: i64| jiff::Span::new().days(n);
        let start = due.checked_sub(days(7)).unwrap_or(due);
        let mut early = start;
        if !plan.plan_id.starts_with(RECURRING_PLAN_PREFIX) {
            // An owner-entered plan counts payments made early, once it existed.
            let created = parse_instant(&plan.created_at)
                .map(|c| c.to_zoned(self.zone.clone()).date())
                .unwrap_or(start);
            early = due
                .checked_sub(days(EARLY_PAYMENT_LIMIT_DAYS))
                .unwrap_or(due)
                .max(start.min(created));
        }
        let end = due
            .checked_add(days(if plan.pending { 14 } else { 7 }))
            .unwrap_or(due);
        (early, start, end)
    }

    fn matching(&self, plan: &PlannedPayment, start: Date, end: Date) -> Vec<&'a Tx> {
        self.matching_in(&self.evidence, plan, start, end)
    }

    fn matching_in(
        &self,
        pool: &[&'a Tx],
        plan: &PlannedPayment,
        start: Date,
        end: Date,
    ) -> Vec<&'a Tx> {
        pool.iter()
            .copied()
            .filter(|t| {
                let direction_ok = match plan.direction {
                    Direction::Outflow => t.kind == Kind::Outflow,
                    Direction::Inflow => matches!(t.kind, Kind::Inflow | Kind::Refund),
                };
                let on = t.effective_on();
                self.matches_identity(plan, t)
                    && direction_ok
                    && Self::matches_amount(plan, t)
                    && start <= on
                    && on <= end
            })
            .collect()
    }
}

/// Match plans to transactions. `archived` plans are never shown, but a
/// payment in their due window stays theirs: it is not offered to another
/// plan as an early payment.
pub fn reconcile_planned_payments(
    plans: &[PlannedPayment],
    archived: &[PlannedPayment],
    transactions: &[Tx],
    now: &Zoned,
    original_categories: &HashMap<String, Option<String>>,
) -> Vec<PlannedPaymentView> {
    let zone = now.time_zone().clone();
    let today = now.date();
    let (mut billed, mut evidence): (Vec<&Tx>, Vec<&Tx>) = transactions
        .iter()
        .filter(|t| {
            matches!(t.kind, Kind::Outflow | Kind::Inflow | Kind::Refund)
                && t.occurred_at.timestamp() <= now.timestamp()
        })
        .partition(|t| t.subscription_id.is_some());
    billed.sort_by(|a, b| a.transaction_id.cmp(&b.transaction_id));
    // The Python projection listed transactions by canonical id as text.
    evidence.sort_by(|a, b| a.transaction_id.cmp(&b.transaction_id));
    let r = Reconciler {
        zone,
        original_categories,
        evidence,
        billed,
    };
    let dated: Vec<(&PlannedPayment, Date)> = plans
        .iter()
        .filter_map(|p| due_on(p).map(|d| (p, d)))
        .collect();
    let windows: HashMap<&str, (Date, Date, Date)> = archived
        .iter()
        .filter_map(|p| due_on(p).map(|d| (p, d)))
        .chain(dated.iter().copied())
        .map(|(p, d)| (p.plan_id.as_str(), r.window(p, d)))
        .collect();
    // Early matching is a fallback: a payment in any plan's normal window
    // (archived plans included) belongs to that plan, not to a later one.
    let claimed: HashSet<&str> = archived
        .iter()
        .chain(dated.iter().map(|(p, _)| *p))
        .filter_map(|p| {
            windows
                .get(p.plan_id.as_str())
                .map(|(_, start, end)| r.matching(p, *start, *end))
        })
        .flatten()
        .map(|t| t.transaction_id.as_str())
        .collect();
    let mut candidates: HashMap<&str, Vec<&Tx>> = HashMap::new();
    for (plan, _) in &dated {
        let (early, start, end) = windows[plan.plan_id.as_str()];
        let mut found = r.matching(plan, start, end);
        if let Ok(before) = start.checked_sub(jiff::Span::new().days(1)) {
            found.extend(
                r.matching(plan, early, before)
                    .into_iter()
                    .filter(|t| !claimed.contains(t.transaction_id.as_str())),
            );
        }
        candidates.insert(plan.plan_id.as_str(), found);
    }
    let mut usage: HashMap<&str, usize> = HashMap::new();
    for found in candidates.values() {
        for t in found {
            *usage.entry(t.transaction_id.as_str()).or_default() += 1;
        }
    }
    let mut ordered = dated;
    ordered.sort_by(|(a, ad), (b, bd)| (ad, &a.name, &a.plan_id).cmp(&(bd, &b.name, &b.plan_id)));
    ordered
        .into_iter()
        .map(|(plan, due)| {
            let found = &candidates[plan.plan_id.as_str()];
            let unique = match found.as_slice() {
                [only] if usage[only.transaction_id.as_str()] == 1 => Some(*only),
                _ => None,
            };
            // The same payment also set up as a bill: the bill keeps the
            // transaction, the plan is settled by it.
            let via_bill = if found.is_empty() {
                let (_, start, end) = windows[plan.plan_id.as_str()];
                r.matching_in(&r.billed, plan, start, end).first().copied()
            } else {
                None
            };
            let status = if unique.is_some() || via_bill.is_some() {
                PlanStatus::Paid
            } else if !found.is_empty() {
                PlanStatus::NeedsReview
            } else if plan.pending {
                PlanStatus::Pending
            } else if due < today {
                PlanStatus::Overdue
            } else {
                PlanStatus::Upcoming
            };
            PlannedPaymentView {
                plan_id: plan.plan_id.clone(),
                name: plan.name.clone(),
                match_category: plan.match_category.clone(),
                direction: plan.direction,
                amount: plan.amount.clone(),
                due_on: plan.due_on.clone(),
                created_at: plan.created_at.clone(),
                reimbursement_for: plan.reimbursement_for.clone(),
                pending: plan.pending,
                account_id: plan.account_id.clone(),
                match_description: plan.match_description.clone(),
                status,
                paid_on: unique.or(via_bill).map(|t| t.effective_on().to_string()),
                transaction_id: unique.map(|t| t.transaction_id.clone()),
                candidate_count: found.len(),
                candidates: found
                    .iter()
                    .map(|t| PlanMatchCandidate {
                        transaction_id: t.transaction_id.clone(),
                        occurred_on: t.effective_on().to_string(),
                        description: t
                            .description
                            .clone()
                            .unwrap_or_else(|| "Transaction description unavailable".into()),
                        amount: t.money(),
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Transactions that paid a planned outflow: known payments, not habits.
pub fn paid_outflow_ids(payments: &[PlannedPaymentView]) -> HashSet<String> {
    payments
        .iter()
        .filter(|p| p.status == PlanStatus::Paid && p.direction == Direction::Outflow)
        .filter_map(|p| p.transaction_id.clone())
        .collect()
}

/// Spending with only explicitly linked, received reimbursements netted: such
/// an inflow becomes a refund in the category of the outflow it pays back.
pub fn net_spending_transactions(transactions: &[Tx], payments: &[PlannedPaymentView]) -> Vec<Tx> {
    let by_id: HashMap<&str, &Tx> = transactions
        .iter()
        .map(|t| (t.transaction_id.as_str(), t))
        .collect();
    let mut linked: HashMap<&str, &Tx> = HashMap::new();
    for p in payments {
        let original = by_id.get(p.reimbursement_for.as_deref().unwrap_or(""));
        if let (Some(original), Some(paid_by)) = (original, &p.transaction_id)
            && p.direction == Direction::Inflow
            && p.status == PlanStatus::Paid
            && original.kind == Kind::Outflow
            && original.currency == p.amount.currency
        {
            linked.insert(paid_by.as_str(), original);
        }
    }
    transactions
        .iter()
        .map(|t| match linked.get(t.transaction_id.as_str()) {
            Some(original) => Tx {
                kind: Kind::Refund,
                category: original.category.clone(),
                subscription_id: original.subscription_id.clone(),
                ..t.clone()
            },
            None => t.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const ZONE: &str = "Europe/Amsterdam";

    fn now() -> Zoned {
        "2026-09-25T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap()
    }

    fn eur(amount: &str) -> UnitMoney {
        UnitMoney {
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
        }
    }

    fn plan(id: &str) -> PlannedPayment {
        PlannedPayment {
            plan_id: id.into(),
            name: "Insurance instalment".into(),
            match_category: "Insurance".into(),
            direction: Direction::Outflow,
            amount: eur("49.90"),
            due_on: "2026-09-25".into(),
            created_at: "2026-09-25T12:00:00Z".into(),
            reimbursement_for: None,
            pending: false,
            account_id: None,
            match_description: None,
        }
    }

    fn tx(id: &str, category: Option<&str>, amount: &str, when: &str, kind: Kind) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 1,
            account_id: "bank".into(),
            counterparty_account_id: None,
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: format!("{when}T12:00:00[{ZONE}]").parse().unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: category.map(Into::into),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn insurance(id: &str) -> Tx {
        tx(id, Some("Insurance"), "49.90", "2026-09-24", Kind::Outflow)
    }

    fn reconcile(plans: &[PlannedPayment], txs: &[Tx]) -> Vec<PlannedPaymentView> {
        reconcile_planned_payments(plans, &[], txs, &now(), &HashMap::new())
    }

    #[test]
    fn plan_is_paid_only_by_one_unique_matching_outflow() {
        let first = plan("first");
        assert_eq!(
            reconcile(std::slice::from_ref(&first), &[])[0].status,
            PlanStatus::Upcoming
        );
        let wrong = [
            tx(
                "other",
                Some("Groceries"),
                "49.90",
                "2026-09-24",
                Kind::Outflow,
            ),
            tx(
                "price",
                Some("Insurance"),
                "50",
                "2026-09-24",
                Kind::Outflow,
            ),
        ];
        assert_eq!(
            reconcile(std::slice::from_ref(&first), &wrong)[0].status,
            PlanStatus::Upcoming
        );
        let mut all = wrong.to_vec();
        all.push(insurance("match"));
        let paid = &reconcile(std::slice::from_ref(&first), &all)[0];
        assert_eq!(paid.status, PlanStatus::Paid);
        assert_eq!(paid.paid_on.as_deref(), Some("2026-09-24"));
        assert_eq!(paid.transaction_id.as_deref(), Some("match"));
        let mut linked = insurance("bill");
        linked.subscription_id = Some("bill-1".into());
        // A payment a bill claims settles the plan; the bill keeps it.
        let via_bill = &reconcile(std::slice::from_ref(&first), &[linked])[0];
        assert_eq!(via_bill.status, PlanStatus::Paid);
        assert_eq!(via_bill.paid_on.as_deref(), Some("2026-09-24"));
        assert_eq!(via_bill.transaction_id, None);
        let ambiguous = &reconcile(
            std::slice::from_ref(&first),
            &[insurance("a"), insurance("b")],
        )[0];
        assert_eq!(
            (ambiguous.status, ambiguous.candidate_count),
            (PlanStatus::NeedsReview, 2)
        );
        let shared = reconcile(&[first, plan("second")], &[insurance("a")]);
        assert!(shared.iter().all(|v| v.status == PlanStatus::NeedsReview));
    }

    #[test]
    fn bought_wish_plan_matches_its_payment_in_any_category() {
        let mut bought = plan("wish-zoo");
        bought.match_category = "Wishes".into();
        bought.amount = eur("114");
        bought.due_on = "2026-09-24".into();
        bought.account_id = Some("bank".into());
        let mut elsewhere = tx(
            "other-account",
            Some("Entertainment"),
            "114",
            "2026-09-24",
            Kind::Outflow,
        );
        elsewhere.account_id = "savings".into();
        let found = &reconcile(
            std::slice::from_ref(&bought),
            &[
                elsewhere,
                tx(
                    "price",
                    Some("Entertainment"),
                    "98",
                    "2026-09-24",
                    Kind::Outflow,
                ),
                tx(
                    "zoo",
                    Some("Entertainment"),
                    "114",
                    "2026-09-24",
                    Kind::Outflow,
                ),
            ],
        )[0];
        assert_eq!(found.status, PlanStatus::Paid);
        assert_eq!(found.transaction_id.as_deref(), Some("zoo"));
    }

    #[test]
    fn closure_purchase_matches_category_in_its_month_from_any_account() {
        let mut purchase = plan("closure-2026-08-01-laptop");
        purchase.match_category = "Electronics".into();
        purchase.amount = eur("1000");
        purchase.due_on = "2026-09-01".into();
        purchase.account_id = Some("savings".into());
        let e = Some("Electronics");
        let found = &reconcile(
            &[purchase],
            &[
                tx("too-dear", e, "1100.01", "2026-09-24", Kind::Outflow),
                tx("too-cheap", e, "5", "2026-09-24", Kind::Outflow),
                tx("before", e, "1000", "2026-08-31", Kind::Outflow),
                tx("laptop", e, "1049.99", "2026-09-24", Kind::Outflow),
            ],
        )[0];
        assert_eq!(found.status, PlanStatus::Paid);
        assert_eq!(found.transaction_id.as_deref(), Some("laptop"));
    }

    #[test]
    fn owner_plan_paid_early_after_it_was_created_counts_as_paid() {
        let mut lessons = plan("lessons");
        lessons.match_category = "Driving Lessons".into();
        lessons.due_on = "2026-10-06".into();
        lessons.created_at = "2026-09-14T12:00:00Z".into();
        let d = Some("Driving Lessons");
        let found = &reconcile(
            &[lessons],
            &[
                tx("before-plan", d, "49.90", "2026-09-13", Kind::Outflow),
                tx("early", d, "49.90", "2026-09-24", Kind::Outflow),
            ],
        )[0];
        assert_eq!(found.status, PlanStatus::Paid);
        assert_eq!(found.transaction_id.as_deref(), Some("early"));
    }

    #[test]
    fn expected_income_is_completed_only_by_inflow() {
        let mut expected = plan("income");
        expected.direction = Direction::Inflow;
        let debit = insurance("debit");
        assert_eq!(
            reconcile(&[expected.clone()], &[debit])[0].status,
            PlanStatus::Upcoming
        );
        let received = tx(
            "salary",
            Some("Insurance"),
            "49.90",
            "2026-09-24",
            Kind::Inflow,
        );
        assert_eq!(
            reconcile(&[expected], &[received])[0].status,
            PlanStatus::Paid
        );
    }

    #[test]
    fn salary_is_paid_by_categorised_inflow_with_a_different_amount() {
        let mut expected = plan("salary");
        expected.direction = Direction::Inflow;
        expected.match_category = "Salary".into();
        let pay = tx("pay", Some("Salary"), "3012.40", "2026-09-24", Kind::Inflow);
        let view = &reconcile(&[expected], &[pay])[0];
        assert_eq!(view.transaction_id.as_deref(), Some("pay"));
    }

    #[test]
    fn reservation_matches_bank_amount_and_merchant_without_category() {
        let mut pending = plan("pending");
        pending.pending = true;
        pending.account_id = Some("bank".into());
        pending.match_description = Some("Example Bar".into());
        let mut posted = tx("posted", None, "49.90", "2026-10-05", Kind::Outflow);
        posted.description = Some("CARD PAYMENT EXAMPLE BAR CITY".into());
        let later: Zoned = "2026-10-06T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        let run = |txs: &[Tx]| {
            let none = HashMap::new();
            reconcile_planned_payments(&[pending.clone()], &[], txs, &later, &none)[0].clone()
        };
        assert_eq!(run(&[]).status, PlanStatus::Pending);
        let mut other_account = posted.clone();
        other_account.account_id = "another-bank".into();
        let mut other_amount = posted.clone();
        other_amount.amount = "49.91".parse().unwrap();
        let mut other_bar = posted.clone();
        other_bar.description = Some("Other bar".into());
        other_bar.category = Some("Insurance".into());
        let mut barge = posted.clone();
        barge.description = Some("Example Barge".into());
        let mut early = posted.clone();
        early.occurred_at = "2026-09-17T12:00:00[Europe/Amsterdam]".parse().unwrap();
        assert_eq!(
            run(&[other_account, other_amount, other_bar, barge, early]).status,
            PlanStatus::Pending
        );
        let matched = run(&[posted.clone()]);
        assert_eq!(matched.transaction_id.as_deref(), Some("posted"));
        let mut twin = posted.clone();
        twin.transaction_id = "other".into();
        assert_eq!(run(&[posted, twin]).status, PlanStatus::NeedsReview);
    }

    #[test]
    fn monthly_plans_created_early_do_not_share_one_payment() {
        let mut october = plan("oct");
        october.due_on = "2026-10-15".into();
        october.created_at = "2026-09-20T12:00:00Z".into();
        let mut november = october.clone();
        november.plan_id = "nov".into();
        november.due_on = "2026-11-15".into();
        let paid = [tx(
            "oct-payment",
            Some("Insurance"),
            "49.90",
            "2026-10-15",
            Kind::Outflow,
        )];
        let checked: Zoned = "2026-10-20T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        let none = HashMap::new();
        let both = [october.clone(), november.clone()];
        let views = reconcile_planned_payments(&both, &[], &paid, &checked, &none);
        assert_eq!(views[0].transaction_id.as_deref(), Some("oct-payment"));
        assert_eq!(views[1].status, PlanStatus::Upcoming);
        // Archiving the paid October plan must not hand its payment to November.
        let alone = reconcile_planned_payments(&[november], &[october], &paid, &checked, &none);
        assert_eq!(alone[0].status, PlanStatus::Upcoming);
    }

    #[test]
    fn original_category_of_shared_spending_still_matches() {
        let shared = tx(
            "shared",
            Some("Gifts"),
            "49.90",
            "2026-09-24",
            Kind::Outflow,
        );
        let originals = HashMap::from([("shared".to_owned(), Some("Insurance".to_owned()))]);
        let views = reconcile_planned_payments(&[plan("p")], &[], &[shared], &now(), &originals);
        assert_eq!(views[0].status, PlanStatus::Paid);
    }

    #[test]
    fn recurring_salary_fills_months_without_a_hand_entered_plan() {
        let settings = OwnerSettings {
            payday_day: Some(31),
            monthly_salary_eur: Some("3000".parse().unwrap()),
            ..OwnerSettings::default()
        };
        let mut manual = plan("manual-salary");
        manual.direction = Direction::Inflow;
        manual.match_category = " salary ".into();
        manual.due_on = "2026-10-30".into();
        let plans = with_recurring_salary(&[manual], &settings, &now(), Some(date(2026, 8, 20)));
        let generated: Vec<(&str, &str)> = plans[1..]
            .iter()
            .map(|p| (p.plan_id.as_str(), p.due_on.as_str()))
            .collect();
        assert_eq!(
            generated,
            [
                ("recurring-salary-2026-08-01", "2026-08-31"),
                ("recurring-salary-2026-09-01", "2026-09-30"),
                ("recurring-salary-2026-11-01", "2026-11-30"),
                ("recurring-salary-2026-12-01", "2026-12-31"),
            ]
        );
        assert_eq!(plans[1].created_at, "2026-09-25T14:00:00+02:00");
        assert!(with_recurring_salary(&[], &OwnerSettings::default(), &now(), None).is_empty());
    }

    #[test]
    fn received_reimbursement_becomes_a_refund_of_the_original() {
        let mut original = tx(
            "dinner",
            Some("Restaurants"),
            "80",
            "2026-09-20",
            Kind::Outflow,
        );
        original.subscription_id = Some("bill".into());
        let back = tx("back", Some("Transfers"), "40", "2026-09-24", Kind::Inflow);
        let mut expected = plan("back-plan");
        expected.direction = Direction::Inflow;
        expected.match_category = "Transfers".into();
        expected.amount = eur("40");
        expected.reimbursement_for = Some("dinner".into());
        let txs = [original, back, insurance("bill-payment")];
        let views = reconcile(&[expected, plan("insurance")], &txs);
        let netted = net_spending_transactions(&txs, &views);
        assert_eq!(netted[1].kind, Kind::Refund);
        assert_eq!(netted[1].category.as_deref(), Some("Restaurants"));
        assert_eq!(netted[1].subscription_id.as_deref(), Some("bill"));
        assert_eq!(netted[0], txs[0]);
        assert_eq!(
            paid_outflow_ids(&views),
            HashSet::from(["bill-payment".to_owned()])
        );
    }
}
