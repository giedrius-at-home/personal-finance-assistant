//! The cutting ladder: how each planned purchase is paid for, month by month.
//!
//! Pure arithmetic, no I/O. For every demand the order is: the card first when
//! the wish asked for it, then free money, then cuts to budget groups (the
//! highest importance number first, never below their floor), then the card,
//! and what is left is `short`.
//!
//! Each month first pays what it already lacks ("Already short", when its
//! free money is negative), then last month's card charge ("Credit card"),
//! which may not go back on the card: what free money and cuts cannot pay is
//! short, and that short and those cuts are traced back to the verdicts that
//! put money on the card the month before.

use finance_types::report::wishes::{Cut, Verdict, WishMonth};
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

/// What a month pays with while its demands are judged.
struct Purse<'g> {
    free: Decimal,
    card_used: Decimal,
    card_limit: Decimal,
    /// Groups in the order they are cut; limits carry earlier cuts.
    groups: Vec<(&'g GroupRoom, Decimal)>,
}

impl Purse<'_> {
    fn card_room(&self) -> Decimal {
        (self.card_limit - self.card_used).max(Decimal::ZERO)
    }

    /// Run one demand down the ladder; `card_allowed` false skips the card.
    fn pay(&mut self, demand: Demand, card_allowed: bool) -> Verdict {
        let verdict = |demand: Demand| Verdict {
            wish_id: demand.wish_id,
            label: demand.label,
            amount: demand.amount,
            from_free: Decimal::ZERO,
            cuts: Vec::new(),
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
            set_aside: demand.set_aside,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
            must: false,
            skipped: false,
        };
        if demand.amount < Decimal::ZERO {
            // A cheaper bill: money comes back.
            self.free -= demand.amount;
            let from_free = demand.amount;
            return Verdict {
                from_free,
                ..verdict(demand)
            };
        }
        let mut rem = demand.amount;
        let mut on_card = Decimal::ZERO;
        if demand.on_card && card_allowed {
            let take = rem.min(self.card_room());
            on_card += take;
            self.card_used += take;
            rem -= take;
        }
        let from_free = rem.min(self.free.max(Decimal::ZERO));
        self.free -= from_free;
        rem -= from_free;

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
            let take = rem.min(self.card_room());
            on_card += take;
            self.card_used += take;
            rem -= take;
        }
        Verdict {
            from_free,
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

/// Judge every demand of every month. `demands[i]` belongs to `months[i]`.
pub fn judge(
    months: &[MonthBaseline],
    demands: &[Vec<Demand>],
    card_limit: Decimal,
) -> Vec<WishMonth> {
    let mut out: Vec<WishMonth> = Vec::with_capacity(months.len());
    for (i, base) in months.iter().enumerate() {
        let card_charge_in = out.last().map_or(Decimal::ZERO, |prev| prev.card_used);
        let mut groups: Vec<(&GroupRoom, Decimal)> =
            base.groups.iter().map(|g| (g, g.limit)).collect();
        groups.sort_by(|a, b| {
            b.0.importance
                .cmp(&a.0.importance)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });
        let mut purse = Purse {
            free: base.free.max(Decimal::ZERO),
            card_used: base.cards.iter().map(|c| c.used).sum(),
            card_limit,
            groups,
        };

        let mut verdicts = Vec::new();
        if base.free < Decimal::ZERO {
            verdicts.push(purse.pay(
                Demand {
                    wish_id: None,
                    label: ALREADY_SHORT.to_string(),
                    amount: -base.free,
                    on_card: false,
                    set_aside: false,
                    must: false,
                },
                true,
            ));
        }
        if card_charge_in > Decimal::ZERO {
            let charge = purse.pay(
                Demand {
                    wish_id: None,
                    label: CARD_CHARGE.to_string(),
                    amount: card_charge_in,
                    on_card: false,
                    set_aside: false,
                    must: false,
                },
                false,
            );
            if let Some(prev) = out.last_mut() {
                trace_charge(prev, &charge);
            }
            verdicts.push(charge);
        }
        for demand in demands.get(i).cloned().unwrap_or_default() {
            verdicts.push(purse.pay(demand, true));
        }

        out.push(WishMonth {
            month: format!("{:04}-{:02}-01", base.month.year(), base.month.month()),
            free_before: base.free,
            free_left: purse.free,
            card_charge_in,
            card_used: purse.card_used,
            cards: Vec::new(),
            verdicts,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn base(free: &str, groups: Vec<GroupRoom>, card_used: &str) -> MonthBaseline {
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
            ..base(free, groups, card_used)
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
            &[base("500", vec![], "0")],
            &[vec![demand("tv", "150", false)]],
            d("2500"),
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
            &[base("0", two_groups(), "0")],
            &[vec![demand("tv", "200", false)]],
            d("2500"),
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
            &[base("0", two_groups(), "9.98"), base("0", vec![], "0")],
            &[vec![demand("tv", "400", false)], vec![]],
            d("2500"),
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
            &[base("500", vec![], "9.98"), base("2000", vec![], "0")],
            &[vec![demand("trip", "900", true)], vec![]],
            d("2500"),
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
            &[base("0", vec![], "2500")],
            &[vec![demand("tv", "50", false)]],
            d("2500"),
        );
        assert_eq!(out[0].verdicts[0].short, d("50"));
    }

    #[test]
    fn wishes_stack_in_order() {
        let out = judge(
            &[base("100", vec![], "0")],
            &[vec![demand("a", "80", false), demand("b", "80", false)]],
            d("2500"),
        );
        assert_eq!(out[0].verdicts[0].from_free, d("80"));
        assert_eq!(out[0].verdicts[1].from_free, d("20"));
        assert_eq!(out[0].verdicts[1].on_card, d("60"));
    }

    #[test]
    fn overspent_group_gives_no_room() {
        let out = judge(
            &[base("0", vec![group("fun", 1, "100", "140")], "0")],
            &[vec![demand("tv", "50", false)]],
            d("2500"),
        );
        let v = &out[0].verdicts[0];
        assert!(v.cuts.is_empty());
        assert_eq!(v.on_card, d("50"));
    }

    #[test]
    fn unpaid_card_charge_is_short_not_rolled() {
        let out = judge(
            &[
                base("0", vec![], "0"),
                base("0", vec![], "0"),
                base("0", vec![], "0"),
            ],
            &[vec![demand("tv", "300", false)], vec![], vec![]],
            d("2500"),
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
            d("2500"),
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
    fn card_charge_shortfall_split_by_on_card() {
        // October: 100 typed usage, a 300 and a 100 wish on the card.
        // November pays the 500 charge: going out 200 → 0, short 300.
        let out = judge(
            &[
                base_at(10, "0", vec![], "100"),
                base_at(11, "0", vec![group("going out", 2, "200", "0")], "0"),
            ],
            &[
                vec![demand("a", "300", true), demand("b", "100", true)],
                vec![],
            ],
            d("2500"),
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
    fn card_charge_paid_from_free_then_cuts() {
        let out = judge(
            &[
                base_at(10, "0", vec![], "0"),
                base_at(11, "50", vec![group("going out", 2, "120", "0")], "0"),
            ],
            &[vec![demand("tv", "100", true)], vec![]],
            d("2500"),
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
            &[base("-50", vec![group("going out", 2, "120", "0")], "0")],
            &[vec![demand("tv", "30", false)]],
            d("2500"),
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
        let out = judge(&[base("-50", vec![], "2500")], &[vec![]], d("2500"));
        assert_eq!(out[0].verdicts[0].short, d("50"));
    }

    #[test]
    fn later_months_start_from_their_card_usage() {
        let out = judge(
            &[
                base_at(10, "0", vec![], "0"),
                base_at(11, "0", vec![], "2400"),
            ],
            &[vec![], vec![demand("tv", "200", true)]],
            d("2500"),
        );
        let tv = &out[1].verdicts[0];
        assert_eq!(tv.on_card, d("100"));
        assert_eq!(tv.short, d("100"));
        assert_eq!(out[1].card_used, d("2500"));
    }

    #[test]
    fn cuts_carry_across_demands_in_a_month() {
        let out = judge(
            &[base("0", two_groups(), "2500")],
            &[vec![demand("a", "100", false), demand("b", "100", false)]],
            d("2500"),
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
            &[base("30", two_groups(), "2400")],
            &[vec![demand("tv", "200", true)]],
            d("2500"),
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
            &[base(
                "0",
                vec![group("b", 2, "10", "0"), group("a", 2, "10", "0")],
                "2500",
            )],
            &[vec![demand("tv", "15", false)]],
            d("2500"),
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
            &[base("100", vec![], "0")],
            &[vec![demand("rent", "-50", false)]],
            d("2500"),
        );
        assert_eq!(out[0].free_left, d("150"));
        let v = &out[0].verdicts[0];
        assert_eq!(v.from_free, d("-50"));
        assert!(v.cuts.is_empty());
        assert_eq!(v.on_card, Decimal::ZERO);
        assert_eq!(v.short, Decimal::ZERO);
    }
}
