//! The running month's spending groups against the owner's own norm: the
//! budget's rows plus a zero-amount "shadow" row for every group without a
//! budget, and how often the owner goes where. Everything is counted by the
//! budget engine's own months and ownership, so it agrees with Overview.

use std::collections::HashMap;

use finance_types::gateway::{BudgetGroup, BudgetPlan};
use finance_types::report::adjustments::{AdjustmentRow, Adjustments, Habit, StoreHabit};
use finance_types::report::plans::PlannedPaymentView;
use finance_types::report::subscriptions::SubscriptionReport;
use jiff::ToSpan;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::budget::{
    BudgetInputs, HISTORY_MONTHS, Ownership, budget_fixed_ids, counts, held_currencies, month_of,
    monthly_budget, parse_date, plan_for, signed,
};
use crate::context::Ctx;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::reporting::week_start;
use crate::trends::median;

/// Groups that never get a row of their own: money moved rather than spent,
/// and spending the owner has not sorted yet.
pub const NO_SHADOW: [&str; 3] = ["Transfers", "Investments", "Other"];
const UNSORTED_GROUP: &str = "Other";
const SHADOW: &str = "shadow:";
/// The full weeks before this one that make the usual week.
const FULL_WEEKS: usize = 12;
/// The full weeks that, with this one, count as "lately".
const RECENT_WEEKS: usize = 4;

fn plan_month(plan: &BudgetPlan, inputs: &BudgetInputs) -> Date {
    parse_date(&plan.month).unwrap_or_else(|| inputs.now.date().first_of_month())
}

/// The six budget months a plan's history covers, oldest first.
fn history_months(month: Date) -> Vec<Date> {
    (1 - HISTORY_MONTHS..=0)
        .map(|offset| shift_month(month, offset))
        .collect()
}

/// `plan` with a zero-amount group (id `shadow:<group>`) for every reviewed
/// group that has spending no real group claims in the plan's six months,
/// in first-seen order. Never saved.
pub fn shadow_plan(plan: &BudgetPlan, inputs: &BudgetInputs) -> BudgetPlan {
    let months = history_months(plan_month(plan, inputs));
    let ownership = Ownership::new(plan, inputs);
    let mut uncovered: Vec<&str> = Vec::new();
    for t in inputs.transactions {
        if !counts(t, plan, inputs.now)
            || !months.contains(&month_of(t, inputs))
            || ownership.key(t) != "unplanned"
        {
            continue;
        }
        if let Some(group) = ownership.source_group(t.category.as_deref())
            && !NO_SHADOW.contains(&group)
            && !uncovered.contains(&group)
        {
            uncovered.push(group);
        }
    }
    let mut groups = plan.groups.clone();
    groups.extend(uncovered.into_iter().map(|group| BudgetGroup {
        id: format!("{SHADOW}{group}"),
        name: group.to_owned(),
        amount: Decimal::ZERO,
        source_groups: vec![group.to_owned()],
        categories: Vec::new(),
        transaction_ids: Vec::new(),
        account_id: None,
        importance: None,
        floor: None,
    }));
    BudgetPlan {
        groups,
        ..plan.clone()
    }
}

/// Visits and spending of one row or store, by budget month and by week.
#[derive(Default)]
struct Tally {
    month_visits: [usize; HISTORY_MONTHS as usize],
    month_spent: [Decimal; HISTORY_MONTHS as usize],
    week_visits: [usize; FULL_WEEKS],
    this_week: usize,
    weekdays: [usize; 7],
}

impl Tally {
    fn habit(&self) -> Habit {
        let last = self.month_visits.len() - 1;
        let earlier: Vec<usize> = (0..last).filter(|i| self.month_visits[*i] > 0).collect();
        let typical = |values: Vec<Decimal>| (values.len() >= 2).then(|| median(&values)).flatten();
        let weeks: Vec<Decimal> = self.week_visits.iter().map(|n| Decimal::from(*n)).collect();
        Habit {
            visits: self.month_visits[last],
            spent: self.month_spent[last],
            typical_visits: typical(
                earlier
                    .iter()
                    .map(|i| Decimal::from(self.month_visits[*i]))
                    .collect(),
            ),
            typical_spent: typical(earlier.iter().map(|i| self.month_spent[*i]).collect()),
            history_month_count: earlier.len(),
            visits_this_week: self.this_week,
            typical_week_visits: median(&weeks).unwrap_or_default(),
            recent_visits: self.this_week
                + self.week_visits[FULL_WEEKS - RECENT_WEEKS..]
                    .iter()
                    .sum::<usize>(),
            weekday_visits: self.weekdays,
        }
    }
}

/// One currency's rows for the plan's month: the budget's groups, then a
/// shadow row per group without a budget, each with its visit habits.
pub fn adjustments(
    plan: &BudgetPlan,
    subscriptions: &SubscriptionReport,
    period_start: Option<Date>,
    inputs: &BudgetInputs,
) -> Adjustments {
    let shadow = shadow_plan(plan, inputs);
    let budget = monthly_budget(&shadow, subscriptions, period_start, inputs);
    let ownership = Ownership::new(&shadow, inputs);
    let months = history_months(plan_month(plan, inputs));
    let today = inputs.now.date();
    let this_week = week_start(today);
    let first_week = this_week
        .checked_sub((FULL_WEEKS as i64).weeks())
        .unwrap_or(this_week);
    let mut rows: HashMap<&str, Tally> = HashMap::new();
    let mut stores: HashMap<(&str, &str), Tally> = HashMap::new();
    let mut unsorted = Decimal::ZERO;
    for t in inputs.transactions {
        if !counts(t, plan, inputs.now) {
            continue;
        }
        let key = ownership.key(t);
        let month = months.iter().position(|m| *m == month_of(t, inputs));
        let category = t.category.as_deref();
        if key == "unplanned" {
            let group = ownership.source_group(category);
            if month == Some(months.len() - 1)
                && (category.is_none() || group.is_none_or(|g| g == UNSORTED_GROUP))
            {
                unsorted += signed(t);
            }
            continue;
        }
        let on: Date = t.effective_on();
        if matches!(key, "fixed" | "extras") || (month.is_none() && on < first_week) {
            continue;
        }
        let visit = t.kind == Kind::Outflow;
        let count = |tally: &mut Tally| {
            if let Some(index) = month {
                tally.month_spent[index] += signed(t);
                tally.month_visits[index] += usize::from(visit);
            }
            if !visit || on > today {
                return;
            }
            if on >= this_week {
                tally.this_week += 1;
            } else if on >= first_week {
                let week = usize::try_from((on - first_week).get_days() / 7).unwrap_or(0);
                tally.week_visits[week.min(FULL_WEEKS - 1)] += 1;
                tally.weekdays
                    [usize::try_from(on.weekday().to_monday_zero_offset()).unwrap_or(0)] += 1;
            }
        };
        count(rows.entry(key).or_default());
        if let Some(category) = category {
            count(stores.entry((key, category)).or_default());
        }
    }
    let rows = budget
        .groups
        .into_iter()
        .map(|row| {
            let mut row_stores: Vec<StoreHabit> = stores
                .iter()
                .filter(|((id, _), _)| *id == row.id)
                .map(|((_, category), tally)| StoreHabit {
                    category: (*category).to_owned(),
                    habit: tally.habit(),
                })
                .collect();
            row_stores.sort_by(|a, b| {
                (b.habit.recent_visits, &a.category).cmp(&(a.habit.recent_visits, &b.category))
            });
            AdjustmentRow {
                budgeted: !row.id.starts_with(SHADOW),
                group: row.id.strip_prefix(SHADOW).map(str::to_owned),
                habit: rows
                    .get(row.id.as_str())
                    .map(Tally::habit)
                    .unwrap_or_default(),
                stores: row_stores,
                row,
            }
        })
        .collect();
    Adjustments {
        currency: plan.currency.clone(),
        month: shadow.month.clone(),
        rows,
        unsorted,
    }
}

/// The running budget month for every currency the owner holds or spends
/// (the dashboard's `adjustments`).
pub fn all(
    ctx: &Ctx,
    spending: &[Tx],
    payments: &[PlannedPaymentView],
    subscriptions: &SubscriptionReport,
) -> Vec<Adjustments> {
    let fixed_ids = budget_fixed_ids(ctx, spending);
    let month = ctx.current_month();
    let inputs = BudgetInputs {
        transactions: spending,
        mappings: &ctx.snap.group_mappings,
        now: &ctx.now,
        fixed_ids: &fixed_ids,
        payments,
        resolver: Some(&ctx.resolver),
        current_month: Some(month),
    };
    held_currencies(ctx, spending)
        .values()
        .map(|currency| {
            adjustments(
                &plan_for(&ctx.snap.budget_plans, month, currency),
                subscriptions,
                Some(ctx.resolver.period_start_date(month)),
                &inputs,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use finance_types::gateway::GroupMapping;
    use finance_types::report::Currency;
    use finance_types::report::closure::MonthPeriod;
    use jiff::civil::date;
    use jiff::tz::TimeZone;
    use rust_decimal::Decimal;

    use super::*;
    use crate::budget::default_budget;
    use crate::budget::tests::{empty_subscriptions, now, tx};
    use crate::period::PeriodResolver;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    /// A purchase at noon UTC on `day`.
    fn at(id: &str, amount: &str, category: Option<&str>, day: &str) -> Tx {
        Tx {
            category: category.map(Into::into),
            occurred_at: format!("{day}T12:00:00+00:00[UTC]").parse().unwrap(),
            ..tx(id, amount)
        }
    }

    fn mappings() -> Vec<GroupMapping> {
        [
            ("Jumbo", "Groceries"),
            ("Zara", "Shopping"),
            ("Wise", "Transfers"),
            ("Misc", "Other"),
        ]
        .into_iter()
        .map(|(category, group)| GroupMapping {
            category: category.into(),
            group: Some(group.into()),
            revision: 1,
        })
        .collect()
    }

    fn plan() -> BudgetPlan {
        default_budget(date(2026, 9, 1), &Currency::eur())
    }

    fn run(plan: &BudgetPlan, transactions: &[Tx], fixed: &[&str]) -> Adjustments {
        let fixed_ids: HashSet<String> = fixed.iter().map(|s| (*s).to_owned()).collect();
        let mappings = mappings();
        let now = now();
        adjustments(
            plan,
            &empty_subscriptions(),
            None,
            &BudgetInputs {
                transactions,
                mappings: &mappings,
                now: &now,
                fixed_ids: &fixed_ids,
                payments: &[],
                resolver: None,
                current_month: None,
            },
        )
    }

    fn row<'a>(report: &'a Adjustments, id: &str) -> &'a AdjustmentRow {
        report.rows.iter().find(|r| r.row.id == id).unwrap()
    }

    #[test]
    fn uncovered_groups_get_shadow_rows_and_excluded_ones_do_not() {
        let report = run(
            &plan(),
            &[
                at("1", "10", Some("Jumbo"), "2026-09-02"),
                at("2", "40", Some("Zara"), "2026-08-14"),
                at("3", "500", Some("Wise"), "2026-09-03"),
                at("4", "30", Some("Misc"), "2026-09-03"),
            ],
            &[],
        );
        let ids: Vec<&str> = report.rows.iter().map(|r| r.row.id.as_str()).collect();
        assert_eq!(ids, ["groceries", "food-out", "shadow:Shopping"]);
        let shopping = row(&report, "shadow:Shopping");
        assert!(!shopping.budgeted);
        assert_eq!(shopping.group.as_deref(), Some("Shopping"));
        assert_eq!(shopping.row.planned, d(0));
        assert_eq!(shopping.row.name, "Shopping");
        assert!(row(&report, "groceries").budgeted);
        assert_eq!(report.month, "2026-09-01");
    }

    #[test]
    fn a_purchase_a_real_group_claims_counts_once() {
        let mut plan = plan();
        plan.groups[0].categories = vec!["Zara".into()];
        let report = run(&plan, &[at("1", "40", Some("Zara"), "2026-09-02")], &[]);
        assert_eq!(row(&report, "groceries").row.spent, d(40));
        assert_eq!(row(&report, "groceries").habit.visits, 1);
        assert!(report.rows.iter().all(|r| r.row.id != "shadow:Shopping"));
        let total: Decimal = report.rows.iter().map(|r| r.row.spent).sum();
        assert_eq!(total, d(40));
    }

    fn jumbo_history() -> Vec<Tx> {
        let days = [
            "2026-07-07",
            "2026-07-21",
            "2026-08-04",
            "2026-08-11",
            "2026-08-18",
            "2026-08-25",
            "2026-09-01",
            "2026-09-08",
            "2026-09-09",
        ];
        let mut rows: Vec<Tx> = days
            .iter()
            .enumerate()
            .map(|(i, day)| at(&i.to_string(), "10", Some("Jumbo"), day))
            .collect();
        rows.push(Tx {
            kind: Kind::Refund,
            ..at("refund", "5", Some("Jumbo"), "2026-09-09")
        });
        rows
    }

    #[test]
    fn habits_count_visits_per_month_and_week() {
        let report = run(&plan(), &jumbo_history(), &[]);
        let groceries = row(&report, "groceries");
        let habit = &groceries.habit;
        assert_eq!((habit.visits, habit.spent), (3, d(25)));
        assert_eq!(habit.typical_visits, Some(d(3)));
        assert_eq!(habit.typical_spent, Some(d(30)));
        assert_eq!(habit.history_month_count, 2);
        // The week starts Monday 7 September; today is Thursday the 10th.
        assert_eq!(habit.visits_this_week, 2);
        assert_eq!(habit.typical_week_visits, d(1));
        assert_eq!(habit.recent_visits, 6);
        // Seven Tuesdays in the twelve full weeks from 15 June.
        assert_eq!(habit.weekday_visits, [0, 7, 0, 0, 0, 0, 0]);
        assert_eq!(groceries.stores.len(), 1);
        assert_eq!(groceries.stores[0].category, "Jumbo");
        assert_eq!(groceries.stores[0].habit, *habit);
    }

    #[test]
    fn zero_weeks_count_in_the_usual_week() {
        let rows: Vec<Tx> = [
            "2026-07-07",
            "2026-07-14",
            "2026-07-21",
            "2026-08-04",
            "2026-08-11",
        ]
        .iter()
        .enumerate()
        .map(|(i, day)| at(&i.to_string(), "10", Some("Jumbo"), day))
        .collect();
        let report = run(&plan(), &rows, &[]);
        assert_eq!(row(&report, "groceries").habit.typical_week_visits, d(0));
    }

    #[test]
    fn fixed_spending_is_left_out() {
        let report = run(
            &plan(),
            &[
                at("bill", "90", Some("Jumbo"), "2026-09-02"),
                at("shop", "10", Some("Jumbo"), "2026-09-03"),
            ],
            &["bill"],
        );
        let habit = &row(&report, "groceries").habit;
        assert_eq!((habit.visits, habit.spent), (1, d(10)));
    }

    #[test]
    fn stores_sort_by_recent_visits() {
        let mut rows = vec![at("a", "10", Some("Aldi"), "2026-09-01")];
        rows.extend(jumbo_history());
        let mut mappings_plan = plan();
        mappings_plan.groups[0].categories = vec!["Aldi".into()];
        let report = run(&mappings_plan, &rows, &[]);
        let stores: Vec<&str> = row(&report, "groceries")
            .stores
            .iter()
            .map(|s| s.category.as_str())
            .collect();
        assert_eq!(stores, ["Jumbo", "Aldi"]);
    }

    #[test]
    fn unsorted_is_uncategorised_unmapped_and_other() {
        let report = run(
            &plan(),
            &[
                at("1", "10", None, "2026-09-02"),
                at("2", "20", Some("Nowhere"), "2026-09-02"),
                at("3", "30", Some("Misc"), "2026-09-02"),
                at("4", "500", Some("Wise"), "2026-09-02"),
                at("5", "7", Some("Misc"), "2026-08-02"),
            ],
            &[],
        );
        assert_eq!(report.unsorted, d(60));
    }

    #[test]
    fn an_early_closed_month_follows_the_budget_month() {
        let periods = vec![MonthPeriod {
            balance_checks: BTreeMap::new(),
            month: "2026-08-01".into(),
            starts_at: "2026-08-01T00:00:00+00:00".into(),
            cutoff_at: Some("2026-08-28T10:00:00Z".into()),
            status: Default::default(),
            settled_at: None,
            settled_snapshot: None,
            missing_banks: Vec::new(),
            allocations: Vec::new(),
            revision: 0,
            reviewed_through: None,
            reopened_at: None,
        }];
        let resolver = PeriodResolver::new(&periods, HashMap::new(), TimeZone::UTC);
        let transactions = [
            at("1", "10", Some("Jumbo"), "2026-08-30"),
            at("2", "10", Some("Jumbo"), "2026-08-20"),
        ];
        let (mappings, now, fixed_ids) = (mappings(), now(), HashSet::new());
        let report = adjustments(
            &plan(),
            &empty_subscriptions(),
            Some(resolver.period_start_date(date(2026, 9, 1))),
            &BudgetInputs {
                transactions: &transactions,
                mappings: &mappings,
                now: &now,
                fixed_ids: &fixed_ids,
                payments: &[],
                resolver: Some(&resolver),
                current_month: Some(date(2026, 9, 1)),
            },
        );
        let groceries = row(&report, "groceries");
        assert_eq!(groceries.row.spent, d(10));
        assert_eq!(groceries.habit.visits, 1);
        assert_eq!(groceries.row.elapsed_days, 14);
    }
}
