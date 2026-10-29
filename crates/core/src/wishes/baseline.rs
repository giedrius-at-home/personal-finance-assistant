//! Each month's starting position, read from the monthly allocation.
//!
//! Pure arithmetic, no I/O. Month 0 is the allocation's current month, month 1
//! the funded month, and later months repeat month 1 with nothing spent yet.
//! Month 0's card usage is what the owner typed; each later month starts with
//! the card bill's amount as its regular card spending.

use super::ladder::{GroupRoom, MonthBaseline};
use crate::period::shift_month;
use crate::text::casefold;
use finance_types::gateway::{BudgetPlan, WishSettings};
use finance_types::report::allocation::{CurrencyAllocation, GroupAllocation};
use jiff::civil::Date;
use rust_decimal::Decimal;

/// Parse a `YYYY-MM-DD` or `YYYY-MM` month as its first day.
fn parse_month(text: &str) -> Option<Date> {
    text.parse::<Date>()
        .or_else(|_| format!("{text}-01").parse::<Date>())
        .ok()
        .map(|d| d.first_of_month())
}

/// The allocation's current month, as its first day.
pub fn current_month(alloc: &CurrencyAllocation) -> Option<Date> {
    parse_month(&alloc.current_month)
}

/// The plan for `month` in the allocation's currency, else the latest earlier one.
fn plan_for<'a>(
    plans: &'a [BudgetPlan],
    alloc: &CurrencyAllocation,
    month: Date,
) -> Option<&'a BudgetPlan> {
    plans
        .iter()
        .filter(|p| p.currency == alloc.currency)
        .filter_map(|p| parse_month(&p.month).map(|m| (m, p)))
        .filter(|(m, _)| *m <= month)
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}

/// The ranked groups among `rows`; `keep_spent` false treats nothing as spent.
fn rooms(rows: &[GroupAllocation], plan: Option<&BudgetPlan>, keep_spent: bool) -> Vec<GroupRoom> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let key = casefold(&row.group);
            let group = plan.groups.iter().find(|g| casefold(&g.name) == key)?;
            let importance = group.importance?;
            let spent = if keep_spent { row.spent } else { Decimal::ZERO };
            Some(GroupRoom {
                id: group.id.clone(),
                name: row.group.clone(),
                importance,
                limit: row.limit,
                min: group.floor.unwrap_or(Decimal::ZERO).max(spent),
            })
        })
        .collect()
}

/// One baseline per month for `months` months, and whether the recorded card
/// usage is from an earlier month.
pub fn baselines(
    alloc: &CurrencyAllocation,
    plans: &[BudgetPlan],
    card_bill: Decimal,
    settings: &WishSettings,
    earmarked: Decimal,
    months: usize,
) -> (Vec<MonthBaseline>, bool) {
    let Some(month0) = current_month(alloc) else {
        return (Vec::new(), false);
    };
    let (card_used, card_stale) = if parse_month(&settings.card_used_month) == Some(month0) {
        (settings.card_used, false)
    } else {
        let unset = settings.card_used_month.is_empty() && settings.card_used.is_zero();
        (Decimal::ZERO, !unset)
    };
    // `next_wiggle_room` is clamped at zero; the deficit is `next_shortfall`.
    // The card bill is a fixed cost there; every later month pays the card
    // through the ladder instead (last month's usage), so it is added back
    // and the month's own regular card spending is seeded as its usage.
    let next_free = alloc.next_wiggle_room - alloc.next_shortfall + card_bill;
    let out = (0..months)
        .map(|i| {
            let month = shift_month(month0, i32::try_from(i).unwrap_or(i32::MAX));
            let plan = plan_for(plans, alloc, month);
            match i {
                0 => MonthBaseline {
                    month,
                    free: alloc.current_wiggle_room - earmarked,
                    groups: rooms(&alloc.groups, plan, true),
                    card_used,
                },
                _ => MonthBaseline {
                    month,
                    free: next_free,
                    groups: rooms(&alloc.next_group_rows, plan, i == 1),
                    card_used: card_bill,
                },
            }
        })
        .collect();
    (out, card_stale)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use finance_types::gateway::BudgetGroup;
    use finance_types::report::Currency;
    use std::str::FromStr;

    pub(crate) fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    pub(crate) fn row(group: &str, limit: &str, spent: &str) -> GroupAllocation {
        GroupAllocation {
            group: group.into(),
            limit: d(limit),
            spent: d(spent),
            remaining: d(limit) - d(spent),
            projected_month_end: d(spent),
        }
    }

    pub(crate) fn alloc(
        wiggle: &str,
        next_wiggle: &str,
        groups: Vec<GroupAllocation>,
        next_groups: Vec<GroupAllocation>,
    ) -> CurrencyAllocation {
        CurrencyAllocation {
            currency: Currency::eur(),
            current_month: "2026-10-01".into(),
            funded_month: "2026-11-01".into(),
            bank_balance: Decimal::ZERO,
            protected_savings: Decimal::ZERO,
            salary_reserved: Decimal::ZERO,
            salary_received_on: None,
            current_month_funds: Decimal::ZERO,
            current_fixed_remaining: Decimal::ZERO,
            current_plans_remaining: Decimal::ZERO,
            current_groups_remaining: Decimal::ZERO,
            current_wiggle_room: d(wiggle),
            salary_at_risk: Decimal::ZERO,
            next_fixed: Decimal::ZERO,
            next_goals: Decimal::ZERO,
            next_plans: Decimal::ZERO,
            next_groups: Decimal::ZERO,
            next_wiggle_room: d(next_wiggle),
            next_shortfall: Decimal::ZERO,
            groups,
            banks: Vec::new(),
            next_group_rows: next_groups,
            current_expected_income: Decimal::ZERO,
            next_expected_income: Decimal::ZERO,
            expected_salary: Decimal::ZERO,
            next_planned_funds: Decimal::ZERO,
            current_planned_wiggle_room: Decimal::ZERO,
            savings_balance: Decimal::ZERO,
        }
    }

    pub(crate) fn plan_group(
        id: &str,
        name: &str,
        importance: Option<u8>,
        floor: Option<&str>,
    ) -> BudgetGroup {
        BudgetGroup {
            id: id.into(),
            name: name.into(),
            amount: Decimal::ZERO,
            source_groups: Vec::new(),
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance,
            floor: floor.map(d),
        }
    }

    pub(crate) fn plan(month: &str, groups: Vec<BudgetGroup>) -> BudgetPlan {
        BudgetPlan {
            month: month.into(),
            currency: Currency::eur(),
            revision: 0,
            groups,
        }
    }

    fn settings(card_used: &str, month: &str) -> WishSettings {
        WishSettings {
            card_used: d(card_used),
            card_used_month: month.into(),
            ..WishSettings::default()
        }
    }

    #[test]
    fn month_zero_uses_wiggle_room_minus_earmark() {
        let a = alloc("400", "0", vec![], vec![]);
        let (months, _) = baselines(&a, &[], Decimal::ZERO, &settings("0", ""), d("300"), 2);
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].free, d("100"));
        assert_eq!(months[0].month, Date::new(2026, 10, 1).unwrap());
        assert_eq!(months[1].month, Date::new(2026, 11, 1).unwrap());
    }

    #[test]
    fn importance_comes_from_budget_plan() {
        let a = alloc(
            "0",
            "0",
            vec![
                row("Going out", "120", "20"),
                row("Groceries", "450", "200"),
                row("Coffee", "40", "0"),
                row("Unplanned", "40", "0"),
            ],
            vec![row("going out", "120", "0"), row("groceries", "450", "0")],
        );
        let plans = [plan(
            "2026-09-01",
            vec![
                plan_group("g-out", "going out", Some(2), None),
                plan_group("g-groc", "groceries", Some(1), Some("300")),
                plan_group("g-coffee", "Coffee", None, None),
            ],
        )];
        let (months, _) = baselines(
            &a,
            &plans,
            Decimal::ZERO,
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        let g = &months[0].groups;
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].id, "g-out");
        assert_eq!(g[0].name, "Going out");
        assert_eq!(g[0].importance, 2);
        assert_eq!(g[0].limit, d("120"));
        assert_eq!(g[0].min, d("20"));
        assert_eq!(g[1].id, "g-groc");
        assert_eq!(g[1].importance, 1);
        assert_eq!(g[1].min, d("300"));
        assert!(
            months
                .iter()
                .all(|m| m.groups.iter().all(|g| g.name != "Coffee"))
        );
        assert_eq!(months[1].groups.len(), 2);
        assert_eq!(months[2].groups[0].min, Decimal::ZERO);
        assert_eq!(months[2].groups[1].min, d("300"));
    }

    #[test]
    fn later_months_use_spent_zero_template() {
        let a = alloc("0", "50", vec![], vec![row("going out", "120", "30")]);
        let plans = [plan(
            "2026-10-01",
            vec![plan_group("g", "Going Out", Some(2), None)],
        )];
        let (months, _) = baselines(
            &a,
            &plans,
            Decimal::ZERO,
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[1].groups[0].min, d("30"));
        assert_eq!(months[2].groups[0].min, Decimal::ZERO);
        assert_eq!(months[2].free, d("50"));
        assert_eq!(months[2].month, Date::new(2026, 12, 1).unwrap());
        assert_eq!(months[1].card_used, Decimal::ZERO);
    }

    #[test]
    fn stale_card_usage_counts_as_zero() {
        let a = alloc("0", "0", vec![], vec![]);
        let (months, stale) = baselines(
            &a,
            &[],
            Decimal::ZERO,
            &settings("800", "2026-09-01"),
            Decimal::ZERO,
            2,
        );
        assert_eq!(months[0].card_used, Decimal::ZERO);
        assert!(stale);
    }

    #[test]
    fn current_card_usage_is_used() {
        let a = alloc("0", "0", vec![], vec![]);
        let (months, stale) = baselines(
            &a,
            &[],
            Decimal::ZERO,
            &settings("800", "2026-10-01"),
            Decimal::ZERO,
            2,
        );
        assert_eq!(months[0].card_used, d("800"));
        assert!(!stale);
        let (_, stale) = baselines(&a, &[], Decimal::ZERO, &settings("0", ""), Decimal::ZERO, 2);
        assert!(!stale);
    }

    #[test]
    fn next_month_shortfall_makes_free_negative() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.next_shortfall = d("50");
        let (months, _) = baselines(&a, &[], d("9.98"), &settings("0", ""), Decimal::ZERO, 3);
        assert_eq!(months[1].free, d("-40.02"));
        assert_eq!(months[2].free, d("-40.02"));
    }

    #[test]
    fn regular_card_spending_carries_into_every_later_month() {
        let a = alloc("0", "200", vec![], vec![]);
        let (months, _) = baselines(
            &a,
            &[],
            d("100"),
            &settings("40", "2026-10-01"),
            Decimal::ZERO,
            4,
        );
        assert_eq!(months[0].card_used, d("40"));
        for m in &months[1..] {
            assert_eq!(m.free, d("300"));
            assert_eq!(m.card_used, d("100"));
        }
        // Each later month pays the month before's regular card spending.
        let judged =
            super::super::ladder::judge(&months, &[vec![], vec![], vec![], vec![]], d("2500"));
        assert_eq!(judged[1].card_charge_in, d("40"));
        assert_eq!(judged[2].card_charge_in, d("100"));
        assert_eq!(judged[3].card_charge_in, d("100"));
        assert_eq!(judged[3].free_left, d("200"));
    }

    #[test]
    fn card_bill_added_back_next_month() {
        let a = alloc("0", "200", vec![], vec![]);
        let (months, _) = baselines(&a, &[], d("9.98"), &settings("0", ""), Decimal::ZERO, 2);
        assert_eq!(months[1].free, d("209.98"));
        assert_eq!(months[0].free, Decimal::ZERO);
    }
}
