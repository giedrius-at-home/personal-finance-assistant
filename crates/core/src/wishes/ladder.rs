//! The cutting ladder: how each planned purchase is paid for, month by month.
//!
//! Pure arithmetic, no I/O. For every demand the order is: the cards first
//! when the wish asked for them, then free money, then the savings pot, then
//! cuts to budget groups (the highest importance number first, never below
//! their floor), then the cards, and what is left is `short`. The cards are
//! one pool, filled in list order, each up to its limit.
//!
//! Each month first pays what it already lacks ("Already short", when its
//! free money is negative), then last month's card charge ("Credit card"),
//! which may not go back on the card: what free money, savings and cuts
//! cannot pay is short, and that short and those cuts are traced back to the
//! verdicts that put money on the card the month before. Then money a cheaper
//! bill frees, then the must wishes, then the rest: within each, set-asides,
//! purchases, then repeating costs.
//!
//! A set-aside takes as much of its share as free money, savings and cuts
//! allow and never touches the cards; the saver spreads what was not saved
//! over the months left. A month where nothing fits is a skip.

use super::schedule::Saver;
use finance_types::report::wishes::{CardUsed, Cut, Verdict, WishMonth};
use jiff::civil::Date;
use rust_decimal::Decimal;

/// A budget group that can be trimmed.
#[derive(Clone, Debug)]
pub struct GroupRoom {
    pub id: String,
    pub name: String,
    pub importance: u8,
    pub limit: Decimal,
    /// `max(floor, spent)`: the lowest the limit may go.
    pub min: Decimal,
}

/// One month's starting position.
#[derive(Clone, Debug)]
pub struct MonthBaseline {
    pub month: Date,
    pub free: Decimal,
    pub groups: Vec<GroupRoom>,
    /// What is already on each card this month before any wish.
    pub cards: Vec<CardRoom>,
    /// The savings pot; only month 0's is read, later months carry what the
    /// month before left.
    pub savings: Decimal,
}

/// One card's limit and what is already on it.
#[derive(Clone, Debug)]
pub struct CardRoom {
    pub card_id: String,
    pub name: String,
    pub limit: Decimal,
    pub used: Decimal,
}

/// Something to pay for in a month.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    pub wish_id: Option<String>,
    pub label: String,
    pub amount: Decimal,
    pub on_card: bool,
    /// A monthly saving set-aside rather than the purchase itself.
    pub set_aside: bool,
    /// For a wish that must be bought.
    pub must: bool,
}

pub const ALREADY_SHORT: &str = "Already short";
pub const CARD_CHARGE: &str = "Credit card";

/// What a month pays with while its demands are judged. Owned, so a
/// set-aside can be tried on a copy.
#[derive(Clone, Debug)]
struct Purse {
    free: Decimal,
    savings: Decimal,
    /// In list order; `used` grows as wishes go on them.
    cards: Vec<CardRoom>,
    /// Groups in the order they are cut; limits carry earlier cuts.
    groups: Vec<(GroupRoom, Decimal)>,
}

/// A verdict for `demand` before any money is found.
fn verdict(demand: Demand) -> Verdict {
    Verdict {
        wish_id: demand.wish_id,
        label: demand.label,
        amount: demand.amount,
        from_free: Decimal::ZERO,
        from_savings: Decimal::ZERO,
        cuts: Vec::new(),
        on_card: Decimal::ZERO,
        short: Decimal::ZERO,
        set_aside: demand.set_aside,
        next_short: Decimal::ZERO,
        next_cuts: Vec::new(),
        must: demand.must,
        skipped: false,
    }
}

impl Purse {
    fn card_used(&self) -> Decimal {
        self.cards.iter().map(|c| c.used).sum()
    }

    /// Put up to `amount` on the cards, in list order; returns what went on.
    fn take_card(&mut self, amount: Decimal) -> Decimal {
        let mut rem = amount;
        for card in &mut self.cards {
            let take = rem.min((card.limit - card.used).max(Decimal::ZERO));
            card.used += take;
            rem -= take;
        }
        amount - rem
    }

    /// Run one demand down the ladder; `card_allowed` false skips the cards.
    fn pay(&mut self, demand: Demand, card_allowed: bool) -> Verdict {
        if demand.amount < Decimal::ZERO {
            // A cheaper bill: money comes back.
            self.free -= demand.amount;
            return Verdict {
                from_free: demand.amount,
                ..verdict(demand)
            };
        }
        let mut rem = demand.amount;
        let mut on_card = Decimal::ZERO;
        if demand.on_card && card_allowed {
            let take = self.take_card(rem);
            on_card += take;
            rem -= take;
        }
        let from_free = rem.min(self.free.max(Decimal::ZERO));
        self.free -= from_free;
        rem -= from_free;
        let from_savings = rem.min(self.savings.max(Decimal::ZERO));
        self.savings -= from_savings;
        rem -= from_savings;

        let mut cuts = Vec::new();
        for (group, current) in &mut self.groups {
            if rem <= Decimal::ZERO {
                break;
            }
            let room = (*current - group.min).max(Decimal::ZERO);
            let take = rem.min(room);
            if take > Decimal::ZERO {
                cuts.push(Cut {
                    group_id: group.id.clone(),
                    name: group.name.clone(),
                    from: *current,
                    to: *current - take,
                });
                *current -= take;
                rem -= take;
            }
        }

        if !demand.on_card && card_allowed {
            let take = self.take_card(rem);
            on_card += take;
            rem -= take;
        }
        Verdict {
            from_free,
            from_savings,
            cuts,
            on_card,
            short: rem,
            ..verdict(demand)
        }
    }
}

/// Give each verdict that put money on the card its share (by `on_card`
/// over the whole charge) of the charge's cuts and short.
fn trace_charge(prev: &mut WishMonth, charge: &Verdict) {
    if prev.card_used <= Decimal::ZERO {
        return;
    }
    let total = prev.card_used;
    for v in prev
        .verdicts
        .iter_mut()
        .filter(|v| v.wish_id.is_some() && v.on_card > Decimal::ZERO)
    {
        let share = v.on_card / total;
        v.next_short = (charge.short * share).round_dp(2);
        v.next_cuts = charge
            .cuts
            .iter()
            .filter_map(|cut| {
                let take = ((cut.from - cut.to) * share).round_dp(2);
                (take > Decimal::ZERO).then(|| Cut {
                    to: cut.from - take,
                    ..cut.clone()
                })
            })
            .collect();
    }
}

/// A demand the ladder raises itself, not a wish's.
fn own(label: &str, amount: Decimal) -> Demand {
    Demand {
        wish_id: None,
        label: label.to_string(),
        amount,
        on_card: false,
        set_aside: false,
        must: false,
    }
}

/// Pay as much of a set-aside as fits without the cards; what does not fit
/// is left for later months. Nothing fitting is a skip.
fn set_aside(purse: &mut Purse, demand: Demand) -> Verdict {
    let mut trial = purse.clone();
    let tried = trial.pay(demand.clone(), false);
    let paid = demand.amount - tried.short;
    if paid <= Decimal::ZERO {
        return Verdict {
            amount: Decimal::ZERO,
            skipped: true,
            ..verdict(demand)
        };
    }
    *purse = trial;
    Verdict {
        amount: paid,
        short: Decimal::ZERO,
        ..tried
    }
}

/// Judge every month. `repeats[i]` are month `i`'s recurring wishes and bill
/// changes (money freed first); `saver` hands out the one-off wishes'
/// set-asides and purchases and learns what was paid.
pub fn judge(
    months: &[MonthBaseline],
    repeats: &[Vec<Demand>],
    saver: &mut Saver,
) -> Vec<WishMonth> {
    let mut out: Vec<WishMonth> = Vec::with_capacity(months.len());
    for (i, base) in months.iter().enumerate() {
        let card_charge_in = out.last().map_or(Decimal::ZERO, |prev| prev.card_used);
        let savings_before = out.last().map_or(base.savings, |prev| prev.savings_left);
        let mut groups: Vec<(GroupRoom, Decimal)> =
            base.groups.iter().map(|g| (g.clone(), g.limit)).collect();
        groups.sort_by(|a, b| {
            b.0.importance
                .cmp(&a.0.importance)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });
        let mut purse = Purse {
            free: base.free.max(Decimal::ZERO),
            savings: savings_before,
            cards: base.cards.clone(),
            groups,
        };

        let mut verdicts = Vec::new();
        if base.free < Decimal::ZERO {
            verdicts.push(purse.pay(own(ALREADY_SHORT, -base.free), true));
        }
        if card_charge_in > Decimal::ZERO {
            let charge = purse.pay(own(CARD_CHARGE, card_charge_in), false);
            if let Some(prev) = out.last_mut() {
                trace_charge(prev, &charge);
            }
            verdicts.push(charge);
        }

        let (freed, costs): (Vec<Demand>, Vec<Demand>) = repeats
            .get(i)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .partition(|d| d.amount < Decimal::ZERO);
        for demand in freed {
            verdicts.push(purse.pay(demand, true));
        }
        let set_asides = saver.set_asides(i);
        for must in [true, false] {
            for demand in set_asides.iter().filter(|d| d.must == must).cloned() {
                let id = demand.wish_id.clone().unwrap_or_default();
                let v = set_aside(&mut purse, demand);
                if !v.skipped {
                    saver.paid(&id, v.amount);
                }
                verdicts.push(v);
            }
            for demand in saver.purchases(i).into_iter().filter(|d| d.must == must) {
                if let Some(id) = &demand.wish_id {
                    saver.paid(id, demand.amount);
                }
                verdicts.push(purse.pay(demand, true));
            }
            for demand in costs.iter().filter(|d| d.must == must).cloned() {
                verdicts.push(purse.pay(demand, true));
            }
        }

        out.push(WishMonth {
            month: format!("{:04}-{:02}-01", base.month.year(), base.month.month()),
            free_before: base.free,
            free_left: purse.free,
            savings_before,
            savings_left: purse.savings,
            card_charge_in,
            card_used: purse.card_used(),
            cards: purse
                .cards
                .iter()
                .map(|c| CardUsed {
                    card_id: c.card_id.clone(),
                    name: c.name.clone(),
                    used: c.used,
                })
                .collect(),
            verdicts,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wishes::schedule::{Planned, Saver};
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn group(id: &str, importance: u8, limit: &str, min: &str) -> GroupRoom {
        GroupRoom {
            id: id.into(),
            name: id.into(),
            importance,
            limit: d(limit),
            min: d(min),
        }
    }

    fn with_card(free: &str, groups: Vec<GroupRoom>, card_used: &str) -> MonthBaseline {
        MonthBaseline {
            month: Date::new(2026, 11, 1).unwrap(),
            free: d(free),
            groups,
            cards: vec![CardRoom {
                card_id: "card-1".into(),
                name: "Credit card".into(),
                limit: d("2500"),
                used: d(card_used),
            }],
            savings: Decimal::ZERO,
        }
    }

    fn demand(label: &str, amount: &str, on_card: bool) -> Demand {
        Demand {
            wish_id: Some(label.into()),
            label: label.into(),
            amount: d(amount),
            on_card,
            set_aside: false,
            must: false,
        }
    }

    fn base_at(month: u8, free: &str, groups: Vec<GroupRoom>, card_used: &str) -> MonthBaseline {
        MonthBaseline {
            month: Date::new(2026, i8::try_from(month).unwrap(), 1).unwrap(),
            ..with_card(free, groups, card_used)
        }
    }

    fn two_groups() -> Vec<GroupRoom> {
        vec![
            group("groceries", 1, "450", "300"),
            group("going out", 2, "120", "0"),
        ]
    }

    #[test]
    fn fits_in_free_money() {
        let out = judge(
            &[with_card("500", vec![], "0")],
            &[vec![demand("tv", "150", false)]],
            &mut no_saver(),
        );
        let v = &out[0].verdicts[0];
        assert_eq!(v.from_free, d("150"));
        assert!(v.cuts.is_empty());
        assert_eq!(out[0].free_left, d("350"));
        assert_eq!(out[0].free_before, d("500"));
        assert_eq!(out[0].month, "2026-11-01");
    }

    #[test]
    fn cuts_going_out_before_groceries() {
        let out = judge(
            &[with_card("0", two_groups(), "0")],
            &[vec![demand("tv", "200", false)]],
            &mut no_saver(),
        );
        let v = &out[0].verdicts[0];
        assert_eq!(
            v.cuts,
            vec![
                Cut {
                    group_id: "going out".into(),
                    name: "going out".into(),
                    from: d("120"),
                    to: d("0")
                },
                Cut {
                    group_id: "groceries".into(),
                    name: "groceries".into(),
                    from: d("450"),
                    to: d("370")
                },
            ]
        );
        assert_eq!(v.on_card, d("0"));
    }

    #[test]
    fn groceries_never_below_300_then_card() {
        let out = judge(
            &[
                with_card("0", two_groups(), "9.98"),
                with_card("0", vec![], "0"),
            ],
            &[vec![demand("tv", "400", false)], vec![]],
            &mut no_saver(),
        );
        let v = &out[0].verdicts[0];
        assert_eq!(v.cuts[0].to, d("0"));
        assert_eq!(v.cuts[1].to, d("300"));
        assert_eq!(v.on_card, d("130"));
        assert_eq!(out[0].card_used, d("139.98"));
        assert_eq!(out[1].card_charge_in, d("139.98"));
        assert_eq!(out[1].verdicts[0].label, "Credit card");
        assert_eq!(out[1].verdicts[0].wish_id, None);
    }

    #[test]
    fn card_wish_goes_on_card_first() {
        let out = judge(
            &[
                with_card("500", vec![], "9.98"),
                with_card("2000", vec![], "0"),
            ],
            &[vec![demand("trip", "900", true)], vec![]],
            &mut no_saver(),
        );
        let v = &out[0].verdicts[0];
        assert_eq!(v.on_card, d("900"));
        assert_eq!(v.from_free, d("0"));
        assert_eq!(out[0].free_left, d("500"));
        assert_eq!(out[1].card_charge_in, d("909.98"));
    }

    #[test]
    fn card_full_leaves_short() {
        let out = judge(
            &[with_card("0", vec![], "2500")],
            &[vec![demand("tv", "50", false)]],
            &mut no_saver(),
        );
        assert_eq!(out[0].verdicts[0].short, d("50"));
    }

    #[test]
    fn wishes_stack_in_order() {
        let out = judge(
            &[with_card("100", vec![], "0")],
            &[vec![demand("a", "80", false), demand("b", "80", false)]],
            &mut no_saver(),
        );
        assert_eq!(out[0].verdicts[0].from_free, d("80"));
        assert_eq!(out[0].verdicts[1].from_free, d("20"));
        assert_eq!(out[0].verdicts[1].on_card, d("60"));
    }

    #[test]
    fn overspent_group_gives_no_room() {
        let out = judge(
            &[with_card("0", vec![group("fun", 1, "100", "140")], "0")],
            &[vec![demand("tv", "50", false)]],
            &mut no_saver(),
        );
        let v = &out[0].verdicts[0];
        assert!(v.cuts.is_empty());
        assert_eq!(v.on_card, d("50"));
    }

    #[test]
    fn unpaid_card_charge_is_short_not_rolled() {
        let out = judge(
            &[
                with_card("0", vec![], "0"),
                with_card("0", vec![], "0"),
                with_card("0", vec![], "0"),
            ],
            &[vec![demand("tv", "300", false)], vec![], vec![]],
            &mut no_saver(),
        );
        let charge = &out[1].verdicts[0];
        assert_eq!(charge.label, "Credit card");
        assert_eq!(charge.on_card, Decimal::ZERO);
        assert_eq!(charge.short, d("300"));
        assert_eq!(out[2].card_charge_in, Decimal::ZERO);
        assert_eq!(out[0].verdicts[0].next_short, d("300"));
    }

    #[test]
    fn card_wish_with_nothing_free_next_month_is_short() {
        let out = judge(
            &[base_at(10, "0", vec![], "0"), base_at(11, "0", vec![], "0")],
            &[vec![demand("tv", "500", true)], vec![]],
            &mut no_saver(),
        );
        let charge = &out[1].verdicts[0];
        assert_eq!(
            (charge.wish_id.as_deref(), charge.label.as_str()),
            (None, "Credit card")
        );
        assert_eq!(charge.short, d("500"));
        assert_eq!(out[0].verdicts[0].on_card, d("500"));
        assert_eq!(out[0].verdicts[0].next_short, d("500"));
        assert!(out[0].verdicts[0].next_cuts.is_empty());
    }

    #[test]
    fn card_charge_paid_from_free_then_cuts() {
        let out = judge(
            &[
                base_at(10, "0", vec![], "0"),
                base_at(11, "50", vec![group("going out", 2, "120", "0")], "0"),
            ],
            &[vec![demand("tv", "100", true)], vec![]],
            &mut no_saver(),
        );
        let charge = &out[1].verdicts[0];
        assert_eq!(charge.from_free, d("50"));
        assert_eq!(charge.cuts[0].to, d("70"));
        assert_eq!(charge.short, Decimal::ZERO);
        assert_eq!(out[0].verdicts[0].next_short, Decimal::ZERO);
        assert_eq!(out[0].verdicts[0].next_cuts[0].to, d("70"));
    }

    #[test]
    fn month_already_short_is_paid_first() {
        let out = judge(
            &[with_card(
                "-50",
                vec![group("going out", 2, "120", "0")],
                "0",
            )],
            &[vec![demand("tv", "30", false)]],
            &mut no_saver(),
        );
        let first = &out[0].verdicts[0];
        assert_eq!(
            (first.wish_id.as_deref(), first.label.as_str()),
            (None, "Already short")
        );
        assert_eq!(first.amount, d("50"));
        assert_eq!(first.cuts[0].to, d("70"));
        let tv = &out[0].verdicts[1];
        assert_eq!(tv.from_free, Decimal::ZERO);
        assert_eq!(tv.cuts[0].from, d("70"));
        assert_eq!(tv.cuts[0].to, d("40"));
        assert_eq!(out[0].free_before, d("-50"));
        assert_eq!(out[0].free_left, Decimal::ZERO);
    }

    #[test]
    fn already_short_without_room_is_short() {
        let out = judge(
            &[with_card("-50", vec![], "2500")],
            &[vec![]],
            &mut no_saver(),
        );
        assert_eq!(out[0].verdicts[0].short, d("50"));
    }

    #[test]
    fn cuts_carry_across_demands_in_a_month() {
        let out = judge(
            &[with_card("0", two_groups(), "2500")],
            &[vec![demand("a", "100", false), demand("b", "100", false)]],
            &mut no_saver(),
        );
        let (a, b) = (&out[0].verdicts[0], &out[0].verdicts[1]);
        assert_eq!(a.cuts.len(), 1);
        assert_eq!((a.cuts[0].from, a.cuts[0].to), (d("120"), d("20")));
        assert_eq!((b.cuts[0].from, b.cuts[0].to), (d("20"), d("0")));
        assert_eq!((b.cuts[1].from, b.cuts[1].to), (d("450"), d("370")));
    }

    #[test]
    fn card_wish_over_card_room_falls_back_to_free_then_cuts() {
        let out = judge(
            &[with_card("30", two_groups(), "2400")],
            &[vec![demand("tv", "200", true)]],
            &mut no_saver(),
        );
        let tv = &out[0].verdicts[0];
        assert_eq!(tv.on_card, d("100"));
        assert_eq!(tv.from_free, d("30"));
        assert_eq!(tv.cuts[0].to, d("50"));
        assert_eq!(tv.short, Decimal::ZERO);
    }

    #[test]
    fn equal_importance_cut_by_name() {
        let out = judge(
            &[with_card(
                "0",
                vec![group("b", 2, "10", "0"), group("a", 2, "10", "0")],
                "2500",
            )],
            &[vec![demand("tv", "15", false)]],
            &mut no_saver(),
        );
        let names: Vec<_> = out[0].verdicts[0]
            .cuts
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn bill_change_down_frees_money() {
        let out = judge(
            &[with_card("100", vec![], "0")],
            &[vec![demand("rent", "-50", false)]],
            &mut no_saver(),
        );
        assert_eq!(out[0].free_left, d("150"));
        let v = &out[0].verdicts[0];
        assert_eq!(v.from_free, d("-50"));
        assert!(v.cuts.is_empty());
        assert_eq!(v.on_card, Decimal::ZERO);
        assert_eq!(v.short, Decimal::ZERO);
    }

    fn no_saver() -> Saver {
        Saver::new(&[], oct(), 0)
    }

    fn oct() -> Date {
        Date::new(2026, 10, 1).unwrap()
    }

    fn card(id: &str, limit: &str, used: &str) -> CardRoom {
        CardRoom {
            card_id: id.into(),
            name: id.into(),
            limit: d(limit),
            used: d(used),
        }
    }

    /// Visa (limit 100) and Mastercard (limit 200), both unused.
    fn two_cards() -> Vec<CardRoom> {
        vec![card("visa", "100", "0"), card("mc", "200", "0")]
    }

    fn base(free: &str, cards: Vec<CardRoom>) -> MonthBaseline {
        MonthBaseline {
            month: oct(),
            free: d(free),
            groups: Vec::new(),
            cards,
            savings: Decimal::ZERO,
        }
    }

    fn base_cards(free: &str, cards: Vec<(&str, &str, &str)>) -> MonthBaseline {
        base(
            free,
            cards
                .into_iter()
                .map(|(id, limit, used)| card(id, limit, used))
                .collect(),
        )
    }

    /// Consecutive months from October with these free amounts.
    fn months_of<const N: usize>(free: [&str; N], cards: &[CardRoom]) -> Vec<MonthBaseline> {
        free.iter()
            .enumerate()
            .map(|(i, f)| MonthBaseline {
                month: Date::new(2026, 10, 1)
                    .unwrap()
                    .checked_add(jiff::Span::new().months(i64::try_from(i).unwrap()))
                    .unwrap(),
                ..base(f, cards.to_vec())
            })
            .collect()
    }

    fn bases<const N: usize>(free: [&str; N]) -> Vec<MonthBaseline> {
        months_of(free, &[])
    }

    fn bases_with_cards<const N: usize>(free: [&str; N]) -> Vec<MonthBaseline> {
        months_of(free, &two_cards())
    }

    /// No cards; "going out" 100 (floor 0); `savings` in the pot.
    fn bases_savings<const N: usize>(free: [&str; N], savings: &str) -> Vec<MonthBaseline> {
        months_of(free, &[])
            .into_iter()
            .map(|m| MonthBaseline {
                groups: vec![group("going out", 2, "100", "0")],
                savings: d(savings),
                ..m
            })
            .collect()
    }

    fn saver(items: Vec<Planned>) -> Saver {
        Saver::new(&items, oct(), 12)
    }

    /// A one-off due `month` months from October; all share one creation
    /// time, so their order in the list is the order they were created.
    fn due_in(id: &str, amount: &str, month: usize) -> Planned {
        Planned {
            wish_id: id.into(),
            label: id.into(),
            due: oct()
                .checked_add(jiff::Span::new().months(i64::try_from(month).unwrap()))
                .unwrap(),
            remaining: d(amount),
            on_card: false,
            created_at: "2026-01-01T00:00:00".into(),
            per_month: None,
            saved_this_month: false,
            must: false,
        }
    }

    fn due_in_must(id: &str, amount: &str, month: usize) -> Planned {
        Planned {
            must: true,
            ..due_in(id, amount, month)
        }
    }

    /// Bought outright in `month`: saving was recorded this month, so no
    /// set-asides come before it.
    fn purchase_in(id: &str, amount: &str, month: usize) -> Planned {
        Planned {
            saved_this_month: true,
            ..due_in(id, amount, month)
        }
    }

    fn purchase(id: &str, amount: &str) -> Planned {
        purchase_in(id, amount, 0)
    }

    fn purchase_must(id: &str, amount: &str) -> Planned {
        Planned {
            must: true,
            ..purchase(id, amount)
        }
    }

    fn card_purchase(id: &str, amount: &str) -> Planned {
        Planned {
            on_card: true,
            ..purchase(id, amount)
        }
    }

    #[test]
    fn savings_are_drawn_before_groups_and_carry_down() {
        // free 0, savings 150, going out 100 (floor 0); purchase 120 in month 0, purchase 100 in month 1.
        let out = judge(
            &bases_savings(["0", "0"], "150"),
            &[vec![], vec![]],
            &mut saver(vec![purchase_in("a", "120", 0), purchase_in("b", "100", 1)]),
        );
        assert_eq!(out[0].verdicts[0].from_savings, d("120"));
        assert!(out[0].verdicts[0].cuts.is_empty());
        assert_eq!(out[0].savings_before, d("150"));
        assert_eq!(out[0].savings_left, d("30"));
        assert_eq!(out[1].savings_before, d("30"));
        assert_eq!(out[1].verdicts[0].from_savings, d("30"));
        // going out 100 → 30 for the remaining 70
        assert_eq!(out[1].verdicts[0].cuts[0].to, d("30"));
        assert_eq!(out[1].savings_left, Decimal::ZERO);
    }

    #[test]
    fn must_wish_is_paid_before_an_optional_one() {
        // free 100; optional purchase 80 created first, must purchase 80 due same month, no cards, no groups.
        let out = judge(
            &[base("100", vec![])],
            &[vec![]],
            &mut saver(vec![purchase("opt", "80"), purchase_must("must", "80")]),
        );
        let v = &out[0].verdicts;
        assert_eq!(v[0].wish_id.as_deref(), Some("must"));
        assert_eq!(v[0].short, Decimal::ZERO);
        assert!(v[0].must);
        assert_eq!(v[1].wish_id.as_deref(), Some("opt"));
        assert_eq!(v[1].short, d("60"));
    }

    #[test]
    fn kept_set_aside_lowers_what_later_months_save() {
        // free 300 / 300 / 300; wish 600 due month 2 → 200 kept in month 0,
        // so month 1 sets aside 400 / 2 = 200 and month 2 buys the last 200.
        let out = judge(
            &bases(["300", "300", "300"]),
            &[vec![], vec![], vec![]],
            &mut saver(vec![due_in("w", "600", 2)]),
        );
        let v0 = &out[0].verdicts[0];
        assert!(v0.set_aside && !v0.skipped);
        assert_eq!(v0.amount, d("200"));
        assert_eq!(out[0].free_left, d("100"));
        assert_eq!(out[1].verdicts[0].amount, d("200"));
        assert!(out[1].verdicts[0].set_aside);
        let bought = &out[2].verdicts[0];
        assert_eq!((bought.amount, bought.set_aside), (d("200"), false));
    }

    #[test]
    fn set_aside_takes_what_fits_and_spreads_the_rest() {
        // free 100 / 400 / 400; wish 600 due month 2 → 200 a month, but
        // month 0 fits only 100. The 500 left spreads over the two months after.
        let out = judge(
            &bases(["100", "400", "400"]),
            &[vec![], vec![], vec![]],
            &mut saver(vec![due_in("w", "600", 2)]),
        );
        let first = &out[0].verdicts[0];
        assert!(first.set_aside && !first.skipped);
        assert_eq!((first.amount, first.from_free), (d("100"), d("100")));
        assert_eq!(out[0].free_left, Decimal::ZERO);
        assert_eq!(out[1].verdicts[0].amount, d("250"));
        assert_eq!(out[2].verdicts[0].amount, d("250"));
        assert!(out.iter().all(|m| {
            m.verdicts
                .iter()
                .all(|v| v.short.is_zero() && v.on_card.is_zero())
        }));
    }

    #[test]
    fn set_aside_with_nothing_free_is_skipped() {
        // free 0 / 400 / 400; wish 600 due month 2: month 0 sets nothing aside.
        let out = judge(
            &bases(["0", "400", "400"]),
            &[vec![], vec![], vec![]],
            &mut saver(vec![due_in("w", "600", 2)]),
        );
        assert!(out[0].verdicts[0].skipped && out[0].verdicts[0].amount.is_zero());
        assert_eq!(out[1].verdicts[0].amount, d("300"));
        assert_eq!(out[2].verdicts[0].amount, d("300"));
    }

    #[test]
    fn set_aside_never_uses_the_cards() {
        // Same as the partial case but with cards: the trial never touches them.
        let out = judge(
            &bases_with_cards(["100", "400", "400"]),
            &[vec![], vec![], vec![]],
            &mut saver(vec![due_in("w", "600", 2)]),
        );
        assert_eq!(out[0].verdicts[0].amount, d("100"));
        assert_eq!(out[0].card_used, Decimal::ZERO);
    }

    #[test]
    fn must_set_aside_stays_off_the_cards() {
        // free 100 / 250 / 250; must 600 due month 2. Month 0 saves the 100
        // that fits, month 1 saves 250, month 2 buys the last 250: no cards.
        let out = judge(
            &bases_with_cards(["100", "250", "250"]),
            &[vec![], vec![], vec![]],
            &mut saver(vec![due_in_must("w", "600", 2)]),
        );
        assert_eq!(out[0].verdicts[0].amount, d("100"));
        assert_eq!(out[1].verdicts[0].amount, d("250"));
        let bought = &out[2].verdicts[0];
        assert_eq!((bought.amount, bought.set_aside), (d("250"), false));
        assert!(out.iter().all(|m| m.card_used.is_zero()));
    }

    #[test]
    fn purchase_goes_on_the_cards_when_nothing_was_saved() {
        // free 0 then 0; must 100 due month 1. Nothing fits in month 0, so
        // month 1 buys the whole 100 on the cards.
        let out = judge(
            &bases_with_cards(["0", "0"]),
            &[vec![], vec![]],
            &mut saver(vec![due_in_must("w", "100", 1)]),
        );
        assert!(out[0].verdicts[0].skipped);
        assert_eq!(out[0].card_used, Decimal::ZERO);
        let bought = &out[1].verdicts[0];
        assert_eq!(
            (bought.amount, bought.on_card, bought.set_aside),
            (d("100"), d("100"), false)
        );
    }

    #[test]
    fn card_fill_never_exceeds_limit() {
        // visa limit 100 used 30, mc limit 200 used 0; card purchase 250 → visa 70, mc 180; then 400 more → mc 20 then free/cuts/short.
        let out = judge(
            &[base_cards(
                "0",
                vec![("visa", "100", "30"), ("mc", "200", "0")],
            )],
            &[vec![]],
            &mut saver(vec![card_purchase("a", "250"), card_purchase("b", "400")]),
        );
        let m = &out[0];
        assert_eq!(m.cards[0].used, d("100"));
        assert_eq!(m.cards[1].used, d("200"));
        assert_eq!(m.verdicts[0].on_card, d("250"));
        assert_eq!(m.verdicts[1].on_card, d("20"));
        assert_eq!(m.verdicts[1].short, d("380"));
        assert_eq!(m.card_used, d("300"));
    }

    #[test]
    fn next_month_charge_is_the_sum_of_every_card() {
        // month 0 ends visa 60 / mc 90 → month 1 "Credit card" verdict amount 150, paid from free.
        let mut months = months_of(
            ["0", "200"],
            &[card("visa", "100", "60"), card("mc", "200", "90")],
        );
        months[1].cards = two_cards();
        let out = judge(&months, &[vec![], vec![]], &mut no_saver());
        assert_eq!(
            out[0]
                .cards
                .iter()
                .map(|c| (c.card_id.as_str(), c.used))
                .collect::<Vec<_>>(),
            vec![("visa", d("60")), ("mc", d("90"))]
        );
        assert_eq!(out[1].card_charge_in, d("150"));
        let charge = &out[1].verdicts[0];
        assert_eq!(charge.label, CARD_CHARGE);
        assert_eq!(charge.amount, d("150"));
        assert_eq!(charge.from_free, d("150"));
        assert_eq!(out[1].free_left, d("50"));
    }

    #[test]
    fn later_months_start_from_their_card_usage() {
        // Month 1's cards hold 2400 of 2500 between them.
        let mut months = vec![base_at(10, "0", vec![], "0"), base_at(11, "0", vec![], "0")];
        months[1].cards = vec![card("visa", "2000", "2000"), card("mc", "500", "400")];
        let out = judge(
            &months,
            &[vec![], vec![demand("tv", "200", true)]],
            &mut no_saver(),
        );
        let tv = &out[1].verdicts[0];
        assert_eq!(tv.on_card, d("100"));
        assert_eq!(tv.short, d("100"));
        assert_eq!(out[1].card_used, d("2500"));
        assert_eq!(out[1].cards[1].used, d("500"));
    }

    #[test]
    fn card_charge_shortfall_split_by_on_card() {
        // October: 100 typed usage on the visa, a 300 and a 100 wish on the
        // cards. November pays the 500 charge: going out 200 → 0, short 300.
        let mut months = vec![
            base_at(10, "0", vec![], "0"),
            base_at(11, "0", vec![group("going out", 2, "200", "0")], "0"),
        ];
        months[0].cards = vec![card("visa", "100", "100"), card("mc", "2400", "0")];
        let out = judge(
            &months,
            &[
                vec![demand("a", "300", true), demand("b", "100", true)],
                vec![],
            ],
            &mut no_saver(),
        );
        let charge = &out[1].verdicts[0];
        assert_eq!(charge.amount, d("500"));
        assert_eq!(charge.short, d("300"));
        let (a, b) = (&out[0].verdicts[0], &out[0].verdicts[1]);
        assert_eq!(a.next_short, d("180"));
        assert_eq!(b.next_short, d("60"));
        assert_eq!(
            a.next_cuts,
            vec![Cut {
                group_id: "going out".into(),
                name: "going out".into(),
                from: d("200"),
                to: d("80"),
            }]
        );
        assert_eq!(b.next_cuts[0].to, d("160"));
    }

    #[test]
    fn repeats_go_by_tier_after_wishes() {
        // free 100: a freed 20 first, then the must repeat 50 before the
        // optional purchase 70, and the optional repeat 30 last.
        let mut must_cost = demand("gym", "50", false);
        must_cost.must = true;
        let out = judge(
            &[base("100", vec![])],
            &[vec![
                demand("rent", "-20", false),
                must_cost,
                demand("bus", "30", false),
            ]],
            &mut saver(vec![purchase("tv", "70")]),
        );
        let ids: Vec<_> = out[0]
            .verdicts
            .iter()
            .map(|v| v.wish_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, vec!["rent", "gym", "tv", "bus"]);
        assert_eq!(out[0].verdicts[2].short, Decimal::ZERO);
        assert_eq!(out[0].verdicts[3].short, d("30"));
    }
}
