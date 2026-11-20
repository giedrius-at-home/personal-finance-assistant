//! The wishes and the emergency fund against current habits: the ladder's
//! baselines rewritten so every spending group costs its typical month, and
//! what that judging says to cut. Pure arithmetic, nothing is saved.

use finance_types::gateway::{BudgetPlan, Repeat};
use finance_types::report::Currency;
use finance_types::report::adjustments::Adjustments;
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::wishes::{EmergencyFund, HabitCut, WishMonth};
use rust_decimal::Decimal;

use super::baseline::{parse_month, ranking};
use super::ladder::MonthBaseline;
use super::schedule::Planned;
use crate::period::shift_month;
use crate::text::casefold;
use crate::trends::median;

/// Earlier months a usual month needs.
const USUAL_MONTHS: usize = 3;
/// One spending group as the owner usually lives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HabitRow {
    /// The budget group's id, or `shadow:<group>` without a budget.
    pub id: String,
    pub name: String,
    /// What a month usually costs ([`usual`]).
    pub typical: Decimal,
    /// What the plan gives it; zero without a budget.
    pub budget: Decimal,
    pub typical_visits: Option<Decimal>,
    /// What a visit usually costs.
    pub ticket: Option<Decimal>,
    /// A bare living cost.
    pub essential: bool,
}

/// What a month usually costs: the median of the earlier entries of
/// `history` (all but the last, the running month), months without spending
/// included, counted from the group's first month with spending, once
/// there are three. Lessons paid every other month count for
/// what they cost a month; a one-off counts for nothing. `None` when that
/// is zero.
pub fn usual(history: &[Decimal]) -> Option<Decimal> {
    let earlier = &history[..history.len().saturating_sub(1)];
    // Months before the group's first spending are not part of its record.
    let first = earlier.iter().position(|v| *v > Decimal::ZERO)?;
    let earlier = &earlier[first..];
    if earlier.len() < USUAL_MONTHS {
        return None;
    }
    median(earlier).filter(|m| *m > Decimal::ZERO)
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
                typical: usual(&row.row.history)?,
                id: row.row.id.clone(),
                name: row.row.name.clone(),
                budget: if row.budgeted {
                    row.row.planned
                } else {
                    Decimal::ZERO
                },
                typical_visits: row.habit.typical_visits,
                ticket: row
                    .habit
                    .typical_spent
                    .zip(row.habit.typical_visits)
                    .filter(|(_, visits)| *visits > Decimal::ZERO)
                    .map(|(spent, visits)| (spent / visits).round_dp(2)),
                essential: essentials.contains(&casefold(&row.row.name)),
            })
        })
        .collect()
}

/// `rows` with the budgets the later months really run on: next month's
/// plan, which is what the allocation's free money already paid for. A row
/// without a budget there has none. Without next-month rows the running
/// month's budgets stand.
pub fn with_next_budgets(rows: &[HabitRow], alloc: &CurrencyAllocation) -> Vec<HabitRow> {
    if alloc.next_group_rows.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .map(|row| HabitRow {
            budget: alloc
                .next_group_rows
                .iter()
                .find(|next| casefold(&next.group) == casefold(&row.name))
                .map_or(Decimal::ZERO, |next| next.limit),
            ..row.clone()
        })
        .collect()
}

/// The plan's baselines with every month after the running one living as
/// usual: free money pays what every group usually costs instead of its
/// budget, and nothing is cut, since the owner changes nothing.
pub fn habit_baselines(plan: &[MonthBaseline], rows: &[HabitRow]) -> Vec<MonthBaseline> {
    let above_budget: Decimal = rows.iter().map(|r| r.typical - r.budget).sum();
    plan.iter()
        .enumerate()
        .map(|(i, base)| match i {
            0 => base.clone(),
            _ => MonthBaseline {
                free: base.free - above_budget,
                groups: Vec::new(),
                ..base.clone()
            },
        })
        .collect()
}

fn monthly(every: Repeat, each: Decimal) -> Decimal {
    match every {
        Repeat::Weekly => each * Decimal::from(52) / Decimal::from(12),
        Repeat::Biweekly => each * Decimal::from(26) / Decimal::from(12),
        Repeat::Monthly => each,
        Repeat::Yearly => each / Decimal::from(12),
    }
}

/// What the repeating costs that never end add to a month: bill changes and
/// recurring wishes, at their monthly equivalent.
pub fn ongoing_monthly(items: &[Planned]) -> Decimal {
    items
        .iter()
        .filter_map(|item| match item.per_month {
            Some((every, None, each)) => Some(monthly(every, each)),
            _ => None,
        })
        .sum()
}

/// Every planned repeating cost a month, those with an end included: what
/// the coming months must carry while they run.
pub fn repeating_monthly(items: &[Planned]) -> Decimal {
    items
        .iter()
        .filter_map(|item| item.per_month.map(|(every, _, each)| monthly(every, each)))
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

/// How a monthly `gap` could be closed: every group gives in proportion to
/// its room, and what even that leaves open. A group's room is its usual
/// month down to the floor the owner set in a plan; an essential group the
/// owner never ranked has none. Any split closes the gap; this one asks the
/// same share of everything that can give.
pub fn advice(gap: Decimal, rows: &[HabitRow], plans: &[BudgetPlan]) -> (Vec<HabitCut>, Decimal) {
    if gap <= Decimal::ZERO {
        return (Vec::new(), Decimal::ZERO);
    }
    let mut latest: Vec<&BudgetPlan> = plans.iter().collect();
    latest.sort_by(|a, b| b.month.cmp(&a.month));
    let rooms: Vec<Decimal> = rows
        .iter()
        .map(|row| {
            let ranked = ranking(latest.iter().copied(), &row.name);
            match ranked {
                Some(group) => row.typical - group.floor.unwrap_or_default(),
                None if row.essential => Decimal::ZERO,
                None => row.typical,
            }
            .max(Decimal::ZERO)
        })
        .collect();
    let room: Decimal = rooms.iter().sum();
    let closed = gap.min(room);
    let mut cuts: Vec<HabitCut> = rows
        .iter()
        .zip(&rooms)
        .filter(|(_, r)| **r > Decimal::ZERO)
        .map(|(row, r)| HabitCut {
            group_id: row.id.clone(),
            name: row.name.clone(),
            typical: row.typical.round_dp(2),
            monthly: (closed * *r / room).round_dp(2),
            typical_visits: row.typical_visits,
            ticket: row.ticket,
        })
        .collect();
    cuts.sort_by(|a, b| (b.monthly, &a.name).cmp(&(a.monthly, &b.name)));
    (cuts, (gap - room).max(Decimal::ZERO).round_dp(2))
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
    let (fixed, essentials) = (fixed.round_dp(2), essentials.round_dp(2));
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
        let end = habit_months.last()?;
        let last = parse_month(&end.month)?;
        if steady <= Decimal::ZERO {
            return None;
        }
        // What the wishes left on the cards is charged after the judged
        // months and paid before anything reaches the fund.
        let owed: Decimal = end
            .cards
            .iter()
            .map(|c| (c.used - c.regular).max(Decimal::ZERO))
            .sum();
        let more = ((target - held + owed) / steady).ceil();
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
    use finance_types::report::wishes::{CardUsed, WishMonth};
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
            ticket: None,
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
        let habit = habit_baselines(&plan_bases, &rows());
        assert_eq!(habit[0].free, d("100"));
        assert_eq!(habit[0].groups[0].id, "planned");
        // 900 − (470 + 30 + 75), and nothing is cut: the owner changes nothing.
        assert_eq!((habit[1].free, habit[2].free), (d("325"), d("325")));
        assert!(habit[1].groups.is_empty() && habit[2].groups.is_empty());
    }

    #[test]
    fn a_gap_is_shared_by_the_flexible_groups() {
        let seen = |gap: &str, plans: &[BudgetPlan]| {
            let (cuts, uncovered) = advice(d(gap), &rows(), plans);
            let cuts: Vec<(String, Decimal)> =
                cuts.into_iter().map(|c| (c.group_id, c.monthly)).collect();
            (cuts, uncovered)
        };
        let named = |pairs: &[(&str, &str)]| -> Vec<(String, Decimal)> {
            pairs
                .iter()
                .map(|(id, v)| ((*id).to_owned(), d(v)))
                .collect()
        };
        // 570 : 75; groceries are essential.
        assert_eq!(
            seen("300", &[]),
            (
                named(&[("food-out", "265.12"), ("shadow:Shopping", "34.88")]),
                d("0")
            )
        );
        // More than everything flexible: the rest stays open.
        assert_eq!(
            seen("1000", &[]),
            (
                named(&[("food-out", "570"), ("shadow:Shopping", "75")]),
                d("355")
            )
        );
        assert_eq!(seen("0", &[]), (Vec::new(), d("0")));
        // The owner's floors hold, and a ranked essential group may give.
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
        // Rooms 370, 80 and 75.
        assert_eq!(
            seen("525", &plans),
            (
                named(&[
                    ("food-out", "370"),
                    ("groceries", "80"),
                    ("shadow:Shopping", "75")
                ]),
                d("0")
            )
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
                to_events: Vec::new(),
            },
            budgeted,
            group: (!budgeted).then(|| name.to_owned()),
            habit: Habit {
                typical_visits: Some(d("30")),
                typical_spent: Some(d("360")),
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
                adjustment(
                    "shadow:Education",
                    "Education",
                    "0",
                    &[1712, 0, 0, 450, 1110, 0],
                    false,
                ),
                adjustment("shadow:Lumpy", "Lumpy", "0", &[0, 0, 900, 0, 0, 1], false),
                adjustment("shadow:New", "New", "0", &[40, 40, 5], false),
                adjustment(
                    "shadow:Recent",
                    "Recent",
                    "0",
                    &[0, 0, 300, 320, 310, 9],
                    false,
                ),
                adjustment("shadow:Once", "Once", "0", &[0, 900, 0, 0, 0, 9], false),
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
        // A usual month counts the months without spending too: lessons
        // paid in three months of five cost 450 a month, a one-off nothing.
        // Two earlier months are too few to know.
        let seen: Vec<(&str, Decimal, Decimal, bool)> = rows
            .iter()
            .map(|r| (r.id.as_str(), r.typical, r.budget, r.essential))
            .collect();
        assert_eq!(
            seen,
            [
                ("food-out", d("467.5"), d("100"), false),
                ("groceries", d("330"), d("300"), true),
                ("shadow:Education", d("450"), d("0"), false),
                // Three months of record: the months before it do not count.
                ("shadow:Recent", d("310"), d("0"), false),
            ]
        );
        assert_eq!(rows[0].typical_visits, Some(d("30")));
        assert_eq!(rows[0].ticket, Some(d("12")));
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
        // While it runs, the ending one must be carried too.
        assert_eq!(repeating_monthly(&items).round_dp(2), d("402.33"));
    }

    fn month(month: &str, free_left: &str, savings: (&str, &str)) -> WishMonth {
        WishMonth {
            month: month.into(),
            free_before: Decimal::ZERO,
            free_left: d(free_left),
            savings_before: d(savings.0),
            savings_left: d(savings.1),
            card_charge_in: Decimal::ZERO,
            card_used: Decimal::ZERO,
            cards: Vec::new(),
            verdicts: Vec::new(),
        }
    }

    #[test]
    fn the_fund_is_months_of_bare_living() {
        let months = [month("2026-11-01", "0", ("611", "611"))];
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
            month("2026-11-01", "300", ("500", "500")),
            month("2026-12-01", "400", ("500", "500")),
            month("2027-01-01", "400", ("500", "500")),
        ];
        assert_eq!(fill(&inside, "400").as_deref(), Some("2026-12-01"));
        // After it: 100 held, 1100 to go at 500 a month is three more months.
        let after = [
            month("2026-11-01", "0", ("500", "100")),
            month("2026-12-01", "0", ("100", "100")),
        ];
        assert_eq!(fill(&after, "500").as_deref(), Some("2027-03-01"));
        // 900 still on the card when the judged months end is paid first.
        let mut owing = after.clone();
        owing[1].cards = vec![CardUsed {
            card_id: "c".into(),
            name: "Card".into(),
            used: d("950"),
            regular: d("50"),
        }];
        assert_eq!(fill(&owing, "500").as_deref(), Some("2027-04-01"));
        // Nothing left over: no date.
        assert_eq!(fill(&after, "0"), None);
        assert_eq!(fill(&after, "-50"), None);
        // Already full.
        let full = [month("2026-11-01", "0", ("1500", "1500"))];
        assert_eq!(fill(&full, "0").as_deref(), Some("2026-11-01"));
        assert_eq!(fill(&[], "500"), None);
    }
}
