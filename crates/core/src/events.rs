//! Events: wishes with dates. An event owns the day-to-day spending of its
//! days (and what the owner picks in by hand), so those days stop counting
//! in the spending groups. Pure arithmetic, no I/O.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{Wish, WishKind, WishStatus};
use finance_types::report::Currency;
use finance_types::report::events::{EventMonth, EventPhase, EventSpend, Pace};
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::period::shift_month;

/// The prefix of an event's row and ownership key: `event:<wish id>`.
pub const KEY: &str = "event:";

/// An event as the budget sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventClaim {
    pub wish_id: String,
    pub name: String,
    /// `event:<wish id>`.
    pub key: String,
    pub currency: Currency,
    pub starts: Date,
    pub ends: Date,
    pub picked: HashSet<String>,
    pub excluded: HashSet<String>,
    /// Transaction id → the bought part it paid for. That money is the
    /// part's and is counted there, once.
    pub tied: HashMap<String, String>,
    /// The planned amount less its planned and bought parts: the money for
    /// the days themselves (and payments picked in ahead).
    pub rest: Decimal,
    pub saved: Decimal,
}

fn day(text: &str) -> Option<Date> {
    text.get(..10)?.parse().ok()
}

/// Planned event wishes, earliest start first, then by wish id.
pub fn claims(wishes: &[Wish]) -> Vec<EventClaim> {
    let mut out: Vec<EventClaim> = wishes
        .iter()
        .filter(|w| {
            w.status == WishStatus::Planned && w.kind == WishKind::OneOff && w.parent_id.is_none()
        })
        .filter_map(|w| {
            let event = w.event.as_ref()?;
            let (starts, ends) = (day(&event.starts_on)?, day(&event.ends_on)?);
            if ends < starts {
                return None;
            }
            let mut rest = w.amount;
            let mut tied = HashMap::new();
            for part in wishes.iter().filter(|p| {
                p.parent_id.as_deref() == Some(&w.wish_id) && p.kind == WishKind::OneOff
            }) {
                match part.status {
                    WishStatus::Planned => rest -= part.amount,
                    WishStatus::Bought => {
                        rest -= part.bought.as_ref().map_or(part.amount, |b| b.amount);
                        if let Some(id) =
                            part.bought.as_ref().and_then(|b| b.transaction_id.clone())
                        {
                            tied.insert(id, part.wish_id.clone());
                        }
                    }
                    WishStatus::Dropped | WishStatus::Started => {}
                }
            }
            Some(EventClaim {
                wish_id: w.wish_id.clone(),
                name: w.name.clone(),
                key: format!("{KEY}{}", w.wish_id),
                currency: w.currency.clone(),
                starts,
                ends,
                picked: event.transaction_ids.iter().cloned().collect(),
                excluded: event.excluded_ids.iter().cloned().collect(),
                tied,
                rest: rest.max(Decimal::ZERO),
                saved: w.saved,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.starts, &a.wish_id).cmp(&(b.starts, &b.wish_id)));
    out
}

/// The event a transaction belongs to: one that picked it by hand, else
/// the first whose days hold it when it is day-to-day spending. An event
/// that excluded it never owns it.
pub fn owner<'a>(
    claims: &'a [EventClaim],
    transaction_id: &str,
    on: Date,
    currency: &Currency,
    day_to_day: bool,
) -> Option<&'a EventClaim> {
    let open = |c: &&EventClaim| c.currency == *currency && !c.excluded.contains(transaction_id);
    claims
        .iter()
        .filter(open)
        .find(|c| c.picked.contains(transaction_id) || c.tied.contains_key(transaction_id))
        .or_else(|| {
            claims
                .iter()
                .filter(open)
                .find(|c| day_to_day && c.starts <= on && on <= c.ends)
        })
}

/// A spending group as an event's future months see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupUsual {
    /// Its usual month.
    pub usual: Decimal,
    /// Its limit: it cannot hand over more than it is given.
    pub limit: Decimal,
}

/// What the groups would usually spend on `days` days of `month`: what a
/// later month's groups hand an event, never more than the event's `share`.
pub fn usual_estimate(groups: &[GroupUsual], month: Date, days: u32, share: Decimal) -> Decimal {
    groups
        .iter()
        .map(|g| {
            usual_share(g.usual, month, days)
                .min(g.limit)
                .max(Decimal::ZERO)
        })
        .sum::<Decimal>()
        .min(share.max(Decimal::ZERO))
}

/// What an event still asks of each calendar month, from the running one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventNeed {
    pub wish_id: String,
    pub months: Vec<(Date, Decimal)>,
}

fn month_day(text: &str) -> Option<Date> {
    day(text).map(|d| d.first_of_month())
}

/// The money for the days: the rest less what was picked in ahead.
pub fn days_money(claim: &EventClaim, spend: Option<&EventSpend>) -> Decimal {
    (claim.rest - picked_ahead(spend)).max(Decimal::ZERO)
}

/// What was picked in ahead; refunds never count for more than they undo.
fn picked_ahead(spend: Option<&EventSpend>) -> Decimal {
    spend
        .map_or(Decimal::ZERO, |s| s.picked_ahead)
        .max(Decimal::ZERO)
}

/// Everything spent on the days so far, month by month never below zero.
fn spent_on_days(spend: Option<&EventSpend>) -> Decimal {
    spend.map_or(Decimal::ZERO, |s| {
        s.by_month
            .iter()
            .filter_map(|(m, _)| month_day(m))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|month| spent_in(spend, month))
            .sum()
    })
}

/// What `spend` counts in the budget month starting on `month`; a month
/// of refunds counts as nothing spent, not as money gained.
pub fn spent_in(spend: Option<&EventSpend>, month: Date) -> Decimal {
    spend
        .map_or(Decimal::ZERO, |s| {
            s.by_month
                .iter()
                .filter(|(m, _)| month_day(m) == Some(month))
                .map(|(_, n)| *n)
                .sum()
        })
        .max(Decimal::ZERO)
}

/// Each month's share less what is spent of it. The running month's groups
/// have already handed over their usual days (the budget lowers their
/// reserves); a later month's have not, so their usual days come off here.
/// An event that is over asks for nothing.
pub fn need(
    claim: &EventClaim,
    spend: Option<&EventSpend>,
    groups: &[GroupUsual],
    current_month: Date,
    today: Date,
) -> EventNeed {
    let mut months: Vec<(Date, Decimal)> = if today > claim.ends {
        Vec::new()
    } else {
        shares(claim, days_money(claim, spend))
            .into_iter()
            .filter(|(month, _)| *month >= current_month)
            .map(|(month, share)| {
                let usual = if month > current_month {
                    usual_estimate(groups, month, days_in(claim, month), share)
                } else {
                    Decimal::ZERO
                };
                let left = share - spent_in(spend, month) - usual;
                (month, left.max(Decimal::ZERO))
            })
            .collect()
    };
    // All together never more than is left of the event: an overrun in
    // one month comes off the months after it, the last one first.
    let left = (days_money(claim, spend) - spent_on_days(spend)).max(Decimal::ZERO);
    let mut over = months.iter().map(|(_, n)| *n).sum::<Decimal>() - left;
    for (_, need) in months.iter_mut().rev() {
        if over <= Decimal::ZERO {
            break;
        }
        let cut = over.min(*need);
        *need -= cut;
        over -= cut;
    }
    EventNeed {
        wish_id: claim.wish_id.clone(),
        months,
    }
}

/// What an event comes to on `today`, apart from the ladder's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFacts {
    pub phase: EventPhase,
    pub usual: Decimal,
    pub extra: Decimal,
    pub paid_ahead: Decimal,
    pub spent: Decimal,
    pub left: Decimal,
    pub days: u32,
    pub days_left: u32,
    pub per_day: Decimal,
    pub pace: Pace,
    pub months: Vec<EventMonth>,
}

/// `bought_parts` is what the event's bought parts cost; `usual` is what
/// the groups hand over (or would) in each month.
pub fn facts(
    claim: &EventClaim,
    spend: Option<&EventSpend>,
    bought_parts: Decimal,
    usual: &[(Date, Decimal)],
    today: Date,
) -> EventFacts {
    let money = days_money(claim, spend);
    let months: Vec<EventMonth> = shares(claim, money)
        .into_iter()
        .map(|(month, share)| EventMonth {
            month: month.to_string(),
            share,
            spent: spent_in(spend, month),
            usual: usual
                .iter()
                .filter(|(m, _)| *m == month)
                .map(|(_, n)| *n)
                .sum(),
        })
        .collect();
    let picked_ahead = picked_ahead(spend);
    let on_days = spent_on_days(spend);
    let left = claim.rest - picked_ahead - on_days;
    let days = days(claim);
    let (phase, days_left, days_gone) = if today < claim.starts {
        (EventPhase::Upcoming, 0, 0)
    } else if today > claim.ends {
        (EventPhase::Past, 0, days)
    } else {
        (
            EventPhase::Running,
            span_days(today, claim.ends),
            span_days(claim.starts, today),
        )
    };
    let per_day = if days_left > 0 && left > Decimal::ZERO {
        (left / Decimal::from(days_left)).round_dp(2)
    } else {
        Decimal::ZERO
    };
    let usual_total: Decimal = months.iter().map(|m| m.usual).sum();
    EventFacts {
        phase,
        usual: usual_total,
        extra: (money - usual_total).max(Decimal::ZERO),
        paid_ahead: bought_parts + picked_ahead,
        spent: bought_parts + picked_ahead + on_days,
        left,
        days,
        days_left,
        per_day,
        pace: pace(money, on_days, days_gone, days, left < Decimal::ZERO),
        months,
    }
}

fn span_days(from: Date, to: Date) -> u32 {
    u32::try_from((to - from).get_days() + 1).unwrap_or(0)
}

/// The event's days, first and last included.
pub fn days(claim: &EventClaim) -> u32 {
    span_days(claim.starts, claim.ends)
}

/// The event's days inside the calendar month starting on `month`.
pub fn days_in(claim: &EventClaim, month: Date) -> u32 {
    let from = claim.starts.max(month.first_of_month());
    let to = claim.ends.min(month.last_of_month());
    if to < from { 0 } else { span_days(from, to) }
}

/// The calendar months the event touches, as their first days.
pub fn months(claim: &EventClaim) -> Vec<Date> {
    let mut out = Vec::new();
    let mut month = claim.starts.first_of_month();
    while month <= claim.ends {
        out.push(month);
        month = shift_month(month, 1);
    }
    out
}

/// `days_money` split over the months by the event's days in each; the
/// last month takes the rounding.
pub fn shares(claim: &EventClaim, days_money: Decimal) -> Vec<(Date, Decimal)> {
    let total = Decimal::from(days(claim).max(1));
    let months = months(claim);
    let mut left = days_money;
    months
        .iter()
        .enumerate()
        .map(|(i, month)| {
            let share = if i + 1 == months.len() {
                left
            } else {
                (days_money * Decimal::from(days_in(claim, *month)) / total).round_dp(2)
            };
            left -= share;
            (*month, share)
        })
        .collect()
}

/// What a group that usually spends `usual_month` spends on `event_days`
/// days of `month`.
pub fn usual_share(usual_month: Decimal, month: Date, event_days: u32) -> Decimal {
    let in_month = Decimal::from(month.days_in_month());
    (usual_month * Decimal::from(event_days) / in_month).round_dp(2)
}

/// How the spending on the days stands against the days gone. `over` is
/// the event as a whole having spent more than planned.
pub fn pace(
    days_money: Decimal,
    spent_on_days: Decimal,
    days_gone: u32,
    days: u32,
    over: bool,
) -> Pace {
    if over {
        return Pace::Over;
    }
    let allowed = days_money * Decimal::from(days_gone) / Decimal::from(days.max(1));
    if spent_on_days > allowed {
        Pace::Fast
    } else {
        Pace::OnTrack
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use finance_types::gateway::{EventDetails, WishPurchase, WishSource};
    use jiff::civil::date;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn eur() -> Currency {
        Currency::new("EUR")
    }

    pub(crate) fn claim(starts: &str, ends: &str) -> EventClaim {
        EventClaim {
            wish_id: "e".into(),
            name: "Parents".into(),
            key: "event:e".into(),
            currency: eur(),
            starts: starts.parse().unwrap(),
            ends: ends.parse().unwrap(),
            picked: HashSet::new(),
            excluded: HashSet::new(),
            tied: HashMap::new(),
            rest: d("400"),
            saved: Decimal::ZERO,
        }
    }

    pub(crate) fn wish(id: &str, parent: Option<&str>, amount: &str) -> Wish {
        Wish {
            wish_id: id.into(),
            parent_id: parent.map(str::to_owned),
            name: id.into(),
            amount: d(amount),
            currency: eur(),
            due_on: "2026-11-30".into(),
            source: WishSource::Card,
            saved: Decimal::ZERO,
            saved_month: None,
            status: WishStatus::Planned,
            bought: None,
            kind: WishKind::OneOff,
            must: false,
            event: None,
            created_at: "x".into(),
        }
    }

    pub(crate) fn event(id: &str, amount: &str, starts: &str, ends: &str) -> Wish {
        Wish {
            event: Some(EventDetails {
                starts_on: starts.into(),
                ends_on: ends.into(),
                ..EventDetails::default()
            }),
            ..wish(id, None, amount)
        }
    }

    fn spend(picked_ahead: &str, by_month: &[(&str, &str)]) -> EventSpend {
        EventSpend {
            wish_id: "e".into(),
            picked_ahead: d(picked_ahead),
            by_month: by_month
                .iter()
                .map(|(m, n)| ((*m).to_owned(), d(n)))
                .collect(),
            transactions: Vec::new(),
        }
    }

    #[test]
    fn an_event_asks_each_month_for_its_share_less_what_is_spent() {
        let e = claim("2026-11-30", "2026-12-05");
        let (nov, dec) = (date(2026, 11, 1), date(2026, 12, 1));
        // Nothing spent, judged in November: the running month's groups
        // hand over their usual share themselves, later months net it here.
        let groups = [GroupUsual {
            usual: d("310"),
            limit: d("300"),
        }];
        assert_eq!(
            need(&e, None, &groups, nov, date(2026, 11, 20)).months,
            vec![(nov, d("66.67")), (dec, d("283.33"))]
        );
        // December runs: €100 spent of its share.
        let s = spend("0", &[("2026-12-01", "100")]);
        assert_eq!(
            need(&e, Some(&s), &groups, dec, date(2026, 12, 2)).months,
            vec![(dec, d("233.33"))]
        );
        // Paid ahead comes off the money for the days.
        let s = spend("46", &[]);
        assert_eq!(
            need(&e, Some(&s), &[], nov, date(2026, 11, 20)).months,
            vec![(nov, d("59")), (dec, d("295"))]
        );
        // Spent more than the share: nothing more is asked, never less than zero.
        let s = spend("0", &[("2026-12-01", "500")]);
        assert_eq!(
            need(&e, Some(&s), &groups, dec, date(2026, 12, 2)).months,
            vec![(dec, d("0"))]
        );
        // Over: it asks for nothing.
        assert!(
            need(&e, None, &groups, dec, date(2026, 12, 6))
                .months
                .is_empty()
        );
    }

    #[test]
    fn a_running_event_knows_what_is_spent_left_and_allowed_a_day() {
        // €400 with Zoo (18) still to buy and Naturalis bought for 28.
        let e = EventClaim {
            rest: d("354"),
            ..claim("2026-11-30", "2026-12-05")
        };
        let s = spend("0", &[("2026-11-01", "100"), ("2026-12-01", "87.60")]);
        let usual = [(date(2026, 12, 1), d("50"))];
        let f = facts(&e, Some(&s), d("28"), &usual, date(2026, 12, 2));
        assert_eq!(f.phase, EventPhase::Running);
        assert_eq!((f.spent, f.left), (d("215.60"), d("166.40")));
        assert_eq!((f.days, f.days_left, f.per_day), (6, 4, d("41.60")));
        assert_eq!(
            (f.usual, f.extra, f.paid_ahead),
            (d("50"), d("304"), d("28"))
        );
        // Three days of six gone: €177 allowed, €187.60 spent.
        assert_eq!(f.pace, Pace::Fast);
        assert_eq!(f.months.len(), 2);
        assert_eq!(
            (f.months[1].share, f.months[1].spent),
            (d("295"), d("87.60"))
        );
        // Before it starts and after it ends there is no day to spend on.
        let before = facts(&e, None, d("28"), &usual, date(2026, 11, 20));
        assert_eq!((before.phase, before.days_left), (EventPhase::Upcoming, 0));
        assert_eq!(before.per_day, d("0"));
        let after = facts(&e, Some(&s), d("28"), &usual, date(2026, 12, 6));
        assert_eq!((after.phase, after.days_left), (EventPhase::Past, 0));
        // Over the plan: nothing left a day, and it says so.
        let much = spend("0", &[("2026-12-01", "500")]);
        let over = facts(&e, Some(&much), d("28"), &usual, date(2026, 12, 2));
        assert_eq!((over.pace, over.per_day), (Pace::Over, d("0")));
        assert_eq!(over.left, d("-146"));
    }

    #[test]
    fn an_overrun_in_one_month_comes_off_what_the_next_is_asked() {
        // €150 spent on 30 November against a €66.67 share: €250 is left,
        // and December is asked for that, not for its whole €333.33.
        let e = claim("2026-11-30", "2026-12-05");
        let dec = date(2026, 12, 1);
        let s = spend("0", &[("2026-11-01", "150")]);
        assert_eq!(
            need(&e, Some(&s), &[], dec, date(2026, 12, 1)).months,
            vec![(dec, d("250"))]
        );
    }

    #[test]
    fn a_refund_never_makes_an_event_ask_for_more_than_its_share() {
        let e = claim("2026-11-30", "2026-12-05");
        let dec = date(2026, 12, 1);
        let s = spend("-40", &[("2026-12-01", "-60")]);
        assert_eq!(
            need(&e, Some(&s), &[], dec, date(2026, 12, 2)).months,
            vec![(dec, d("333.33"))]
        );
        let f = facts(&e, Some(&s), d("0"), &[], date(2026, 12, 2));
        assert_eq!((f.left, f.spent), (d("400"), d("0")));
    }

    #[test]
    fn the_usual_days_never_exceed_a_groups_limit_or_the_events_share() {
        let groups = [
            GroupUsual {
                usual: d("310"),
                limit: d("300"),
            },
            GroupUsual {
                usual: d("62"),
                limit: d("0"),
            },
        ];
        // 310 × 5 ÷ 31 = 50 from the first; the second has no limit to give.
        assert_eq!(
            usual_estimate(&groups, date(2026, 12, 1), 5, d("333.33")),
            d("50")
        );
        assert_eq!(
            usual_estimate(&groups, date(2026, 12, 1), 5, d("20")),
            d("20")
        );
    }

    #[test]
    fn the_days_split_over_the_months() {
        let e = claim("2026-11-30", "2026-12-05");
        assert_eq!(days(&e), 6);
        assert_eq!(days_in(&e, date(2026, 11, 1)), 1);
        assert_eq!(days_in(&e, date(2026, 12, 1)), 5);
        assert_eq!(days_in(&e, date(2027, 1, 1)), 0);
        assert_eq!(
            shares(&e, d("400")),
            vec![
                (date(2026, 11, 1), d("66.67")),
                (date(2026, 12, 1), d("333.33"))
            ]
        );
    }

    #[test]
    fn a_group_hands_over_its_usual_days() {
        // €300 a usual December, five days of 31.
        assert_eq!(usual_share(d("300"), date(2026, 12, 1), 5), d("48.39"));
    }

    #[test]
    fn a_dated_day_to_day_purchase_is_the_events_unless_excluded() {
        let mut e = claim("2026-11-30", "2026-12-05");
        let own = |e: &EventClaim, on: Date, day_to_day: bool| {
            owner(std::slice::from_ref(e), "t", on, &eur(), day_to_day).is_some()
        };
        assert!(own(&e, date(2026, 12, 2), true));
        // A bill on those days stays a bill.
        assert!(!own(&e, date(2026, 12, 2), false));
        assert!(!own(&e, date(2026, 12, 6), true));
        assert!(
            owner(
                &[e.clone()],
                "t",
                date(2026, 12, 2),
                &Currency::new("USD"),
                true
            )
            .is_none()
        );
        // Hand-picked: any date, any kind.
        e.picked.insert("t".into());
        assert!(own(&e, date(2026, 1, 12), false));
        e.excluded.insert("t".into());
        assert!(!own(&e, date(2026, 12, 2), true));
    }

    #[test]
    fn overlap_goes_to_a_pick_then_the_earlier_start() {
        let first = claim("2026-11-30", "2026-12-05");
        let mut second = EventClaim {
            wish_id: "f".into(),
            ..claim("2026-12-02", "2026-12-10")
        };
        let both = |second: &EventClaim| vec![first.clone(), second.clone()];
        let on = date(2026, 12, 3);
        assert_eq!(
            owner(&both(&second), "t", on, &eur(), true)
                .unwrap()
                .wish_id,
            "e"
        );
        second.picked.insert("t".into());
        assert_eq!(
            owner(&both(&second), "t", on, &eur(), true)
                .unwrap()
                .wish_id,
            "f"
        );
    }

    #[test]
    fn pace_is_on_the_money_for_the_days() {
        // Two days of six gone: €133.33 of €400 allowed.
        assert_eq!(pace(d("400"), d("100"), 2, 6, false), Pace::OnTrack);
        assert_eq!(pace(d("400"), d("200"), 2, 6, false), Pace::Fast);
        assert_eq!(pace(d("400"), d("100"), 2, 6, true), Pace::Over);
    }

    #[test]
    fn claims_come_from_planned_event_wishes_with_their_rest_and_ties() {
        let parents = event("e", "400", "2026-11-30", "2026-12-05");
        let zoo = wish("zoo", Some("e"), "18");
        let naturalis = Wish {
            status: WishStatus::Bought,
            bought: Some(WishPurchase {
                amount: d("28"),
                on: "2026-11-20".into(),
                source: WishSource::Card,
                cards: Vec::new(),
                transaction_id: Some("n1".into()),
            }),
            ..wish("naturalis", Some("e"), "30")
        };
        let dropped = Wish {
            status: WishStatus::Dropped,
            ..event("gone", "100", "2026-10-01", "2026-10-02")
        };
        let backwards = event("bad", "100", "2026-12-05", "2026-11-30");
        let earlier = event("a", "50", "2026-11-01", "2026-11-02");
        let got = claims(&[
            parents,
            zoo,
            naturalis,
            dropped,
            backwards,
            earlier,
            wish("w", None, "9"),
        ]);
        assert_eq!(
            got.iter().map(|c| c.wish_id.as_str()).collect::<Vec<_>>(),
            vec!["a", "e"]
        );
        assert_eq!(got[1].rest, d("354"));
        assert_eq!(got[1].tied.get("n1").map(String::as_str), Some("naturalis"));
        assert_eq!(got[1].key, "event:e");
    }

    #[test]
    fn a_payment_tied_to_a_bought_part_belongs_to_the_event() {
        let mut e = claim("2026-11-30", "2026-12-05");
        e.tied.insert("n1".into(), "naturalis".into());
        assert!(owner(&[e], "n1", date(2026, 11, 20), &eur(), false).is_some());
    }
}
