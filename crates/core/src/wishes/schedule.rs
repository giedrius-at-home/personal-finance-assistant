//! From stored wishes to what must be paid in each month.
//!
//! Pure arithmetic, no I/O. A wish is saved for as fast as the months allow,
//! soonest due first, and whatever is still unsaved is the purchase itself.

use super::ladder::Demand;
use crate::events::EventNeed;
use crate::outlook::days_between;
use crate::period::shift_month;
use finance_types::gateway::{Repeat, Wish, WishKind, WishSource, WishStatus};
use jiff::Span;
use jiff::civil::Date;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// One purchase still to be paid for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub wish_id: String,
    pub label: String,
    pub due: Date,
    pub remaining: Decimal,
    pub on_card: bool,
    pub created_at: String,
    /// Repeat, last day, and amount per occurrence (a bill change's delta).
    /// `due` is the first occurrence.
    pub per_month: Option<(Repeat, Option<Date>, Decimal)>,
    /// Saving was recorded in month 0, so month 0 sets nothing more aside.
    pub saved_this_month: bool,
    /// Must be bought: a sub-wish or rest is must when its parent is.
    pub must: bool,
}

/// A bill as it stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BillNow {
    pub amount: Decimal,
    pub every: Repeat,
}

/// A parent wish: what is left of its cap after its sub-wishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub parent_id: String,
    pub rest: Decimal,
    pub over_cap: bool,
}

/// The card statement `date` is on, as the first of its month: a statement
/// runs from the 26th to the 25th of the next month and is named by the
/// month it ends in, so 27 Oct is on November's statement (26 Oct to 25 Nov,
/// charged at the start of December).
pub fn statement_month(date: Date) -> Date {
    let month = date.first_of_month();
    if date.day() >= 26 {
        shift_month(month, 1)
    } else {
        month
    }
}

impl Planned {
    /// Whole months from `current_month` to the month the item is paid in.
    pub fn months_from(&self, current_month: Date) -> usize {
        months_ahead(self.month(), current_month)
    }

    /// The month the item is paid in: a card item's statement month, else
    /// the calendar month it is due in. A card item due before today is
    /// moved to today first (see `Inputs`), so it never lands on a closed
    /// statement.
    fn month(&self) -> Date {
        if self.on_card {
            statement_month(self.due)
        } else {
            self.due.first_of_month()
        }
    }
}

fn month_index(date: Date) -> i32 {
    i32::from(date.year()) * 12 + i32::from(date.month())
}

/// Whole months from `current_month` to the month of `due` (never negative).
fn months_ahead(due: Date, current_month: Date) -> usize {
    usize::try_from(month_index(due) - month_index(current_month)).unwrap_or(0)
}

/// Whether `w`'s saving was recorded in `current_month`.
fn saved_in(w: &Wish, current_month: Date) -> bool {
    w.saved_month
        .as_deref()
        .and_then(|m| m.parse::<Date>().ok())
        .is_some_and(|m| m.first_of_month() == current_month.first_of_month())
}

fn leaf(w: &Wish, label: String, remaining: Decimal, current_month: Date, must: bool) -> Planned {
    let due = w.due_on.parse::<Date>().unwrap_or(current_month);
    Planned {
        wish_id: w.wish_id.clone(),
        label,
        due: due.max(current_month),
        remaining: remaining.max(Decimal::ZERO),
        on_card: matches!(w.source, WishSource::Card),
        created_at: w.created_at.clone(),
        per_month: None,
        saved_this_month: saved_in(w, current_month),
        must,
    }
}

/// A recurring wish or bill change as one planned item; `None` when it
/// cannot be judged (unknown bill, bad date).
fn repeating(w: &Wish, bills: &HashMap<String, BillNow>) -> Option<Planned> {
    let (every, ends_on, amount, on_card) = match &w.kind {
        WishKind::OneOff => return None,
        WishKind::Recurring { every, ends_on } => (
            *every,
            ends_on,
            w.amount,
            matches!(w.source, WishSource::Card),
        ),
        // The bill keeps however it is paid today.
        WishKind::BillChange { bill_id, ends_on } => {
            let bill = bills.get(bill_id)?;
            (bill.every, ends_on, w.amount - bill.amount, false)
        }
    };
    let due = w.due_on.parse::<Date>().ok()?;
    Some(Planned {
        wish_id: w.wish_id.clone(),
        label: w.name.clone(),
        due,
        remaining: Decimal::ZERO,
        on_card,
        created_at: w.created_at.clone(),
        saved_this_month: false,
        must: w.must,
        per_month: Some((
            every,
            ends_on.as_deref().and_then(|e| e.parse::<Date>().ok()),
            amount,
        )),
    })
}

/// How many times something starting on `first` and repeating `every` falls
/// in the month of `month`, up to and including `ends_on`.
pub fn occurrences(first: Date, every: Repeat, ends_on: Option<Date>, month: Date) -> u32 {
    let from = month.first_of_month();
    occurrences_between(first, every, ends_on, from, from.last_of_month())
}

/// How many times something starting on `first` and repeating `every` falls
/// on the card statement named `month` (26th of the month before to the 25th).
fn statement_occurrences(first: Date, every: Repeat, ends_on: Option<Date>, month: Date) -> u32 {
    let month = month.first_of_month();
    // Every month has a 25th and a 26th; only a month before year -9999 has
    // no month before it, and the look-ahead never goes back.
    let from = month
        .checked_sub(Span::new().months(1))
        .and_then(|m| m.with().day(26).build())
        .expect("the month before a statement has a 26th");
    let to = month
        .with()
        .day(25)
        .build()
        .expect("every month has a 25th");
    occurrences_between(first, every, ends_on, from, to)
}

/// How many times something starting on `first` and repeating `every` falls
/// between `from` and `to` (both included, within two calendar months), up
/// to and including `ends_on`.
fn occurrences_between(
    first: Date,
    every: Repeat,
    ends_on: Option<Date>,
    from: Date,
    to: Date,
) -> u32 {
    let start = from.max(first);
    let end = ends_on.map_or(to, |e| e.min(to));
    if start > end {
        return 0;
    }
    let step = match every {
        Repeat::Weekly => 7,
        Repeat::Biweekly => 14,
        Repeat::Monthly | Repeat::Yearly => {
            let (a, b) = (from.first_of_month(), to.first_of_month());
            let months: &[Date] = if a == b { &[a] } else { &[a, b] };
            let hits = months
                .iter()
                .filter(|m| every == Repeat::Monthly || m.month() == first.month())
                .filter_map(|m| {
                    let day = first.day().min(m.days_in_month());
                    m.with().day(day).build().ok()
                })
                .filter(|on| *on >= start && *on <= end)
                .count();
            return u32::try_from(hits).unwrap_or(0);
        }
    };
    let skipped = days_between(first, start).div_euclid(step)
        + i64::from(days_between(first, start).rem_euclid(step) != 0);
    let upto = days_between(first, end).div_euclid(step);
    u32::try_from((upto - skipped + 1).max(0)).unwrap_or(0)
}

/// The wish a verdict or item id belongs to: an event's month share is
/// `<wish id>#<YYYY-MM>`.
pub fn base_id(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

/// Every planned purchase, plus what is left of each parent's cap.
pub fn plan_items(
    wishes: &[Wish],
    current_month: Date,
    bills: &HashMap<String, BillNow>,
) -> (Vec<Planned>, Vec<Family>) {
    plan_items_with(wishes, current_month, bills, &[])
}

/// One item per month an event still asks money of, the money saved for
/// it paying the earliest month first.
fn event_items(w: &Wish, need: &EventNeed, current_month: Date) -> Vec<Planned> {
    let starts = w
        .event
        .as_ref()
        .and_then(|e| e.starts_on.parse::<Date>().ok())
        .unwrap_or(current_month);
    let mut saved = w.saved;
    need.months
        .iter()
        .filter_map(|(month, amount)| {
            let paid = saved.min(*amount);
            saved -= paid;
            let remaining = *amount - paid;
            (remaining > Decimal::ZERO).then(|| Planned {
                wish_id: format!("{}#{}", w.wish_id, &month.to_string()[..7]),
                label: w.name.clone(),
                due: starts.max(*month).max(current_month),
                remaining,
                on_card: false,
                created_at: w.created_at.clone(),
                per_month: None,
                saved_this_month: saved_in(w, current_month),
                must: w.must,
            })
        })
        .collect()
}

/// [`plan_items`], with what each event still asks month by month: an
/// event's own money (its rest) is those months, not one purchase.
pub fn plan_items_with(
    wishes: &[Wish],
    current_month: Date,
    bills: &HashMap<String, BillNow>,
    needs: &[EventNeed],
) -> (Vec<Planned>, Vec<Family>) {
    let need_of = |w: &Wish| {
        w.event
            .as_ref()
            .and_then(|_| needs.iter().find(|n| n.wish_id == w.wish_id))
    };
    let mut children: HashMap<&str, Vec<&Wish>> = HashMap::new();
    for w in wishes {
        if let Some(p) = w.parent_id.as_deref() {
            children.entry(p).or_default().push(w);
        }
    }
    // Repeating wishes are always leaves, whatever parent they were stored with.
    let mut items: Vec<Planned> = wishes
        .iter()
        .filter(|w| w.status == WishStatus::Planned)
        .filter_map(|w| repeating(w, bills))
        .collect();
    let one_off = |w: &Wish| w.kind == WishKind::OneOff;
    let mut families = Vec::new();
    for w in wishes
        .iter()
        .filter(|w| w.parent_id.is_none() && one_off(w))
    {
        if w.status != WishStatus::Planned {
            continue;
        }
        let subs: Vec<&Wish> = children
            .get(w.wish_id.as_str())
            .map(|v| v.iter().copied().filter(|s| one_off(s)).collect())
            .unwrap_or_default();
        if subs.is_empty() {
            if let Some(need) = need_of(w) {
                items.extend(event_items(w, need, current_month));
                continue;
            }
            items.push(leaf(
                w,
                w.name.clone(),
                w.amount - w.saved,
                current_month,
                w.must,
            ));
            continue;
        }
        let mut rest = w.amount;
        for sub in subs {
            rest -= super::part_cost(w, sub);
            if sub.status == WishStatus::Planned {
                items.push(leaf(
                    sub,
                    sub.name.clone(),
                    sub.amount - sub.saved,
                    current_month,
                    sub.must || w.must,
                ));
            }
        }
        let over_cap = rest < Decimal::ZERO;
        let rest = rest.max(Decimal::ZERO);
        families.push(Family {
            parent_id: w.wish_id.clone(),
            rest,
            over_cap,
        });
        if let Some(need) = need_of(w) {
            items.extend(event_items(w, need, current_month));
            continue;
        }
        items.push(leaf(
            w,
            format!("{}: rest", w.name),
            rest - w.saved,
            current_month,
            w.must,
        ));
    }
    (items, families)
}

/// Recurring wishes and bill changes in each of `months` months from
/// `current_month`: money a cheaper bill frees comes first, then costs.
pub fn repeat_demands(items: &[Planned], current_month: Date, months: usize) -> Vec<Vec<Demand>> {
    (0..months)
        .map(|i| {
            let month = shift_month(current_month, i32::try_from(i).unwrap_or(0));
            let (freed, costs): (Vec<_>, Vec<_>) = items
                .iter()
                .filter_map(|item| {
                    let (every, ends_on, each) = item.per_month?;
                    let count = if item.on_card {
                        statement_occurrences(item.due, every, ends_on, month)
                    } else {
                        occurrences(item.due, every, ends_on, month)
                    };
                    let amount = Decimal::from(count) * each;
                    (amount != Decimal::ZERO).then(|| Demand {
                        wish_id: Some(item.wish_id.clone()),
                        label: item.label.clone(),
                        amount,
                        on_card: item.on_card,
                        set_aside: false,
                        must: item.must,
                    })
                })
                .partition(|d| d.amount < Decimal::ZERO);
            freed.into_iter().chain(costs).collect()
        })
        .collect()
}

/// One one-off purchase being saved for.
#[derive(Clone, Debug)]
struct Saving {
    item: Planned,
    due_idx: usize,
    left: Decimal,
}

/// Hands out each month's set-asides from what is still unsaved, so a
/// skipped month spreads its share over the months left.
#[derive(Clone, Debug)]
pub struct Saver {
    savings: Vec<Saving>,
}

impl Saver {
    /// One saving per one-off item still to pay and due within `months`.
    pub fn new(items: &[Planned], current_month: Date, months: usize) -> Self {
        let mut savings: Vec<Saving> = items
            .iter()
            .filter(|i| i.per_month.is_none() && i.remaining > Decimal::ZERO)
            .map(|i| Saving {
                item: i.clone(),
                due_idx: months_ahead(i.month(), current_month),
                left: i.remaining,
            })
            .filter(|s| s.due_idx < months)
            .collect();
        // The soonest wish is saved for first; a later one gets what is left.
        savings.sort_by(|a, b| {
            (a.item.due, &a.item.created_at).cmp(&(b.item.due, &b.item.created_at))
        });
        Self { savings }
    }

    fn find(&self, wish_id: &str) -> Option<&Saving> {
        self.savings.iter().find(|s| s.item.wish_id == wish_id)
    }

    /// Set-asides this month, soonest due first: one per item due after
    /// `month`, asking for its whole unsaved rest (the ladder keeps what
    /// fits). Saving recorded this month means month 0 sets nothing more
    /// aside.
    pub fn set_asides(&self, month: usize) -> Vec<Demand> {
        self.savings
            .iter()
            .filter(|s| s.due_idx > month && s.left > Decimal::ZERO)
            .filter(|s| !(month == 0 && s.item.saved_this_month))
            .map(|s| Demand {
                wish_id: Some(s.item.wish_id.clone()),
                label: format!("Saving for {}", s.item.label),
                amount: s.left,
                on_card: false,
                set_aside: true,
                must: s.item.must,
            })
            .collect()
    }

    /// Purchases due in `month`: each item's whole unsaved rest, by due date
    /// then creation. Overdue items are already due in month 0.
    pub fn purchases(&self, month: usize) -> Vec<Demand> {
        let mut due: Vec<&Saving> = self
            .savings
            .iter()
            .filter(|s| s.due_idx == month && s.left > Decimal::ZERO)
            .collect();
        due.sort_by(|a, b| (a.item.due, &a.item.created_at).cmp(&(b.item.due, &b.item.created_at)));
        due.into_iter()
            .map(|s| Demand {
                wish_id: Some(s.item.wish_id.clone()),
                label: s.item.label.clone(),
                amount: s.left,
                on_card: s.item.on_card,
                set_aside: false,
                must: s.item.must,
            })
            .collect()
    }

    /// Record that `amount` of `wish_id` was paid (set-aside or purchase).
    pub fn paid(&mut self, wish_id: &str, amount: Decimal) {
        if let Some(s) = self.savings.iter_mut().find(|s| s.item.wish_id == wish_id) {
            s.left = (s.left - amount).max(Decimal::ZERO);
        }
    }

    /// What is still unsaved for `wish_id` (zero when not being saved for).
    pub fn remaining(&self, wish_id: &str) -> Decimal {
        self.find(wish_id).map_or(Decimal::ZERO, |s| s.left)
    }
}

/// The most months the look-ahead covers.
pub const MAX_MONTHS: usize = 60;

/// Months to look ahead: through the latest due month, 2 to 60, and at least
/// 12 when anything recurs.
pub fn horizon(items: &[Planned], current_month: Date) -> usize {
    let latest = items
        .iter()
        .map(|i| {
            // A card item is charged the month after its statement.
            let ahead = months_ahead(i.month(), current_month) + 1 + usize::from(i.on_card);
            if i.per_month.is_some() {
                ahead.max(12)
            } else {
                ahead
            }
        })
        .max()
        .unwrap_or(0);
    latest.clamp(2, MAX_MONTHS)
}

/// [`earmarked`], an event holding only what it still asks.
pub fn earmarked_with(wishes: &[Wish], needs: &[EventNeed]) -> Decimal {
    // An event is never "bought", so what it has spent would stay held for
    // good: release what is saved beyond its months' needs and its parts.
    let released: Decimal = wishes
        .iter()
        .filter(|w| {
            w.status == WishStatus::Planned && w.kind == WishKind::OneOff && w.parent_id.is_none()
        })
        .filter_map(|w| {
            let need = w
                .event
                .as_ref()
                .and_then(|_| needs.iter().find(|n| n.wish_id == w.wish_id))?;
            let parts: Decimal = wishes
                .iter()
                .filter(|p| {
                    p.parent_id.as_deref() == Some(&w.wish_id)
                        && p.status == WishStatus::Planned
                        && p.kind == WishKind::OneOff
                })
                .map(|p| (p.amount - p.saved).max(Decimal::ZERO))
                .sum();
            let asked = need.months.iter().map(|(_, n)| *n).sum::<Decimal>() + parts;
            Some((w.saved - asked).max(Decimal::ZERO))
        })
        .sum();
    earmarked(wishes) - released
}

/// Money already set aside for planned wishes. Parts of a parent that is no
/// longer planned (or gone) are not judged, so their savings are not held.
pub fn earmarked(wishes: &[Wish]) -> Decimal {
    let parent_planned = |id: &str| {
        wishes
            .iter()
            .any(|p| p.wish_id == id && p.status == WishStatus::Planned)
    };
    wishes
        .iter()
        .filter(|w| w.status == WishStatus::Planned && w.kind == WishKind::OneOff)
        .filter(|w| w.parent_id.as_deref().is_none_or(parent_planned))
        .map(|w| w.saved)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::gateway::WishPurchase;
    use finance_types::report::Currency;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn month(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn wish(id: &str, parent: Option<&str>, amount: &str, due: &str, card: bool) -> Wish {
        Wish {
            wish_id: id.into(),
            parent_id: parent.map(String::from),
            name: id.into(),
            amount: d(amount),
            currency: Currency::new("EUR"),
            due_on: due.into(),
            source: if card {
                WishSource::Card
            } else {
                WishSource::Account {
                    account_id: "a".into(),
                }
            },
            saved: Decimal::ZERO,
            saved_month: None,
            status: WishStatus::Planned,
            bought: None,
            kind: WishKind::OneOff,
            must: false,
            created_at: format!("2026-01-01T00:00:{id}"),
            event: None,
        }
    }

    fn event_wish(amount: &str, saved: &str) -> Wish {
        Wish {
            saved: d(saved),
            event: Some(finance_types::gateway::EventDetails {
                starts_on: "2026-11-30".into(),
                ends_on: "2026-12-05".into(),
                ..Default::default()
            }),
            ..wish("e", None, amount, "2026-11-30", true)
        }
    }

    fn need(months: &[(&str, &str)]) -> Vec<crate::events::EventNeed> {
        vec![crate::events::EventNeed {
            wish_id: "e".into(),
            months: months.iter().map(|(m, n)| (month(m), d(n))).collect(),
        }]
    }

    fn remaining(items: &[Planned]) -> Vec<(&str, Decimal)> {
        items
            .iter()
            .map(|i| (i.wish_id.as_str(), i.remaining))
            .collect()
    }

    #[test]
    fn an_event_is_one_item_per_month_it_still_asks_money_of() {
        let nov = month("2026-11-01");
        let needs = need(&[("2026-11-01", "66.67"), ("2026-12-01", "283.33")]);
        let (items, _) = plan_items_with(&[event_wish("400", "0")], nov, &HashMap::new(), &needs);
        assert_eq!(
            remaining(&items),
            vec![("e#2026-11", d("66.67")), ("e#2026-12", d("283.33"))]
        );
        assert_eq!(items[0].due, month("2026-11-30"));
        assert_eq!(items[1].due, month("2026-12-01"));
        // An event is not put on a card by choice: the ladder decides.
        assert!(!items[0].on_card);
        // Money saved for it pays the earliest month first.
        let (items, _) = plan_items_with(&[event_wish("400", "100")], nov, &HashMap::new(), &needs);
        assert_eq!(remaining(&items), vec![("e#2026-12", d("250"))]);
        // An event that asks for nothing more leaves no item.
        let (items, _) = plan_items_with(
            &[event_wish("400", "0")],
            nov,
            &HashMap::new(),
            &[crate::events::EventNeed {
                wish_id: "e".into(),
                months: Vec::new(),
            }],
        );
        assert!(items.is_empty());
    }

    #[test]
    fn an_events_parts_stay_their_own_items() {
        let nov = month("2026-11-01");
        let zoo = wish("zoo", Some("e"), "18", "2026-12-02", false);
        let needs = need(&[("2026-12-01", "300")]);
        let (items, fams) =
            plan_items_with(&[event_wish("400", "0"), zoo], nov, &HashMap::new(), &needs);
        assert_eq!(
            remaining(&items),
            vec![("zoo", d("18")), ("e#2026-12", d("300"))]
        );
        assert_eq!(fams[0].rest, d("382"));
    }

    #[test]
    fn money_saved_for_an_event_is_held_only_for_what_it_still_asks() {
        let zoo = wish("zoo", Some("e"), "18", "2026-12-02", false);
        let wishes = [event_wish("400", "300"), zoo];
        // Still asking 100 for its days and 18 for a part: 118 of 300 held.
        let asking = need(&[("2026-12-01", "100")]);
        assert_eq!(earmarked_with(&wishes, &asking), d("118"));
        // Over and done: nothing asked, only the part still to buy.
        let done = need(&[]);
        assert_eq!(earmarked_with(&wishes, &done), d("18"));
        // Without a reading of the event it is held like any wish.
        assert_eq!(earmarked_with(&wishes, &[]), d("300"));
    }

    #[test]
    fn a_month_share_reads_as_its_wish() {
        assert_eq!(base_id("e#2026-12"), "e");
        assert_eq!(base_id("e"), "e");
    }

    fn amounts(v: &[Demand]) -> Vec<Decimal> {
        v.iter().map(|x| x.amount).collect()
    }

    fn oct() -> Date {
        month("2026-10-01")
    }

    fn wish_must(id: &str, parent: Option<&str>, amount: &str, due: &str) -> Wish {
        Wish {
            must: true,
            ..wish(id, parent, amount, due, false)
        }
    }

    fn planned(id: &str, amount: &str, due: &str) -> Planned {
        Planned {
            wish_id: id.into(),
            label: id.into(),
            due: month(due),
            remaining: d(amount),
            on_card: false,
            created_at: format!("2026-01-01T00:00:{id}"),
            per_month: None,
            saved_this_month: false,
            must: false,
        }
    }

    /// Runs the saver paying every set-aside and purchase in full.
    fn saved_in_full(items: &[Planned], months: usize) -> Vec<Vec<Demand>> {
        let mut s = Saver::new(items, oct(), months);
        (0..months)
            .map(|m| {
                let mut out = s.set_asides(m);
                out.extend(s.purchases(m));
                for x in &out {
                    s.paid(x.wish_id.as_deref().unwrap(), x.amount);
                }
                out
            })
            .collect()
    }

    #[test]
    fn single_wish_this_month() {
        let (items, fams) = plan_items(
            &[wish("w", None, "150", "2026-11-12", false)],
            month("2026-11-01"),
            &HashMap::new(),
        );
        assert!(fams.is_empty());
        let s = Saver::new(&items, month("2026-11-01"), 2);
        assert!(s.set_asides(0).is_empty());
        assert_eq!(amounts(&s.purchases(0)), vec![d("150")]);
        assert!(s.set_asides(1).is_empty() && s.purchases(1).is_empty());
    }

    #[test]
    fn asks_for_the_whole_rest_until_it_is_saved() {
        let cur = oct();
        let (items, _) = plan_items(
            &[wish("flights", None, "900", "2026-12-20", true)],
            cur,
            &HashMap::new(),
        );
        let out = saved_in_full(&items, 3);
        assert_eq!(amounts(&out[0]), vec![d("900")]);
        assert!(out[0][0].set_aside && !out[0][0].on_card);
        assert_eq!(out[0][0].label, "Saving for flights");
        assert!(out[1].is_empty() && out[2].is_empty());
        // Through January, when December's statement is charged.
        assert_eq!(horizon(&items, cur), 4);
        // Nothing saved by the due month: the purchase is the whole amount, on the card.
        let s = Saver::new(&items, cur, 3);
        let bought = &s.purchases(2)[0];
        assert_eq!(
            (bought.amount, bought.on_card, bought.set_aside),
            (d("900"), true, false)
        );
        assert_eq!(bought.label, "flights");
    }

    #[test]
    fn saved_reduces_set_aside() {
        let mut w = wish("flights", None, "900", "2026-12-20", false);
        w.saved = d("300");
        let (items, _) = plan_items(&[w], oct(), &HashMap::new());
        assert_eq!(amounts(&saved_in_full(&items, 3)[0]), vec![d("600")]);
    }

    #[test]
    fn purchase_takes_what_was_not_saved() {
        let (items, _) = plan_items(
            &[wish("w", None, "100", "2026-12-20", false)],
            oct(),
            &HashMap::new(),
        );
        let mut s = Saver::new(&items, oct(), 3);
        assert_eq!(amounts(&s.set_asides(0)), vec![d("100")]);
        s.paid("w", d("40"));
        assert_eq!(amounts(&s.set_asides(1)), vec![d("60")]);
        assert!(s.set_asides(2).is_empty());
        assert_eq!(amounts(&s.purchases(2)), vec![d("60")]);
    }

    #[test]
    fn parent_rest_after_subs() {
        let cur = month("2026-10-01");
        let ws = [
            wish("trip", None, "3000", "2026-12-31", false),
            wish("fl", Some("trip"), "900", "2026-11-10", false),
            wish("hotel", Some("trip"), "1200", "2026-12-01", false),
        ];
        let (items, fams) = plan_items(&ws, cur, &HashMap::new());
        assert_eq!(
            fams,
            vec![Family {
                parent_id: "trip".into(),
                rest: d("900"),
                over_cap: false
            }]
        );
        let rest = items.iter().find(|i| i.wish_id == "trip").unwrap();
        assert_eq!(rest.remaining, d("900"));
        assert_eq!(rest.label, "trip: rest");
        assert_eq!(rest.due, month("2026-12-31"));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn bought_sub_cheaper_returns_leftover() {
        let cur = month("2026-10-01");
        let mut sub = wish("fl", Some("trip"), "900", "2026-11-10", false);
        sub.status = WishStatus::Bought;
        sub.bought = Some(WishPurchase {
            amount: d("800"),
            on: "2026-10-05".into(),
            source: WishSource::Card,
            cards: Vec::new(),
            transaction_id: None,
        });
        let ws = [wish("trip", None, "3000", "2026-12-31", false), sub];
        let (items, fams) = plan_items(&ws, cur, &HashMap::new());
        assert_eq!(fams[0].rest, d("2200"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].remaining, d("2200"));
    }

    #[test]
    fn over_cap_flags_and_rest_is_zero() {
        let cur = oct();
        let ws = [
            wish("p", None, "1000", "2026-12-31", false),
            wish("a", Some("p"), "700", "2026-11-10", false),
            wish("b", Some("p"), "500", "2026-11-10", false),
        ];
        let (items, fams) = plan_items(&ws, cur, &HashMap::new());
        assert!(fams[0].over_cap);
        assert_eq!(fams[0].rest, Decimal::ZERO);
        assert!(
            items
                .iter()
                .filter(|i| i.wish_id == "p")
                .all(|i| i.remaining == Decimal::ZERO)
        );
        let out = saved_in_full(&items, 3);
        assert!(out.iter().flatten().all(|x| x.amount > Decimal::ZERO));
        assert!(
            out.iter()
                .flatten()
                .all(|x| x.wish_id.as_deref() != Some("p"))
        );
    }

    #[test]
    fn parent_without_children_is_all_rest() {
        let cur = month("2026-10-01");
        let (items, fams) = plan_items(
            &[wish("p", None, "3000", "2026-10-20", false)],
            cur,
            &HashMap::new(),
        );
        assert!(fams.is_empty());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].remaining, d("3000"));
    }

    #[test]
    fn overdue_wish_is_due_now() {
        let cur = oct();
        let (items, _) = plan_items(
            &[wish("w", None, "50", "2026-09-01", false)],
            cur,
            &HashMap::new(),
        );
        let s = Saver::new(&items, cur, 2);
        assert_eq!(amounts(&s.purchases(0)), vec![d("50")]);
        assert!(s.purchases(1).is_empty());
        assert_eq!(horizon(&items, cur), 2);
    }

    #[test]
    fn dropped_and_bought_are_ignored() {
        let cur = month("2026-10-01");
        let mut a = wish("a", None, "50", "2026-10-20", false);
        a.status = WishStatus::Dropped;
        let mut b = wish("b", None, "60", "2026-10-20", false);
        b.status = WishStatus::Bought;
        let (items, fams) = plan_items(&[a, b], cur, &HashMap::new());
        assert!(items.is_empty() && fams.is_empty());
    }

    #[test]
    fn children_of_dropped_parent_ignored() {
        let cur = month("2026-10-01");
        let mut p = wish("p", None, "500", "2026-10-20", false);
        p.status = WishStatus::Dropped;
        let (items, _) = plan_items(
            &[p, wish("c", Some("p"), "50", "2026-10-20", false)],
            cur,
            &HashMap::new(),
        );
        assert!(items.is_empty());
    }

    #[test]
    fn ordering_set_asides_by_due_then_purchases_by_date_then_created() {
        let a = wish("a", None, "10", "2026-10-25", false);
        let b = wish("b", None, "20", "2026-10-05", false);
        let c = wish("c", None, "60", "2026-11-05", false);
        let (items, _) = plan_items(&[a, b, c], oct(), &HashMap::new());
        let out = saved_in_full(&items, 2);
        let labels: Vec<_> = out[0].iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, vec!["Saving for c", "b", "a"]);
    }

    #[test]
    fn horizon_clamps() {
        let cur = month("2026-10-01");
        let (items, _) = plan_items(
            &[wish("w", None, "50", "2030-10-20", false)],
            cur,
            &HashMap::new(),
        );
        assert_eq!(horizon(&items, cur), 49);
        let (items, _) = plan_items(
            &[wish("w", None, "50", "2032-10-20", false)],
            cur,
            &HashMap::new(),
        );
        assert_eq!(horizon(&items, cur), 60);
        assert_eq!(horizon(&[], cur), 2);
    }

    #[test]
    fn earmarked_sums_planned_saved() {
        let mut a = wish("a", None, "50", "2026-10-20", false);
        a.saved = d("10");
        let mut b = wish("b", Some("a"), "50", "2026-10-20", false);
        b.saved = d("5");
        let mut c = wish("c", None, "50", "2026-10-20", false);
        c.saved = d("99");
        c.status = WishStatus::Dropped;
        assert_eq!(earmarked(&[a, b, c]), d("15"));
    }

    #[test]
    fn saving_recorded_this_month_skips_month_zero() {
        let cur = oct();
        let mut w = wish("flights", None, "900", "2026-12-20", false);
        w.saved = d("300");
        w.saved_month = Some("2026-10-01".into());
        let (items, _) = plan_items(&[w.clone()], cur, &HashMap::new());
        let out = saved_in_full(&items, 3);
        assert!(out[0].is_empty());
        assert_eq!(amounts(&out[1]), vec![d("600")]);
        assert!(out[2].is_empty());
        // Saved in an earlier month: month 0 asks for the rest as before.
        w.saved_month = Some("2026-09-01".into());
        let (items, _) = plan_items(&[w.clone()], cur, &HashMap::new());
        assert_eq!(amounts(&saved_in_full(&items, 3)[0]), vec![d("600")]);
        // Due this month: unchanged.
        w.due_on = "2026-10-20".into();
        w.saved_month = Some("2026-10-01".into());
        let (items, _) = plan_items(&[w], cur, &HashMap::new());
        assert_eq!(amounts(&saved_in_full(&items, 3)[0]), vec![d("600")]);
    }

    #[test]
    fn parent_rest_saved_this_month_skips_month_zero() {
        let mut trip = wish("trip", None, "1000", "2026-12-20", false);
        trip.saved = d("100");
        trip.saved_month = Some("2026-10-01".into());
        let ws = [trip, wish("fl", Some("trip"), "400", "2026-12-01", false)];
        let (items, _) = plan_items(&ws, oct(), &HashMap::new());
        let rest: Vec<_> = saved_in_full(&items, 3)
            .iter()
            .map(|m| {
                m.iter()
                    .filter(|x| x.wish_id.as_deref() == Some("trip"))
                    .map(|x| x.amount)
                    .sum::<Decimal>()
            })
            .collect();
        assert_eq!(rest, vec![Decimal::ZERO, d("500"), Decimal::ZERO]);
    }

    #[test]
    fn saver_asks_for_the_rest_until_it_is_saved() {
        // 900 due in 2 months (idx 2), nothing saved: every month asks for all of it.
        let items = vec![planned("w", "900", "2026-12-15")];
        let mut s = Saver::new(&items, oct(), 3);
        assert_eq!(s.set_asides(0)[0].amount, d("900"));
        // Month 0 skipped: nothing paid. Month 1 asks for the 900 again.
        assert_eq!(s.set_asides(1)[0].amount, d("900"));
        s.paid("w", d("450"));
        assert_eq!(s.remaining("w"), d("450"));
        assert_eq!(s.set_asides(2), vec![]); // due month: purchase, not set-aside
        assert_eq!(s.purchases(2)[0].amount, d("450"));
        s.paid("w", d("450"));
        assert!(s.purchases(2).is_empty());
        assert_eq!(s.remaining("w"), Decimal::ZERO);
    }

    #[test]
    fn saver_leaves_out_items_due_past_the_horizon() {
        let items = vec![planned("w", "900", "2027-03-15")];
        let s = Saver::new(&items, oct(), 3);
        assert!(s.set_asides(0).is_empty());
        assert_eq!(s.remaining("w"), Decimal::ZERO);
    }

    #[test]
    fn set_asides_come_soonest_due_first() {
        let items = vec![
            planned("later", "900", "2027-03-15"),
            planned("soon", "300", "2026-12-15"),
            planned("soon_b", "300", "2026-12-15"),
        ];
        let s = Saver::new(&items, oct(), 6);
        let ids: Vec<_> = s
            .set_asides(0)
            .into_iter()
            .map(|d| d.wish_id.unwrap())
            .collect();
        assert_eq!(ids, ["soon", "soon_b", "later"]);
    }

    #[test]
    fn must_flag_flows_to_demands_and_parts() {
        let parent = wish_must("p", None, "1000", "2027-01-10");
        let sub = wish("s", Some("p"), "400", "2026-12-10", false); // not flagged itself
        let (items, _) = plan_items(&[parent, sub], oct(), &HashMap::new());
        assert!(items.iter().all(|i| i.must));
        let s = Saver::new(&items, oct(), 4);
        assert_eq!(s.set_asides(0).len(), 2);
        assert!(s.set_asides(0).iter().all(|d| d.must));
        let (items, _) = plan_items(
            &[wish("x", None, "100", "2026-10-10", false)],
            oct(),
            &HashMap::new(),
        );
        assert!(!items[0].must);
    }

    #[test]
    fn repeat_demands_keep_freed_first_and_carry_must() {
        let cur = oct();
        let mut psy = recurring("psy", "80", "2026-10-03", Repeat::Monthly, None);
        psy.must = true;
        let mut down = wish("down", None, "900", "2026-10-01", false);
        down.kind = WishKind::BillChange {
            bill_id: "rent".into(),
            ends_on: None,
        };
        let one_off = wish("w", None, "100", "2026-10-20", false);
        let (items, _) = plan_items(&[psy, down, one_off], cur, &rent_bill());
        let out = repeat_demands(&items, cur, 2);
        assert_eq!(out.len(), 2);
        for m in &out {
            let labels: Vec<_> = m.iter().map(|x| x.label.as_str()).collect();
            assert_eq!(labels, vec!["down", "psy"]);
            assert_eq!(amounts(m), vec![d("-50"), d("80")]);
            assert!(!m[0].must && m[1].must);
        }
    }

    #[test]
    fn earmarked_skips_parts_of_settled_parents() {
        let mut p = wish("p", None, "500", "2026-12-20", false);
        p.status = WishStatus::Bought;
        let mut c = wish("c", Some("p"), "50", "2026-12-20", false);
        c.saved = d("20");
        let mut orphan = wish("o", Some("gone"), "50", "2026-12-20", false);
        orphan.saved = d("7");
        let mut live = wish("l", None, "50", "2026-12-20", false);
        live.saved = d("5");
        assert_eq!(earmarked(&[p, c, orphan, live]), d("5"));
    }

    #[test]
    fn recurring_on_card_goes_on_card() {
        let cur = month("2026-10-01");
        let mut w = recurring("psy", "80", "2026-10-03", Repeat::Monthly, None);
        w.source = WishSource::Card;
        let mut change = wish("up", None, "1250", "2026-10-01", true);
        change.kind = WishKind::BillChange {
            bill_id: "rent".into(),
            ends_on: None,
        };
        let (items, _) = plan_items(&[w, change], cur, &rent_bill());
        let out = repeat_demands(&items, cur, 2);
        let psy = out[0].iter().find(|x| x.label == "psy").unwrap();
        assert!(psy.on_card);
        let up = out[0].iter().find(|x| x.label == "up").unwrap();
        assert!(!up.on_card);
    }

    fn recurring(id: &str, amount: &str, due: &str, every: Repeat, ends: Option<&str>) -> Wish {
        let mut w = wish(id, None, amount, due, false);
        w.kind = WishKind::Recurring {
            every,
            ends_on: ends.map(String::from),
        };
        w
    }

    fn rent_bill() -> HashMap<String, BillNow> {
        HashMap::from([(
            "rent".to_string(),
            BillNow {
                amount: d("950"),
                every: Repeat::Monthly,
            },
        )])
    }

    #[test]
    fn occurrences_weekly_counts_weekdays() {
        let first = month("2026-11-03");
        assert_eq!(
            occurrences(first, Repeat::Weekly, None, month("2026-11-01")),
            4
        );
        assert_eq!(
            occurrences(first, Repeat::Weekly, None, month("2026-12-01")),
            5
        );
    }

    #[test]
    fn occurrences_biweekly() {
        let first = month("2026-11-03");
        assert_eq!(
            occurrences(first, Repeat::Biweekly, None, month("2026-11-01")),
            2
        );
        assert_eq!(
            occurrences(first, Repeat::Biweekly, None, month("2026-12-01")),
            3
        );
    }

    #[test]
    fn occurrences_respect_start_and_end() {
        let first = month("2026-12-15");
        let end = Some(month("2027-02-15"));
        let n = |m: &str| occurrences(first, Repeat::Monthly, end, month(m));
        assert_eq!(n("2026-11-01"), 0);
        assert_eq!(n("2026-12-01"), 1);
        assert_eq!(n("2027-02-01"), 1);
        assert_eq!(n("2027-03-01"), 0);
        assert_eq!(
            occurrences(
                first,
                Repeat::Monthly,
                Some(month("2027-02-14")),
                month("2027-02-01")
            ),
            0
        );
    }

    #[test]
    fn occurrences_clamp_day_and_yearly() {
        let first = month("2026-01-31");
        assert_eq!(
            occurrences(first, Repeat::Monthly, None, month("2026-02-01")),
            1
        );
        assert_eq!(
            occurrences(first, Repeat::Yearly, None, month("2027-01-01")),
            1
        );
        assert_eq!(
            occurrences(first, Repeat::Yearly, None, month("2027-02-01")),
            0
        );
    }

    #[test]
    fn recurring_demand_each_month() {
        let cur = month("2026-10-01");
        let w = recurring("psy", "80", "2026-11-03", Repeat::Biweekly, None);
        let (items, _) = plan_items(&[w], cur, &HashMap::new());
        let out = repeat_demands(&items, cur, 3);
        assert!(out[0].is_empty());
        assert_eq!(amounts(&out[1]), vec![d("160")]);
        assert_eq!(amounts(&out[2]), vec![d("240")]);
        assert_eq!(out[1][0].label, "psy");
    }

    #[test]
    fn bill_change_up_is_delta() {
        let cur = month("2026-10-01");
        let mut w = wish("up", None, "1250", "2027-02-01", false);
        w.kind = WishKind::BillChange {
            bill_id: "rent".into(),
            ends_on: None,
        };
        let (items, _) = plan_items(&[w], cur, &rent_bill());
        let out = repeat_demands(&items, cur, 6);
        for (i, m) in out.iter().enumerate() {
            if i < 4 {
                assert!(m.is_empty());
            } else {
                assert_eq!(amounts(m), vec![d("300")]);
            }
        }
    }

    #[test]
    fn bill_change_unknown_bill_and_zero_delta_give_nothing() {
        let cur = month("2026-10-01");
        let mut w = wish("x", None, "1250", "2026-11-01", false);
        w.kind = WishKind::BillChange {
            bill_id: "nope".into(),
            ends_on: None,
        };
        assert!(plan_items(&[w.clone()], cur, &rent_bill()).0.is_empty());
        w.kind = WishKind::BillChange {
            bill_id: "rent".into(),
            ends_on: None,
        };
        w.amount = d("950");
        let (items, _) = plan_items(&[w], cur, &rent_bill());
        assert!(repeat_demands(&items, cur, 3).iter().all(Vec::is_empty));
    }

    #[test]
    fn horizon_at_least_twelve_with_recurring() {
        let cur = month("2026-10-01");
        let w = recurring("r", "5", "2026-10-20", Repeat::Monthly, None);
        let (items, _) = plan_items(&[w], cur, &HashMap::new());
        assert_eq!(horizon(&items, cur), 12);
    }

    #[test]
    fn started_is_ignored_and_recurring_saved_not_earmarked() {
        let cur = month("2026-10-01");
        let mut w = recurring("r", "5", "2026-10-20", Repeat::Monthly, None);
        w.saved = d("40");
        assert_eq!(earmarked(&[w.clone()]), Decimal::ZERO);
        w.status = WishStatus::Started;
        assert!(plan_items(&[w], cur, &HashMap::new()).0.is_empty());
    }

    #[test]
    fn recurring_with_parent_is_still_a_leaf() {
        let cur = month("2026-10-01");
        let mut r = recurring("r", "5", "2026-10-20", Repeat::Monthly, None);
        r.parent_id = Some("p".into());
        let ws = [wish("p", None, "300", "2026-12-20", false), r];
        let (items, fams) = plan_items(&ws, cur, &HashMap::new());
        assert!(fams.is_empty());
        assert_eq!(items.len(), 2);
        assert_eq!(
            items.iter().find(|i| i.wish_id == "p").unwrap().remaining,
            d("300")
        );
    }

    #[test]
    fn statement_month_turns_on_the_26th() {
        assert_eq!(statement_month(month("2026-10-25")), month("2026-10-01"));
        assert_eq!(statement_month(month("2026-10-26")), month("2026-11-01"));
        assert_eq!(statement_month(month("2026-11-01")), month("2026-11-01"));
        assert_eq!(statement_month(month("2026-12-31")), month("2027-01-01"));
    }

    #[test]
    fn card_wish_lands_in_its_statement_month() {
        let card = wish("card", None, "100", "2026-10-27", true);
        let bank = wish("bank", None, "100", "2026-10-27", false);
        let (items, _) = plan_items(&[card, bank], oct(), &HashMap::new());
        let s = Saver::new(&items, oct(), 3);
        let ids =
            |v: Vec<Demand>| -> Vec<String> { v.into_iter().filter_map(|x| x.wish_id).collect() };
        assert_eq!(ids(s.purchases(0)), vec!["bank".to_string()]);
        assert_eq!(ids(s.purchases(1)), vec!["card".to_string()]);
        // Through December, when November's statement is charged.
        assert_eq!(horizon(&items, oct()), 3);
    }

    #[test]
    fn repeating_card_wish_counts_by_statement() {
        let mut w = recurring("gym", "30", "2026-10-27", Repeat::Monthly, None);
        w.source = WishSource::Card;
        let (items, _) = plan_items(&[w], oct(), &HashMap::new());
        let out = repeat_demands(&items, oct(), 3);
        // 27 Oct is November's statement, 27 Nov December's.
        assert!(out[0].is_empty());
        assert_eq!(amounts(&out[1]), vec![d("30")]);
        assert_eq!(amounts(&out[2]), vec![d("30")]);
        // A weekly card wish: Oct 26 - Nov 25 holds 5 Mondays from 26 Oct.
        let mut w = recurring("lunch", "10", "2026-10-26", Repeat::Weekly, None);
        w.source = WishSource::Card;
        let (items, _) = plan_items(&[w], oct(), &HashMap::new());
        let out = repeat_demands(&items, oct(), 2);
        assert!(out[0].is_empty());
        assert_eq!(amounts(&out[1]), vec![d("50")]);
    }
}
