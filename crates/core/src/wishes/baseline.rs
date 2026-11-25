//! Each month's starting position, read from the monthly allocation.
//!
//! Pure arithmetic, no I/O. Month 0 is the allocation's current month, month 1
//! the funded month, and later months repeat month 1 with nothing spent yet.
//! Month 0's card usage is what the owner typed for the statement ending on
//! the 25th of month 0; each later month starts with the card bill's amount
//! as its regular card spending. After the 25th month 0's statement is
//! closed (charged in month 1, nothing more goes on it) and the owner types
//! the next statement's usage, which month 1 holds.

use super::ladder::{CardRoom, GroupRoom, MonthBaseline};
use super::schedule::statement_month;
use crate::period::shift_month;
use crate::text::casefold;
use finance_types::gateway::{BudgetGroup, BudgetPlan, WishSettings};
use finance_types::report::allocation::{CurrencyAllocation, GroupAllocation};
use jiff::civil::Date;
use rust_decimal::Decimal;

/// Parse a `YYYY-MM-DD` or `YYYY-MM` month as its first day.
pub(crate) fn parse_month(text: &str) -> Option<Date> {
    text.parse::<Date>()
        .or_else(|_| format!("{text}-01").parse::<Date>())
        .ok()
        .map(|d| d.first_of_month())
}

/// The allocation's current month, as its first day.
pub fn current_month(alloc: &CurrencyAllocation) -> Option<Date> {
    parse_month(&alloc.current_month)
}

/// The allocation currency's plans at or before `month`, latest first.
fn plans_upto<'a>(
    plans: &'a [BudgetPlan],
    alloc: &CurrencyAllocation,
    month: Date,
) -> Vec<&'a BudgetPlan> {
    let mut upto: Vec<(Date, &BudgetPlan)> = plans
        .iter()
        .filter(|p| p.currency == alloc.currency)
        .filter_map(|p| parse_month(&p.month).map(|m| (m, p)))
        .filter(|(m, _)| *m <= month)
        .collect();
    upto.sort_by_key(|(m, _)| std::cmp::Reverse(*m));
    upto.into_iter().map(|(_, p)| p).collect()
}

/// The group named `name` in the latest plan that ranks it (`plans` latest
/// first): its importance and floor are the ones that hold.
pub fn ranking<'a>(
    plans: impl IntoIterator<Item = &'a BudgetPlan>,
    name: &str,
) -> Option<&'a BudgetGroup> {
    let key = casefold(name);
    plans
        .into_iter()
        .filter_map(|p| p.groups.iter().find(|g| casefold(&g.name) == key))
        .find(|g| g.importance.is_some())
}

/// The ranked groups among `rows`; `keep_spent` false treats nothing as spent.
/// A group's id is the latest plan's; its importance and floor come from the
/// latest plan that ranks it, so a default plan keeps last month's ranking.
fn rooms(rows: &[GroupAllocation], plans: &[&BudgetPlan], keep_spent: bool) -> Vec<GroupRoom> {
    rows.iter()
        .filter_map(|row| {
            let key = casefold(&row.group);
            let latest = plans
                .iter()
                .find_map(|p| p.groups.iter().find(|g| casefold(&g.name) == key))?;
            let ranked = ranking(plans.iter().copied(), &row.group)?;
            let importance = ranked.importance?;
            let spent = if keep_spent { row.spent } else { Decimal::ZERO };
            Some(GroupRoom {
                id: latest.id.clone(),
                name: row.group.clone(),
                importance,
                limit: row.limit,
                min: ranked.floor.unwrap_or(Decimal::ZERO).max(spent),
            })
        })
        .collect()
}

/// Each card bill as next month's fixed costs counted it: the usage typed for
/// month 0's statement (charged in month 1), else the bill's own amount.
pub fn counted_bills(
    card_bills: &[(String, Decimal)],
    settings: &WishSettings,
    month0: Date,
) -> Vec<(String, Decimal)> {
    card_bills
        .iter()
        .map(|(id, bill)| {
            let typed = settings
                .cards
                .iter()
                .find(|c| c.card_id == *id && parse_month(&c.used_month) == Some(month0));
            (id.clone(), typed.map_or(*bill, |c| c.used))
        })
        .collect()
}

/// One baseline per month for `months` months, and the ids of the cards whose
/// recorded usage is for a statement that closed before `today`. `card_bills`
/// is each card's linked bill amount (a card without a linked bill has no
/// entry).
///
/// The statement open on `today` is month 0's or month 1's. Any other
/// (`today` far from the budget month) marks every card with usage stale.
pub fn baselines(
    alloc: &CurrencyAllocation,
    today: Date,
    plans: &[BudgetPlan],
    card_bills: &[(String, Decimal)],
    settings: &WishSettings,
    earmarked: Decimal,
    months: usize,
) -> (Vec<MonthBaseline>, Vec<String>) {
    let Some(month0) = current_month(alloc) else {
        return (Vec::new(), Vec::new());
    };
    let mut stale = Vec::new();
    let mut month0_cards = Vec::new();
    let mut month1_cards = Vec::new();
    let mut next_cards = Vec::new();
    let month1 = shift_month(month0, 1);
    for card in &settings.cards {
        let bill = card_bills
            .iter()
            .find(|(id, _)| *id == card.card_id)
            .map_or(Decimal::ZERO, |(_, amount)| *amount);
        // Usage is for the statement it names, and goes stale once that
        // statement closes (after its 25th).
        let typed = parse_month(&card.used_month);
        let open = statement_month(today);
        let unset = card.used_month.is_empty() && card.used.is_zero();
        if typed != Some(open) && !unset {
            stale.push(card.card_id.clone());
        }
        let room = |used, limit| CardRoom {
            card_id: card.card_id.clone(),
            name: card.name.clone(),
            limit,
            used,
        };
        if open == month1 {
            // Month 0's statement is closed: still charged in month 1 (what
            // was typed for it, else the bill), with no room left on it.
            let closed = if typed == Some(month0) {
                card.used
            } else {
                bill
            };
            let open_used = if typed == Some(month1) {
                bill.max(card.used)
            } else {
                bill
            };
            month0_cards.push(room(closed, closed));
            month1_cards.push(room(open_used, card.limit));
        } else {
            let used = if typed == Some(open) && open == month0 {
                card.used
            } else {
                Decimal::ZERO
            };
            month0_cards.push(room(used, card.limit));
            month1_cards.push(room(bill, card.limit));
        }
        next_cards.push(room(bill, card.limit));
    }
    let bills_total: Decimal = counted_bills(card_bills, settings, month0)
        .iter()
        .map(|(_, amount)| *amount)
        .sum();
    // `next_wiggle_room` is clamped at zero; the deficit is `next_shortfall`.
    // Next month's fixed costs count each card bill and its savings goals'
    // contributions; every later month pays the cards through the ladder
    // instead (last month's usage), and its goals are not money to spend, so
    // the bills are added back and each card's regular spending is seeded as
    // its usage. The goals are added back too: they are set aside, not spent.
    let next_free = alloc.next_wiggle_room - alloc.next_shortfall + alloc.next_goals + bills_total;
    // Without a plan for next month there are no next rows: repeat this
    // month's groups with nothing spent.
    let (next_rows, next_spent) = if alloc.next_group_rows.is_empty() {
        (&alloc.groups, false)
    } else {
        (&alloc.next_group_rows, true)
    };
    // The savings pot wishes may draw on once free money runs out, less
    // what is already saved for wishes (each wish's remaining excludes it).
    // Money saved for wishes is deducted once: from the pot first, and only
    // what the pot cannot cover comes out of this month's free money.
    let pot_raw = alloc.savings_balance + alloc.protected_savings;
    let savings = (pot_raw - earmarked).max(Decimal::ZERO);
    let free0 = alloc.current_wiggle_room - (earmarked - pot_raw).max(Decimal::ZERO);
    let out = (0..months)
        .map(|i| {
            let month = shift_month(month0, i32::try_from(i).unwrap_or(i32::MAX));
            let plans = plans_upto(plans, alloc, month);
            match i {
                0 => MonthBaseline {
                    month,
                    free: free0,
                    groups: rooms(&alloc.groups, &plans, true),
                    cards: month0_cards.clone(),
                    savings,
                },
                _ => MonthBaseline {
                    month,
                    free: next_free,
                    groups: rooms(next_rows, &plans, i == 1 && next_spent),
                    cards: if i == 1 {
                        month1_cards.clone()
                    } else {
                        next_cards.clone()
                    },
                    savings,
                },
            }
        })
        .collect();
    (out, stale)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wishes::ladder::Demand;
    use finance_types::gateway::{BudgetGroup, CardSettings};
    use finance_types::report::Currency;
    use std::str::FromStr;

    fn oct20() -> Date {
        Date::new(2026, 10, 20).unwrap()
    }

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
            current_extra_income: Decimal::ZERO,
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
            cards: vec![CardSettings {
                used: d(card_used),
                used_month: month.into(),
                ..WishSettings::default().cards[0].clone()
            }],
            ..WishSettings::default()
        }
    }

    fn card(id: &str, used_month: &str, used: &str, bill_id: Option<&str>) -> CardSettings {
        CardSettings {
            card_id: id.into(),
            name: id.into(),
            limit: d("2500"),
            used: d(used),
            used_month: used_month.into(),
            bill_id: bill_id.map(Into::into),
        }
    }

    /// The first card's linked bill, as `dashboard_parts` reports it.
    fn bill(amount: &str) -> Vec<(String, Decimal)> {
        vec![("card-1".to_string(), d(amount))]
    }

    #[test]
    fn typed_usage_is_the_bill_next_month_counted() {
        let month0 = Date::new(2026, 10, 1).unwrap();
        let cards = |month: &str| WishSettings {
            cards: vec![card("card-1", month, "9.98", Some("b1"))],
            ..WishSettings::default()
        };
        // Typed for month 0's statement: that is what month 1 is charged.
        let counted = counted_bills(&bill("400"), &cards("2026-10-01"), month0);
        assert_eq!(counted, bill("9.98"));
        // Typed for another statement: the bill's own amount stands.
        let counted = counted_bills(&bill("400"), &cards("2026-09-01"), month0);
        assert_eq!(counted, bill("400"));
    }

    #[test]
    fn month_zero_uses_wiggle_room_minus_earmark() {
        let a = alloc("400", "0", vec![], vec![]);
        let (months, _) = baselines(&a, oct20(), &[], &[], &settings("0", ""), d("300"), 2);
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
            oct20(),
            &plans,
            &[],
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
    fn importance_carries_from_an_earlier_plan() {
        let a = alloc(
            "0",
            "0",
            vec![row("Groceries", "450", "100")],
            vec![row("Groceries", "300", "0")],
        );
        let plans = [
            plan(
                "2026-10-01",
                vec![plan_group("g-oct", "Groceries", Some(1), Some("250"))],
            ),
            // November is the default plan: no importance, its own id.
            plan(
                "2026-11-01",
                vec![plan_group("groceries", "Groceries", None, None)],
            ),
        ];
        let (months, _) = baselines(
            &a,
            oct20(),
            &plans,
            &[],
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        for m in &months[1..] {
            assert_eq!(m.groups.len(), 1);
            assert_eq!(m.groups[0].id, "groceries");
            assert_eq!(m.groups[0].importance, 1);
            assert_eq!(m.groups[0].min, d("250"));
        }
        assert_eq!(months[0].groups[0].id, "g-oct");
    }

    #[test]
    fn missing_next_rows_fall_back_to_this_months_groups() {
        let a = alloc("0", "0", vec![row("Going out", "120", "90")], vec![]);
        let plans = [plan(
            "2026-10-01",
            vec![plan_group("g", "Going out", Some(2), None)],
        )];
        let (months, _) = baselines(
            &a,
            oct20(),
            &plans,
            &[],
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[0].groups[0].min, d("90"));
        for m in &months[1..] {
            assert_eq!(m.groups[0].limit, d("120"));
            assert_eq!(m.groups[0].min, Decimal::ZERO);
        }
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
            oct20(),
            &plans,
            &[],
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[1].groups[0].min, d("30"));
        assert_eq!(months[2].groups[0].min, Decimal::ZERO);
        assert_eq!(months[2].free, d("50"));
        assert_eq!(months[2].month, Date::new(2026, 12, 1).unwrap());
        assert_eq!(months[1].cards[0].used, Decimal::ZERO);
    }

    #[test]
    fn stale_card_usage_counts_as_zero() {
        let a = alloc("0", "0", vec![], vec![]);
        let (months, stale) = baselines(
            &a,
            oct20(),
            &[],
            &[],
            &settings("800", "2026-09-01"),
            Decimal::ZERO,
            2,
        );
        assert_eq!(months[0].cards[0].used, Decimal::ZERO);
        assert_eq!(stale, vec!["card-1".to_string()]);
    }

    #[test]
    fn current_card_usage_is_used() {
        let a = alloc("0", "0", vec![], vec![]);
        let (months, stale) = baselines(
            &a,
            oct20(),
            &[],
            &[],
            &settings("800", "2026-10-01"),
            Decimal::ZERO,
            2,
        );
        assert_eq!(months[0].cards[0].used, d("800"));
        assert!(stale.is_empty());
        let (_, stale) = baselines(&a, oct20(), &[], &[], &settings("0", ""), Decimal::ZERO, 2);
        assert!(stale.is_empty());
    }

    #[test]
    fn next_month_shortfall_makes_free_negative() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.next_shortfall = d("50");
        let (months, _) = baselines(
            &a,
            oct20(),
            &[],
            &bill("9.98"),
            &settings("0", ""),
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[1].free, d("-40.02"));
        assert_eq!(months[2].free, d("-40.02"));
    }

    #[test]
    fn regular_card_spending_carries_into_every_later_month() {
        let a = alloc("0", "200", vec![], vec![]);
        let (months, _) = baselines(
            &a,
            oct20(),
            &[],
            &bill("100"),
            &settings("40", "2026-10-01"),
            Decimal::ZERO,
            4,
        );
        assert_eq!(months[0].cards[0].used, d("40"));
        for m in &months[1..] {
            // Next month's fixed costs counted the typed 40, not the 100.
            assert_eq!(m.free, d("240"));
            assert_eq!(m.cards[0].used, d("100"));
        }
        // Each later month pays the month before's regular card spending.
        let mut saver = super::super::schedule::Saver::new(&[], months[0].month, 4);
        let judged =
            super::super::ladder::judge(&months, &[vec![], vec![], vec![], vec![]], &mut saver);
        assert_eq!(judged[1].card_charge_in, d("40"));
        assert_eq!(judged[2].card_charge_in, d("100"));
        assert_eq!(judged[3].card_charge_in, d("100"));
        assert_eq!(judged[3].free_left, d("140"));
    }

    #[test]
    fn card_bill_added_back_next_month() {
        let a = alloc("0", "200", vec![], vec![]);
        let (months, _) = baselines(
            &a,
            oct20(),
            &[],
            &bill("9.98"),
            &settings("0", ""),
            Decimal::ZERO,
            2,
        );
        assert_eq!(months[1].free, d("209.98"));
        assert_eq!(months[0].free, Decimal::ZERO);
    }

    #[test]
    fn savings_pot_excludes_money_saved_for_wishes() {
        let mut a = alloc("0", "100", vec![], vec![]);
        a.savings_balance = d("500");
        a.protected_savings = d("100");
        let (months, _) = baselines(&a, oct20(), &[], &[], &WishSettings::default(), d("150"), 3);
        assert!(months.iter().all(|m| m.savings == d("450")));
        let (months, _) = baselines(&a, oct20(), &[], &[], &WishSettings::default(), d("700"), 3);
        assert!(months.iter().all(|m| m.savings.is_zero()));
    }

    #[test]
    fn earmarked_is_deducted_once() {
        let mut a = alloc("100", "0", vec![], vec![]);
        a.savings_balance = d("1000");
        let (months, _) = baselines(&a, oct20(), &[], &[], &WishSettings::default(), d("500"), 2);
        assert_eq!(months[0].free, d("100"));
        assert!(months.iter().all(|m| m.savings == d("500")));
        a.savings_balance = d("200");
        let (months, _) = baselines(&a, oct20(), &[], &[], &WishSettings::default(), d("500"), 2);
        assert_eq!(months[0].free, d("-200"));
        assert!(months.iter().all(|m| m.savings.is_zero()));
    }

    #[test]
    fn later_months_add_goal_contributions_back() {
        let mut a = alloc("0", "100", vec![], vec![]);
        a.next_shortfall = Decimal::ZERO;
        a.next_goals = d("250");
        let (months, _) = baselines(
            &a,
            oct20(),
            &[],
            &[],
            &WishSettings::default(),
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[1].free, d("350"));
        assert_eq!(months[2].free, d("350"));
    }

    #[test]
    fn cards_seed_from_their_own_bills_and_stale_is_per_card() {
        let settings = WishSettings {
            cards: vec![
                card("visa", "2026-10-01", "40", Some("b1")),
                card("mc", "2026-09-01", "70", None),
            ],
            ..WishSettings::default()
        };
        let a = alloc("0", "200", vec![], vec![]);
        let (months, stale) = baselines(
            &a,
            oct20(),
            &[],
            &[("visa".into(), d("500"))],
            &settings,
            Decimal::ZERO,
            3,
        );
        assert_eq!(months[0].cards[0].used, d("40"));
        assert_eq!(months[0].cards[1].used, Decimal::ZERO);
        assert_eq!(stale, vec!["mc".to_string()]);
        assert_eq!(months[1].cards[0].used, d("500"));
        assert_eq!(months[1].cards[1].used, Decimal::ZERO);
        // Visa's usage was typed for month 0: that 40 is what was counted.
        assert_eq!(months[1].free, a.next_wiggle_room + a.next_goals + d("40"));
    }

    #[test]
    fn card_usage_goes_stale_after_the_25th() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.current_month = "2026-11-01".into();
        let s = settings("800", "2026-11-01");
        let on = |day| Date::new(2026, 11, day).unwrap();
        let (months, stale) = baselines(&a, on(20), &[], &[], &s, Decimal::ZERO, 2);
        assert_eq!(months[0].cards[0].used, d("800"));
        assert!(stale.is_empty());
        // On 26 Nov November's statement is closed: it is still charged in
        // December, but nothing more goes on it.
        let (months, stale) = baselines(&a, on(26), &[], &[], &s, Decimal::ZERO, 2);
        assert_eq!(months[0].cards[0].used, d("800"));
        assert_eq!(months[0].cards[0].limit, d("800"));
        assert_eq!(stale, vec!["card-1".to_string()]);
    }

    #[test]
    fn next_statement_usage_counts_in_month_one() {
        // On 27 Oct the owner types November's statement so far: October's
        // statement is closed, so month 1 (November) holds it, and October's
        // falls back to the card bill with no room left.
        let a = alloc("0", "0", vec![], vec![]);
        let today = Date::new(2026, 10, 27).unwrap();
        let s = settings("120", "2026-11-01");
        let (months, stale) = baselines(&a, today, &[], &bill("100"), &s, Decimal::ZERO, 3);
        assert!(stale.is_empty());
        assert_eq!(months[0].cards[0].used, d("100"));
        assert_eq!(months[0].cards[0].limit, d("100"));
        assert_eq!(months[1].cards[0].used, d("120"));
        assert_eq!(months[2].cards[0].used, d("100"));
        // Below the card's regular spending, the bill still seeds month 1.
        let s = settings("40", "2026-11-01");
        let (months, _) = baselines(&a, today, &[], &bill("100"), &s, Decimal::ZERO, 3);
        assert_eq!(months[1].cards[0].used, d("100"));
        // October's usage is stale once its statement closed, but it is
        // still what October's statement charges in November.
        let s = settings("40", "2026-10-01");
        let (months, stale) = baselines(&a, today, &[], &bill("100"), &s, Decimal::ZERO, 3);
        assert_eq!(stale, vec!["card-1".to_string()]);
        assert_eq!(months[0].cards[0].used, d("40"));
        assert_eq!(months[0].cards[0].limit, d("40"));
        assert_eq!(months[1].cards[0].used, d("100"));
    }

    #[test]
    fn closed_statement_is_charged_next_month_and_takes_nothing() {
        let a = alloc("1000", "1000", vec![], vec![]);
        let today = Date::new(2026, 10, 27).unwrap();
        let s = settings("120", "2026-11-01");
        let (months, _) = baselines(&a, today, &[], &bill("100"), &s, Decimal::ZERO, 3);
        let card = |label: &str| Demand {
            wish_id: Some(label.into()),
            label: label.into(),
            amount: d("50"),
            on_card: true,
            set_aside: false,
            must: false,
        };
        let mut saver = super::super::schedule::Saver::new(&[], months[0].month, 3);
        let judged = super::super::ladder::judge(
            &months,
            &[vec![card("oct")], vec![card("nov")], vec![]],
            &mut saver,
        );
        // October's statement (the bill) is charged in November.
        assert_eq!(judged[1].card_charge_in, d("100"));
        // Month 0 cannot put anything on the card.
        assert_eq!(judged[0].verdicts[0].on_card, Decimal::ZERO);
        assert_eq!(judged[0].verdicts[0].from_free, d("50"));
        // November starts at the 120 typed and takes the 50.
        assert_eq!(judged[1].card_used, d("170"));
        assert_eq!(judged[2].card_charge_in, d("170"));
    }
}
