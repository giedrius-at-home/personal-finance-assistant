//! Spending → Plan & patterns, "Adjustments": every spending group against
//! the owner's own typical month, what drives a group that runs above it,
//! one budget action, and how often the owner goes where.

use finance_types::gateway::{BudgetGroup, BudgetPlan};
use finance_types::report::adjustments::{AdjustmentRow, Habit};
use finance_types::report::budget::BudgetRow;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::pages::overview::visuals::{Pace, month_end};

/// Earlier months with spending a typical month needs.
const TYPICAL_MONTHS: usize = 3;
/// Days of the month and purchases before a pace means anything.
const EARLY_DAYS: usize = 7;
const EARLY_VISITS: usize = 2;
/// The least a finding must be worth, in the currency's units.
const WORTH: Decimal = Decimal::from_parts(25, 0, 0, false, 0);
/// Visits lately that put a place in the habits table.
const SHOWN_VISITS: usize = 4;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn ratio(percent: i64) -> Decimal {
    Decimal::new(percent, 2)
}

fn median(mut values: Vec<Decimal>) -> Option<Decimal> {
    values.sort();
    let n = values.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(values[n / 2]),
        _ => Some((values[n / 2 - 1] + values[n / 2]) / Decimal::TWO),
    }
}

/// The owner's typical month for a row: the median of the earlier months
/// that had spending, once there are three.
pub fn typical(row: &BudgetRow) -> Option<Decimal> {
    let earlier = row.history.len().saturating_sub(1);
    let spent: Vec<Decimal> = row.history[..earlier]
        .iter()
        .copied()
        .filter(|v| *v > Decimal::ZERO)
        .collect();
    (spent.len() >= TYPICAL_MONTHS)
        .then(|| median(spent))
        .flatten()
}

/// Too little of the month, or too few purchases, to say where it is heading.
pub fn too_early(row: &AdjustmentRow) -> bool {
    row.row.elapsed_days < EARLY_DAYS || row.habit.visits < EARLY_VISITS
}

/// Where the month is heading at its daily pace (Overview's projection).
pub fn heading(row: &AdjustmentRow, days: usize) -> Option<Decimal> {
    if too_early(row) {
        return None;
    }
    // The division leaves a hair of error; a finding must not hang on it.
    month_end(&row.row, Pace::Daily, days).map(|end| end.round_dp(2))
}

/// How far above a typical month the row is heading, when that is at least
/// a quarter and worth mentioning.
pub fn above_norm(row: &AdjustmentRow, days: usize) -> Option<Decimal> {
    let typical = typical(&row.row)?;
    let excess = heading(row, days)? - typical;
    (excess >= typical * ratio(25) && excess >= WORTH).then_some(excess)
}

/// A budget that does not fit the owner's habit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetFit {
    /// A fifth or more below a typical month.
    Low,
    /// A fifth or more above it, and worth mentioning.
    High,
}

pub fn budget_fit(row: &AdjustmentRow) -> Option<BudgetFit> {
    if !row.budgeted {
        return None;
    }
    let typical = typical(&row.row)?;
    let budget = row.row.planned;
    if budget <= typical * ratio(80) {
        Some(BudgetFit::Low)
    } else if budget >= typical * ratio(120) && budget - typical >= WORTH {
        Some(BudgetFit::High)
    } else {
        None
    }
}

/// `amount` to the nearest ten, halves up.
pub fn round_to_ten(amount: Decimal) -> Decimal {
    (amount / Decimal::TEN).round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        * Decimal::TEN
}

/// The one change to the budget plan a row suggests, with the new amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Set(Decimal),
    Lower(Decimal),
    Add(Decimal),
}

impl Action {
    pub fn amount(&self) -> Decimal {
        match self {
            Self::Set(amount) | Self::Lower(amount) | Self::Add(amount) => *amount,
        }
    }
}

pub fn action(row: &AdjustmentRow, days: usize) -> Option<Action> {
    let amount = round_to_ten(typical(&row.row)?);
    match budget_fit(row) {
        Some(BudgetFit::Low) => Some(Action::Set(amount)),
        Some(BudgetFit::High) => Some(Action::Lower(amount)),
        None if !row.budgeted && amount > Decimal::ZERO => {
            above_norm(row, days).map(|_| Action::Add(amount))
        }
        None => None,
    }
}

/// `Health & personal care` → `health-personal-care`.
pub fn slug(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// `plan` (the saved one, never the shadow plan) with the action applied.
pub fn apply(plan: &BudgetPlan, row: &AdjustmentRow, action: &Action) -> BudgetPlan {
    let mut plan = plan.clone();
    match action {
        Action::Set(amount) | Action::Lower(amount) => {
            for group in plan.groups.iter_mut().filter(|g| g.id == row.row.id) {
                group.amount = *amount;
            }
        }
        Action::Add(amount) => {
            let name = row.group.clone().unwrap_or_else(|| row.row.name.clone());
            let base = slug(&name);
            let taken = |id: &str| plan.groups.iter().any(|g| g.id == id);
            let id = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base}-{n}")))
                .find(|id| !taken(id))
                .unwrap_or(base);
            plan.groups.push(BudgetGroup {
                id,
                source_groups: vec![name.clone()],
                name,
                amount: *amount,
                categories: Vec::new(),
                transaction_ids: Vec::new(),
                account_id: None,
                importance: None,
                floor: None,
            });
        }
    }
    plan
}

/// A store behind a row's excess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Driver {
    pub category: String,
    pub visits: usize,
    pub spent: Decimal,
    /// The usual `(visits, spending)` of a month; `None` for a new store.
    pub usual: Option<(Decimal, Decimal)>,
}

/// The up to three stores furthest above their usual spending for this
/// point of the month; a store without history counts all it spent.
pub fn drivers(row: &AdjustmentRow, days: usize) -> Vec<Driver> {
    let days = days.max(row.row.elapsed_days).max(1);
    let progress = Decimal::from(row.row.elapsed_days) / Decimal::from(days);
    let mut ranked: Vec<(Decimal, Driver)> = row
        .stores
        .iter()
        .map(|store| {
            let habit = &store.habit;
            let usual = habit.typical_visits.zip(habit.typical_spent);
            let expected = usual.map_or(Decimal::ZERO, |(_, spent)| spent * progress);
            (
                habit.spent - expected,
                Driver {
                    category: store.category.clone(),
                    visits: habit.visits,
                    spent: habit.spent,
                    usual,
                },
            )
        })
        .filter(|(excess, _)| *excess > Decimal::ZERO)
        .collect();
    ranked.sort_by(|a, b| (b.0, &a.1.category).cmp(&(a.0, &b.1.category)));
    ranked
        .into_iter()
        .take(3)
        .map(|(_, driver)| driver)
        .collect()
}

/// The one or two weekdays holding at least a quarter of the visits each.
pub fn mostly(weekday_visits: &[usize; 7]) -> String {
    let total: usize = weekday_visits.iter().sum();
    let mut busy: Vec<usize> = (0..7)
        .filter(|day| total > 0 && weekday_visits[*day] * 4 >= total)
        .collect();
    busy.sort_by_key(|day| std::cmp::Reverse(weekday_visits[*day]));
    busy.truncate(2);
    busy.sort_unstable();
    if busy.is_empty() {
        return "any day".to_owned();
    }
    busy.into_iter()
        .map(|day| WEEKDAYS[day])
        .collect::<Vec<_>>()
        .join(", ")
}

/// At least two visits above the usual week.
pub fn busy_week(habit: &Habit) -> bool {
    Decimal::from(habit.visits_this_week) >= habit.typical_week_visits + Decimal::TWO
}

/// Visited often enough lately to list.
pub fn shown(habit: &Habit) -> bool {
    habit.recent_visits >= SHOWN_VISITS
}

#[cfg(test)]
mod tests {
    use finance_types::gateway::{BudgetGroup, BudgetPlan};
    use finance_types::report::Currency;
    use finance_types::report::adjustments::{AdjustmentRow, Habit, StoreHabit};
    use finance_types::report::budget::BudgetRow;
    use rust_decimal::Decimal;

    use super::*;

    const DAYS: usize = 30;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    /// A row named "Shopping" whose earlier months are `earlier`.
    fn row(
        planned: &str,
        spent: &str,
        earlier: &[i64],
        elapsed: usize,
        visits: usize,
        budgeted: bool,
    ) -> AdjustmentRow {
        let spent = dec(spent);
        let mut history: Vec<Decimal> = earlier.iter().map(|n| Decimal::from(*n)).collect();
        history.push(spent);
        AdjustmentRow {
            row: BudgetRow {
                id: if budgeted {
                    "food-out"
                } else {
                    "shadow:Shopping"
                }
                .into(),
                name: if budgeted {
                    "Food & drinks out"
                } else {
                    "Shopping"
                }
                .into(),
                planned: dec(planned),
                spent,
                remaining: dec(planned) - spent,
                reserved: Decimal::ZERO,
                scheduled: Decimal::ZERO,
                categories: Vec::new(),
                history,
                daily_spending: Vec::new(),
                elapsed_days: elapsed,
                expected: Vec::new(),
            },
            budgeted,
            group: (!budgeted).then(|| "Shopping".into()),
            habit: Habit {
                visits,
                spent,
                ..Habit::default()
            },
            stores: Vec::new(),
        }
    }

    fn store(category: &str, visits: usize, spent: i64, typical_spent: Option<i64>) -> StoreHabit {
        StoreHabit {
            category: category.into(),
            habit: Habit {
                visits,
                spent: Decimal::from(spent),
                typical_visits: typical_spent.map(|_| Decimal::TWO),
                typical_spent: typical_spent.map(Decimal::from),
                ..Habit::default()
            },
        }
    }

    fn plan() -> BudgetPlan {
        let group = |id: &str, amount: i64| BudgetGroup {
            id: id.into(),
            name: id.into(),
            amount: Decimal::from(amount),
            source_groups: vec![id.into()],
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance: None,
            floor: None,
        };
        BudgetPlan {
            month: "2026-11-01".into(),
            currency: Currency::eur(),
            revision: 2,
            groups: vec![group("groceries", 300), group("food-out", 100)],
        }
    }

    #[test]
    fn typical_needs_three_earlier_months_with_spending() {
        assert_eq!(
            typical(&row("0", "90", &[0, 400, 430, 0, 460], 15, 2, false).row),
            Some(dec("430"))
        );
        assert_eq!(
            typical(&row("0", "90", &[0, 0, 400, 430], 15, 2, false).row),
            None
        );
    }

    #[test]
    fn above_norm_at_the_boundary() {
        let earlier = [400, 400, 400];
        assert_eq!(
            above_norm(&row("0", "250", &earlier, 15, 5, false), DAYS),
            Some(dec("100"))
        );
        assert_eq!(
            above_norm(&row("0", "249.5", &earlier, 15, 5, false), DAYS),
            None
        );
        // A quarter above, but only 20 in money.
        assert_eq!(
            above_norm(&row("0", "40", &[60, 60, 60], 15, 5, false), DAYS),
            None
        );
    }

    #[test]
    fn too_early_gives_no_heading_or_norm_finding_but_keeps_budget_fit() {
        let earlier = [430, 430, 430];
        for (elapsed, visits) in [(6, 9), (20, 1)] {
            let early = row("100", "600", &earlier, elapsed, visits, true);
            assert!(too_early(&early));
            assert_eq!(heading(&early, DAYS), None);
            assert_eq!(above_norm(&early, DAYS), None);
            assert_eq!(budget_fit(&early), Some(BudgetFit::Low));
        }
        assert!(!too_early(&row("100", "600", &earlier, 7, 2, true)));
    }

    #[test]
    fn budget_fit_boundaries() {
        let fit = |planned: &str, earlier: &[i64], budgeted| {
            budget_fit(&row(planned, "0", earlier, 15, 2, budgeted))
        };
        let usual = [430, 430, 430];
        assert_eq!(fit("344", &usual, true), Some(BudgetFit::Low));
        assert_eq!(fit("345", &usual, true), None);
        assert_eq!(fit("516", &usual, true), Some(BudgetFit::High));
        assert_eq!(fit("515", &usual, true), None);
        // A fifth above, but only 20 in money.
        assert_eq!(fit("120", &[100, 100, 100], true), None);
        assert_eq!(fit("0", &usual, false), None);
        assert_eq!(fit("100", &[430, 430], true), None);
    }

    #[test]
    fn a_row_can_be_above_norm_and_under_budgeted() {
        let hot = row("100", "290", &[430, 430, 430], 15, 9, true);
        assert_eq!(heading(&hot, DAYS), Some(dec("580")));
        assert_eq!(above_norm(&hot, DAYS), Some(dec("150")));
        assert_eq!(action(&hot, DAYS), Some(Action::Set(dec("430"))));
    }

    #[test]
    fn actions_follow_the_findings() {
        let spare = row("600", "100", &[430, 430, 430], 15, 4, true);
        assert_eq!(action(&spare, DAYS), Some(Action::Lower(dec("430"))));
        let drifting = row("0", "200", &[150, 148, 152], 15, 4, false);
        assert_eq!(action(&drifting, DAYS), Some(Action::Add(dec("150"))));
        let steady = row("0", "75", &[150, 150, 150], 15, 4, false);
        assert_eq!(action(&steady, DAYS), None);
        let fitting = row("430", "290", &[430, 430, 430], 15, 9, true);
        assert_eq!(action(&fitting, DAYS), None);
    }

    #[test]
    fn a_refund_heavy_row_has_no_finding() {
        let mut refunded = row("0", "-20", &[150, 150, 150], 15, 2, false);
        refunded.stores = vec![store("Zara", 0, -20, Some(40))];
        assert_eq!(heading(&refunded, DAYS), Some(Decimal::ZERO));
        assert_eq!(above_norm(&refunded, DAYS), None);
        assert_eq!(action(&refunded, DAYS), None);
        assert!(drivers(&refunded, DAYS).is_empty());
    }

    #[test]
    fn round_to_ten_rounds_half_up() {
        for (amount, rounded) in [("425", "430"), ("424.99", "420"), ("4", "0"), ("5", "10")] {
            assert_eq!(round_to_ten(dec(amount)), dec(rounded));
        }
    }

    #[test]
    fn apply_sets_and_adds_without_touching_other_groups() {
        let plan = plan();
        let food = row("100", "290", &[430, 430, 430], 15, 9, true);
        let set = apply(&plan, &food, &Action::Set(dec("430")));
        assert_eq!(set.groups[1].amount, dec("430"));
        assert_eq!((&set.groups[0], set.revision), (&plan.groups[0], 2));
        assert_eq!(set.groups.len(), 2);

        let shopping = row("0", "200", &[150, 150, 150], 15, 4, false);
        let added = apply(&plan, &shopping, &Action::Add(dec("150")));
        assert_eq!(added.groups[..2], plan.groups[..]);
        let new = &added.groups[2];
        assert_eq!(
            (new.id.as_str(), new.name.as_str()),
            ("shopping", "Shopping")
        );
        assert_eq!(
            (new.amount, &new.source_groups),
            (dec("150"), &vec!["Shopping".to_owned()])
        );
    }

    #[test]
    fn an_added_group_avoids_a_taken_id() {
        let mut plan = plan();
        plan.groups[0].id = "shopping".into();
        plan.groups[1].id = "shopping-2".into();
        let shopping = row("0", "200", &[150, 150, 150], 15, 4, false);
        let added = apply(&plan, &shopping, &Action::Add(dec("150")));
        assert_eq!(added.groups[2].id, "shopping-3");
        assert_eq!(slug("Health & personal care"), "health-personal-care");
    }

    #[test]
    fn drivers_rank_by_pace_adjusted_excess_and_include_new_stores() {
        let mut hot = row("0", "170", &[150, 150, 150], 15, 6, false);
        hot.stores = vec![
            store("B", 2, 40, Some(100)),
            store("C", 3, 30, None),
            store("A", 4, 100, Some(60)),
            store("D", 1, 12, None),
            store("E", 1, 11, None),
        ];
        let found = drivers(&hot, DAYS);
        let names: Vec<&str> = found.iter().map(|d| d.category.as_str()).collect();
        assert_eq!(names, ["A", "C", "D"]);
        assert_eq!(found[0].usual, Some((Decimal::TWO, dec("60"))));
        assert_eq!(
            (found[1].visits, found[1].spent, found[1].usual),
            (3, dec("30"), None)
        );
    }

    #[test]
    fn mostly_names_up_to_two_weekdays() {
        assert_eq!(mostly(&[0, 0, 0, 0, 5, 5, 1]), "Fri, Sat");
        assert_eq!(mostly(&[1, 0, 9, 4, 0, 5, 1]), "Wed, Sat");
        assert_eq!(mostly(&[2, 2, 2, 2, 2, 2, 2]), "any day");
        assert_eq!(mostly(&[0; 7]), "any day");
    }

    #[test]
    fn busy_week_and_shown_thresholds() {
        let habit = |week, usual: &str, recent| Habit {
            visits_this_week: week,
            typical_week_visits: dec(usual),
            recent_visits: recent,
            ..Habit::default()
        };
        assert!(busy_week(&habit(4, "2", 0)));
        assert!(!busy_week(&habit(3, "2", 0)));
        assert!(!busy_week(&habit(3, "1.5", 0)));
        assert!(shown(&habit(0, "0", 4)));
        assert!(!shown(&habit(0, "0", 3)));
    }
}
