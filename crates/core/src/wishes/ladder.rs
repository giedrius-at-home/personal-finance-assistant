//! The cutting ladder: how each planned purchase is paid for, month by month.
//!
//! Pure arithmetic, no I/O. For every demand the order is: the card first when
//! the wish asked for it, then free money, then cuts to budget groups (least
//! important first, never below their floor), then the card, and what is left
//! is `short`.

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
    pub card_used: Decimal,
}

/// Something to pay for in a month.
#[derive(Clone, Debug)]
pub struct Demand {
    pub wish_id: Option<String>,
    pub label: String,
    pub amount: Decimal,
    pub on_card: bool,
}

/// Judge every demand of every month. `demands[i]` belongs to `months[i]`.
pub fn judge(
    months: &[MonthBaseline],
    demands: &[Vec<Demand>],
    card_limit: Decimal,
) -> Vec<WishMonth> {
    let mut out: Vec<WishMonth> = Vec::with_capacity(months.len());
    for (i, base) in months.iter().enumerate() {
        let card_charge_in = match out.last() {
            Some(prev) if i >= 1 => prev.card_used,
            _ => Decimal::ZERO,
        };
        let mut card_used = if i == 0 {
            base.card_used
        } else {
            Decimal::ZERO
        };
        let mut free = base.free;

        // Groups in the order they are cut; limits carry earlier cuts.
        let mut groups: Vec<(&GroupRoom, Decimal)> =
            base.groups.iter().map(|g| (g, g.limit)).collect();
        groups.sort_by(|a, b| {
            b.0.importance
                .cmp(&a.0.importance)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });

        let mut queue: Vec<Demand> = Vec::new();
        if card_charge_in > Decimal::ZERO {
            queue.push(Demand {
                wish_id: None,
                label: "Credit card".to_string(),
                amount: card_charge_in,
                on_card: false,
            });
        }
        queue.extend(demands.get(i).cloned().unwrap_or_default());

        let mut verdicts = Vec::with_capacity(queue.len());
        for demand in queue {
            if demand.amount < Decimal::ZERO {
                // A cheaper bill: money comes back.
                free -= demand.amount;
                verdicts.push(Verdict {
                    wish_id: demand.wish_id,
                    label: demand.label,
                    amount: demand.amount,
                    from_free: demand.amount,
                    cuts: Vec::new(),
                    on_card: Decimal::ZERO,
                    short: Decimal::ZERO,
                });
                continue;
            }
            let mut rem = demand.amount;
            let mut on_card = Decimal::ZERO;
            if demand.on_card {
                let take = rem.min((card_limit - card_used).max(Decimal::ZERO));
                on_card += take;
                card_used += take;
                rem -= take;
            }
            let from_free = rem.min(free.max(Decimal::ZERO));
            free -= from_free;
            rem -= from_free;

            let mut cuts = Vec::new();
            for (group, current) in groups.iter_mut() {
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

            if !demand.on_card {
                let take = rem.min((card_limit - card_used).max(Decimal::ZERO));
                on_card += take;
                card_used += take;
                rem -= take;
            }

            verdicts.push(Verdict {
                wish_id: demand.wish_id,
                label: demand.label,
                amount: demand.amount,
                from_free,
                cuts,
                on_card,
                short: rem,
            });
        }

        out.push(WishMonth {
            month: format!("{:04}-{:02}-01", base.month.year(), base.month.month()),
            free_before: base.free,
            free_left: free,
            card_charge_in,
            card_used,
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
            card_used: d(card_used),
        }
    }

    fn demand(label: &str, amount: &str, on_card: bool) -> Demand {
        Demand {
            wish_id: Some(label.into()),
            label: label.into(),
            amount: d(amount),
            on_card,
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
    fn unpaid_card_charge_rolls_forward() {
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
        assert_eq!(charge.on_card, d("300"));
        assert_eq!(out[2].card_charge_in, d("300"));
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
