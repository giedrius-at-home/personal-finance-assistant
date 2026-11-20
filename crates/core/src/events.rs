//! Events: wishes with dates. An event owns the day-to-day spending of its
//! days (and what the owner picks in by hand), so those days stop counting
//! in the spending groups. Pure arithmetic, no I/O.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{Wish, WishKind, WishStatus};
use finance_types::report::Currency;
use finance_types::report::events::Pace;
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
