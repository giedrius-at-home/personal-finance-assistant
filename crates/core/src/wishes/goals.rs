//! The wishes and the emergency fund against current habits: the ladder's
//! baselines rewritten so every spending group costs its typical month, and
//! what that judging says to cut. Pure arithmetic, nothing is saved.

use std::collections::HashMap;

use finance_types::gateway::{BudgetPlan, Repeat};
use finance_types::report::Currency;
use finance_types::report::adjustments::Adjustments;
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::wishes::{EmergencyFund, HabitCut, WishMonth};
use rust_decimal::Decimal;

use super::baseline::{parse_month, ranking};
use super::ladder::{GroupRoom, MonthBaseline};
use super::schedule::Planned;
use crate::period::shift_month;
use crate::text::casefold;
use crate::trends::median;

/// Earlier months with spending a typical month needs (as the Adjustments
/// card asks).
const TYPICAL_MONTHS: usize = 3;
/// An essential group nobody ranked: never cut.
const ESSENTIAL: u8 = 1;
/// Any other unranked group: cut before the essentials.
const FLEXIBLE: u8 = 2;

/// One spending group as the owner usually lives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HabitRow {
    /// The budget group's id, or `shadow:<group>` without a budget.
    pub id: String,
    pub name: String,
    /// The median of the earlier months with spending.
    pub typical: Decimal,
    /// What the plan gives it; zero without a budget.
    pub budget: Decimal,
    pub typical_visits: Option<Decimal>,
    /// A bare living cost.
    pub essential: bool,
}

/// The median of the earlier entries of `history` (all but the last, the
/// running month) that are above zero, once there are three.
pub fn typical(history: &[Decimal]) -> Option<Decimal> {
    let earlier = history.len().saturating_sub(1);
    let spent: Vec<Decimal> = history[..earlier]
        .iter()
        .copied()
        .filter(|v| *v > Decimal::ZERO)
        .collect();
    (spent.len() >= TYPICAL_MONTHS)
        .then(|| median(&spent))
        .flatten()
}

/// `currency`'s spending groups that have a typical month.
pub fn habit_rows(
    adjustments: &[Adjustments],
    currency: &Currency,
    essentials: &[String],
) -> Vec<HabitRow> {
    let essentials: Vec<String> = essentials.iter().map(|g| casefold(g)).collect();
    adjustments
        .iter()
        .filter(|a| a.currency == *currency)
        .flat_map(|a| &a.rows)
        .filter_map(|row| {
            Some(HabitRow {
                typical: typical(&row.row.history)?,
                id: row.row.id.clone(),
                name: row.row.name.clone(),
                budget: if row.budgeted {
                    row.row.planned
                } else {
                    Decimal::ZERO
                },
                typical_visits: row.habit.typical_visits,
                essential: essentials.contains(&casefold(&row.row.name)),
            })
        })
        .collect()
}

/// The plan's baselines with every month after the running one living as
/// usual: free money pays the typical month of every group instead of its
/// budget, and every group can be cut down from its typical month, an
/// unranked essential one not at all.
pub fn habit_baselines(
    plan: &[MonthBaseline],
    rows: &[HabitRow],
    plans: &[BudgetPlan],
) -> Vec<MonthBaseline> {
    let mut latest: Vec<&BudgetPlan> = plans.iter().collect();
    latest.sort_by(|a, b| b.month.cmp(&a.month));
    let above_budget: Decimal = rows.iter().map(|r| r.typical - r.budget).sum();
    let rooms: Vec<GroupRoom> = rows
        .iter()
        .map(|row| {
            let ranked = ranking(latest.iter().copied(), &row.name);
            let (importance, min) = match ranked.and_then(|g| Some((g.importance?, g.floor))) {
                Some((importance, floor)) => (importance, floor.unwrap_or_default()),
                None if row.essential => (ESSENTIAL, row.typical),
                None => (FLEXIBLE, Decimal::ZERO),
            };
            GroupRoom {
                id: row.id.clone(),
                name: row.name.clone(),
                importance,
                limit: row.typical,
                min: min.min(row.typical),
            }
        })
        .collect();
    plan.iter()
        .enumerate()
        .map(|(i, base)| match i {
            0 => base.clone(),
            _ => MonthBaseline {
                free: base.free - above_budget,
                groups: rooms.clone(),
                ..base.clone()
            },
        })
        .collect()
}

/// What the repeating costs that never end add to a month: bill changes and
/// recurring wishes, at their monthly equivalent.
pub fn ongoing_monthly(items: &[Planned]) -> Decimal {
    items
        .iter()
        .filter_map(|item| match item.per_month {
            Some((every, None, each)) => Some(match every {
                Repeat::Weekly => each * Decimal::from(52) / Decimal::from(12),
                Repeat::Biweekly => each * Decimal::from(26) / Decimal::from(12),
                Repeat::Monthly => each,
                Repeat::Yearly => each / Decimal::from(12),
            }),
            _ => None,
        })
        .sum()
}

/// Bills a month as they will stand: next month's fixed costs without the
/// card bills (cards pay purchases, they are not a living cost), plus the
/// planned changes that stay.
pub fn fixed_monthly(
    alloc: &CurrencyAllocation,
    card_bills: &[(String, Decimal)],
    items: &[Planned],
) -> Decimal {
    let cards: Decimal = card_bills.iter().map(|(_, amount)| *amount).sum();
    (alloc.next_fixed - cards + ongoing_monthly(items)).max(Decimal::ZERO)
}

/// What the habit scenario takes from each group after the running month,
/// the largest total first.
pub fn cuts(habit_months: &[WishMonth], rows: &[HabitRow]) -> Vec<HabitCut> {
    let mut order: Vec<&str> = Vec::new();
    let mut by_group: HashMap<&str, Vec<Decimal>> = HashMap::new();
    for month in habit_months.iter().skip(1) {
        let mut taken: HashMap<&str, Decimal> = HashMap::new();
        for cut in month.verdicts.iter().flat_map(|v| &v.cuts) {
            *taken.entry(cut.group_id.as_str()).or_default() += cut.from - cut.to;
        }
        for row in rows {
            if let Some(amount) = taken.get(row.id.as_str()).filter(|a| **a > Decimal::ZERO) {
                if !by_group.contains_key(row.id.as_str()) {
                    order.push(&row.id);
                }
                by_group.entry(&row.id).or_default().push(*amount);
            }
        }
    }
    let mut found: Vec<HabitCut> = order
        .into_iter()
        .filter_map(|id| {
            let row = rows.iter().find(|r| r.id == id)?;
            let amounts = by_group.get(id)?;
            Some(HabitCut {
                group_id: row.id.clone(),
                name: row.name.clone(),
                typical: row.typical,
                total: amounts.iter().sum(),
                months: amounts.len(),
                largest: amounts.iter().copied().max().unwrap_or_default(),
                typical_visits: row.typical_visits,
            })
        })
        .collect();
    found.sort_by(|a, b| (b.total, &a.name).cmp(&(a.total, &b.name)));
    found
}

/// The emergency fund: `months` of bills and essentials, filled by what the
/// habit months leave after the wishes, then by `steady` (a later month's
/// leftover once the wishes are done). `None` when switched off.
pub fn fund(
    months: u32,
    fixed: Decimal,
    rows: &[HabitRow],
    habit_months: &[WishMonth],
    steady: Decimal,
) -> Option<EmergencyFund> {
    if months == 0 {
        return None;
    }
    let essentials: Decimal = rows.iter().filter(|r| r.essential).map(|r| r.typical).sum();
    let target = (fixed + essentials) * Decimal::from(months);
    let balance = habit_months
        .first()
        .map_or(Decimal::ZERO, |m| m.savings_before);
    let mut left = Decimal::ZERO;
    let mut held = balance;
    let reached = if balance >= target {
        habit_months.first().map(|m| m.month.clone())
    } else {
        habit_months.iter().find_map(|m| {
            left += m.free_left.max(Decimal::ZERO);
            held = m.savings_left + left;
            (held >= target).then(|| m.month.clone())
        })
    };
    let eta = reached.or_else(|| {
        let last = parse_month(&habit_months.last()?.month)?;
        if steady <= Decimal::ZERO {
            return None;
        }
        let more = ((target - held) / steady).ceil();
        let more = i32::try_from(more.mantissa()).ok().filter(|m| *m <= 1200)?;
        Some(shift_month(last, more).to_string())
    });
    Some(EmergencyFund {
        months,
        fixed_monthly: fixed,
        essentials_monthly: essentials,
        target,
        balance,
        eta,
    })
}

#[cfg(test)]
mod tests {
    use finance_types::gateway::Repeat;
    use finance_types::report::Currency;
    use finance_types::report::adjustments::{AdjustmentRow, Adjustments, Habit};
    use finance_types::report::budget::BudgetRow;
    use finance_types::report::wishes::{Cut, Verdict, WishMonth};
    use jiff::civil::date;

    use super::*;
    use crate::wishes::baseline::tests::{alloc, d, plan, plan_group};
    use crate::wishes::ladder::{GroupRoom, MonthBaseline};
    use crate::wishes::schedule::Planned;

    fn habit(id: &str, name: &str, typical: &str, budget: &str, essential: bool) -> HabitRow {
        HabitRow {
            id: id.into(),
            name: name.into(),
            typical: d(typical),
            budget: d(budget),
            typical_visits: None,
            essential,
        }
    }

    fn rows() -> Vec<HabitRow> {
        vec![
            habit("food-out", "Food & drinks out", "570", "100", false),
            habit("groceries", "Groceries", "330", "300", true),
            habit("shadow:Shopping", "Shopping", "75", "0", false),
        ]
    }

    fn base(month: i8, free: &str) -> MonthBaseline {
        MonthBaseline {
            month: date(2026, month, 1),
            free: d(free),
            groups: vec![GroupRoom {
                id: "planned".into(),
                name: "Planned".into(),
                importance: 2,
                limit: d("100"),
                min: d("0"),
            }],
            cards: Vec::new(),
            savings: d("611"),
        }
    }

    #[test]
    fn later_months_cost_the_typical_month_and_month_zero_is_the_plans() {
        let plan_bases = [base(10, "100"), base(11, "900"), base(12, "900")];
        let habit = habit_baselines(&plan_bases, &rows(), &[]);
        assert_eq!(habit[0].free, d("100"));
        assert_eq!(habit[0].groups[0].id, "planned");
        // 900 − (470 + 30 + 75).
        assert_eq!((habit[1].free, habit[2].free), (d("325"), d("325")));
        let rooms: Vec<(&str, u8, Decimal, Decimal)> = habit[1]
            .groups
            .iter()
            .map(|g| (g.id.as_str(), g.importance, g.limit, g.min))
            .collect();
        assert_eq!(
            rooms,
            [
                ("food-out", 2, d("570"), d("0")),
                ("groceries", 1, d("330"), d("330")),
                ("shadow:Shopping", 2, d("75"), d("0")),
            ]
        );
    }

    #[test]
    fn the_owners_ranking_and_floor_win() {
        let plans = [
            plan(
                "2026-09-01",
                vec![plan_group("g", "Groceries", Some(3), Some("250"))],
            ),
            plan(
                "2026-10-01",
                vec![plan_group("f", "food & drinks out", Some(5), Some("200"))],
            ),
        ];
        let habit = habit_baselines(&[base(10, "0"), base(11, "0")], &rows(), &plans);
        let room = |id: &str| habit[1].groups.iter().find(|g| g.id == id).unwrap();
        assert_eq!(
            (room("food-out").importance, room("food-out").min),
            (5, d("200"))
        );
        assert_eq!(
            (room("groceries").importance, room("groceries").min),
            (3, d("250"))
        );
    }

    fn adjustment(
        id: &str,
        name: &str,
        planned: &str,
        history: &[i64],
        budgeted: bool,
    ) -> AdjustmentRow {
        AdjustmentRow {
            row: BudgetRow {
                id: id.into(),
                name: name.into(),
                planned: d(planned),
                spent: Decimal::ZERO,
                remaining: Decimal::ZERO,
                reserved: Decimal::ZERO,
                scheduled: Decimal::ZERO,
                categories: Vec::new(),
                history: history.iter().map(|n| Decimal::from(*n)).collect(),
                daily_spending: Vec::new(),
                elapsed_days: 10,
                expected: Vec::new(),
            },
            budgeted,
            group: (!budgeted).then(|| name.to_owned()),
            habit: Habit {
                typical_visits: Some(d("30")),
                ..Habit::default()
            },
            stores: Vec::new(),
        }
    }

    #[test]
    fn habit_rows_need_a_typical_month() {
        let report = Adjustments {
            currency: Currency::eur(),
            month: "2026-11-01".into(),
            rows: vec![
                adjustment(
                    "food-out",
                    "Food & drinks out",
                    "100",
                    &[627, 0, 363, 572, 90],
                    true,
                ),
                adjustment("groceries", "groceries", "300", &[340, 320, 330, 10], true),
                adjustment("shadow:Tech", "Tech", "0", &[0, 200, 60, 5], false),
            ],
            unsorted: Decimal::ZERO,
        };
        let other = Adjustments {
            currency: Currency::new("USD"),
            ..report.clone()
        };
        let rows = habit_rows(
            &[other, report],
            &Currency::eur(),
            &["Groceries".to_owned()],
        );
        // Tech has two earlier months with spending: not known well enough.
        let seen: Vec<(&str, Decimal, Decimal, bool)> = rows
            .iter()
            .map(|r| (r.id.as_str(), r.typical, r.budget, r.essential))
            .collect();
        assert_eq!(
            seen,
            [
                ("food-out", d("572"), d("100"), false),
                ("groceries", d("330"), d("300"), true),
            ]
        );
        assert_eq!(rows[0].typical_visits, Some(d("30")));
    }

    fn repeating(every: Repeat, each: &str, ends: Option<jiff::civil::Date>) -> Planned {
        Planned {
            wish_id: "w".into(),
            label: "w".into(),
            due: date(2027, 1, 1),
            remaining: d(each),
            on_card: false,
            created_at: String::new(),
            per_month: Some((every, ends, d(each))),
            saved_this_month: false,
            must: false,
        }
    }

    #[test]
    fn fixed_costs_follow_the_planned_changes_that_stay() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.next_fixed = d("1100");
        let one_off = Planned {
            per_month: None,
            ..repeating(Repeat::Monthly, "3000", None)
        };
        let items = [
            repeating(Repeat::Monthly, "250", None),
            repeating(Repeat::Weekly, "10", None),
            repeating(Repeat::Yearly, "120", None),
            repeating(Repeat::Monthly, "99", Some(date(2027, 6, 1))),
            one_off,
        ];
        let fixed = fixed_monthly(&a, &[("card".to_owned(), d("50"))], &items);
        // 1100 − 50 + 250 + 10 × 52 / 12 + 120 / 12.
        assert_eq!(fixed.round_dp(2), d("1353.33"));
        assert_eq!(ongoing_monthly(&items).round_dp(2), d("303.33"));
    }

    fn cut(id: &str, from: &str, to: &str) -> Cut {
        Cut {
            group_id: id.into(),
            name: id.into(),
            from: d(from),
            to: d(to),
        }
    }

    fn month(month: &str, free_left: &str, savings: (&str, &str), cuts: Vec<Cut>) -> WishMonth {
        WishMonth {
            month: month.into(),
            free_before: Decimal::ZERO,
            free_left: d(free_left),
            savings_before: d(savings.0),
            savings_left: d(savings.1),
            card_charge_in: Decimal::ZERO,
            card_used: Decimal::ZERO,
            cards: Vec::new(),
            verdicts: vec![Verdict {
                wish_id: Some("w".into()),
                label: "w".into(),
                amount: Decimal::ZERO,
                from_free: Decimal::ZERO,
                from_savings: Decimal::ZERO,
                cuts,
                on_card: Decimal::ZERO,
                short: Decimal::ZERO,
                set_aside: false,
                next_short: Decimal::ZERO,
                next_cuts: Vec::new(),
                must: false,
                skipped: false,
            }],
        }
    }

    #[test]
    fn cuts_add_up_per_group_the_largest_first() {
        let months = [
            month(
                "2026-11-01",
                "0",
                ("0", "0"),
                vec![cut("planned", "100", "50")],
            ),
            month(
                "2026-12-01",
                "0",
                ("0", "0"),
                vec![cut("food-out", "570", "420")],
            ),
            month(
                "2027-01-01",
                "0",
                ("0", "0"),
                vec![
                    cut("food-out", "570", "540"),
                    cut("food-out", "540", "500"),
                    cut("shadow:Shopping", "75", "0"),
                ],
            ),
        ];
        let found = cuts(&months, &rows());
        let seen: Vec<(&str, Decimal, usize, Decimal)> = found
            .iter()
            .map(|c| (c.group_id.as_str(), c.total, c.months, c.largest))
            .collect();
        // Month 0 is the plan's own judging, not a habit to change.
        assert_eq!(
            seen,
            [
                ("food-out", d("220"), 2, d("150")),
                ("shadow:Shopping", d("75"), 1, d("75")),
            ]
        );
        assert_eq!(
            (found[0].name.as_str(), found[0].typical),
            ("Food & drinks out", d("570"))
        );
    }

    #[test]
    fn the_fund_is_months_of_bare_living() {
        let months = [month("2026-11-01", "0", ("611", "611"), vec![])];
        let fund = fund(6, d("1050"), &rows(), &months, d("500")).unwrap();
        assert_eq!(
            (
                fund.fixed_monthly,
                fund.essentials_monthly,
                fund.target,
                fund.balance
            ),
            (d("1050"), d("330"), d("8280"), d("611"))
        );
        assert!(super::fund(0, d("1050"), &rows(), &months, d("500")).is_none());
    }

    #[test]
    fn the_fund_fills_from_what_the_wishes_leave() {
        let target_rows = [habit("groceries", "Groceries", "100", "100", true)];
        // Target 6 × (100 + 100) = 1200.
        let fill = |months: &[WishMonth], steady: &str| {
            fund(6, d("100"), &target_rows, months, d(steady))
                .unwrap()
                .eta
        };
        // Inside the horizon: 500 + 300 + 400 reaches 1200 in December.
        let inside = [
            month("2026-11-01", "300", ("500", "500"), vec![]),
            month("2026-12-01", "400", ("500", "500"), vec![]),
            month("2027-01-01", "400", ("500", "500"), vec![]),
        ];
        assert_eq!(fill(&inside, "400").as_deref(), Some("2026-12-01"));
        // After it: 100 held, 1100 to go at 500 a month is three more months.
        let after = [
            month("2026-11-01", "0", ("500", "100"), vec![]),
            month("2026-12-01", "0", ("100", "100"), vec![]),
        ];
        assert_eq!(fill(&after, "500").as_deref(), Some("2027-03-01"));
        // Nothing left over: no date.
        assert_eq!(fill(&after, "0"), None);
        assert_eq!(fill(&after, "-50"), None);
        // Already full.
        let full = [month("2026-11-01", "0", ("1500", "1500"), vec![])];
        assert_eq!(fill(&full, "0").as_deref(), Some("2026-11-01"));
        assert_eq!(fill(&[], "500"), None);
    }
}
