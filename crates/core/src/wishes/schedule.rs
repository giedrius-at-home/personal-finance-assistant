//! From stored wishes to what must be paid in each month.
//!
//! Pure arithmetic, no I/O. A wish is saved for over the months left, one
//! equal set-aside per month, and the last portion is the purchase itself.

use super::ladder::Demand;
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

fn month_index(date: Date) -> i32 {
    i32::from(date.year()) * 12 + i32::from(date.month())
}

/// Whole months from `current_month` to the month of `due` (never negative).
fn months_ahead(due: Date, current_month: Date) -> usize {
    usize::try_from(month_index(due) - month_index(current_month)).unwrap_or(0)
}

fn leaf(w: &Wish, label: String, remaining: Decimal, current_month: Date) -> Planned {
    let due = w.due_on.parse::<Date>().unwrap_or(current_month);
    Planned {
        wish_id: w.wish_id.clone(),
        label,
        due: due.max(current_month),
        remaining: remaining.max(Decimal::ZERO),
        on_card: matches!(w.source, WishSource::Card),
        created_at: w.created_at.clone(),
        per_month: None,
    }
}

/// A recurring wish or bill change as one planned item; `None` when it
/// cannot be judged (unknown bill, bad date).
fn repeating(w: &Wish, bills: &HashMap<String, BillNow>) -> Option<Planned> {
    let (every, ends_on, amount) = match &w.kind {
        WishKind::OneOff => return None,
        WishKind::Recurring { every, ends_on } => (*every, ends_on, w.amount),
        WishKind::BillChange { bill_id, ends_on } => {
            let bill = bills.get(bill_id)?;
            (bill.every, ends_on, w.amount - bill.amount)
        }
    };
    let due = w.due_on.parse::<Date>().ok()?;
    Some(Planned {
        wish_id: w.wish_id.clone(),
        label: w.name.clone(),
        due,
        remaining: Decimal::ZERO,
        on_card: false,
        created_at: w.created_at.clone(),
        per_month: Some((
            every,
            ends_on.as_deref().and_then(|e| e.parse::<Date>().ok()),
            amount,
        )),
    })
}

fn shift_months(month: Date, by: usize) -> Date {
    let by = i64::try_from(by).unwrap_or(0);
    month
        .first_of_month()
        .checked_add(Span::new().months(by))
        .unwrap_or(month)
}

fn days_between(from: Date, to: Date) -> i64 {
    from.until(to).map_or(0, |s| i64::from(s.get_days()))
}

/// How many times something starting on `first` and repeating `every` falls
/// in the month of `month`, up to and including `ends_on`.
pub fn occurrences(first: Date, every: Repeat, ends_on: Option<Date>, month: Date) -> u32 {
    let start = month.first_of_month().max(first);
    let last = month.last_of_month();
    let end = ends_on.map_or(last, |e| e.min(last));
    if start > end {
        return 0;
    }
    let step = match every {
        Repeat::Weekly => 7,
        Repeat::Biweekly => 14,
        Repeat::Monthly | Repeat::Yearly => {
            if every == Repeat::Yearly && month.month() != first.month() {
                return 0;
            }
            let day = first.day().min(month.days_in_month());
            return u32::from(
                month
                    .with()
                    .day(day)
                    .build()
                    .is_ok_and(|on| on >= start && on <= end),
            );
        }
    };
    let skipped = days_between(first, start).div_euclid(step)
        + i64::from(days_between(first, start).rem_euclid(step) != 0);
    let upto = days_between(first, end).div_euclid(step);
    u32::try_from((upto - skipped + 1).max(0)).unwrap_or(0)
}

/// Every planned purchase, plus what is left of each parent's cap.
pub fn plan_items(
    wishes: &[Wish],
    current_month: Date,
    bills: &HashMap<String, BillNow>,
) -> (Vec<Planned>, Vec<Family>) {
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
            items.push(leaf(w, w.name.clone(), w.amount - w.saved, current_month));
            continue;
        }
        let mut rest = w.amount;
        for sub in subs {
            match sub.status {
                WishStatus::Planned => {
                    rest -= sub.amount;
                    items.push(leaf(
                        sub,
                        sub.name.clone(),
                        sub.amount - sub.saved,
                        current_month,
                    ));
                }
                WishStatus::Bought => {
                    rest -= sub.bought.as_ref().map_or(sub.amount, |b| b.amount);
                }
                WishStatus::Dropped | WishStatus::Started => {}
            }
        }
        let over_cap = rest < Decimal::ZERO;
        let rest = rest.max(Decimal::ZERO);
        families.push(Family {
            parent_id: w.wish_id.clone(),
            rest,
            over_cap,
        });
        items.push(leaf(
            w,
            format!("{}: rest", w.name),
            rest - w.saved,
            current_month,
        ));
    }
    (items, families)
}

/// What to pay in each of `months` months from `current_month`.
pub fn demands(items: &[Planned], current_month: Date, months: usize) -> Vec<Vec<Demand>> {
    let mut set_asides: Vec<Vec<Demand>> = vec![Vec::new(); months];
    let mut due_items: Vec<Vec<(Date, &str, Demand)>> = vec![Vec::new(); months];
    let mut repeats: Vec<Vec<Demand>> = vec![Vec::new(); months];
    for item in items {
        if let Some((every, ends_on, each)) = item.per_month {
            for (i, out) in repeats.iter_mut().enumerate() {
                let n = occurrences(item.due, every, ends_on, shift_months(current_month, i));
                let amount = Decimal::from(n) * each;
                if amount != Decimal::ZERO {
                    out.push(Demand {
                        wish_id: Some(item.wish_id.clone()),
                        label: item.label.clone(),
                        amount,
                        on_card: false,
                    });
                }
            }
            continue;
        }
        let due_idx = months_ahead(item.due, current_month);
        if due_idx >= months || item.remaining <= Decimal::ZERO {
            continue;
        }
        let n = Decimal::from(due_idx + 1);
        let portion = (item.remaining / n).round_dp(2);
        if portion > Decimal::ZERO {
            for month in set_asides.iter_mut().take(due_idx) {
                month.push(Demand {
                    wish_id: Some(item.wish_id.clone()),
                    label: format!("Saving for {}", item.label),
                    amount: portion,
                    on_card: false,
                });
            }
        }
        let last = item.remaining - portion * Decimal::from(due_idx);
        if last > Decimal::ZERO {
            due_items[due_idx].push((
                item.due,
                item.created_at.as_str(),
                Demand {
                    wish_id: Some(item.wish_id.clone()),
                    label: item.label.clone(),
                    amount: last,
                    on_card: item.on_card,
                },
            ));
        }
    }
    set_asides
        .into_iter()
        .zip(due_items)
        .zip(repeats)
        .map(|((mut out, mut due), repeating)| {
            due.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            out.extend(due.into_iter().map(|(_, _, demand)| demand));
            // Savings from a cheaper bill come first: they free money for the rest.
            let (freed, costs): (Vec<_>, Vec<_>) = repeating
                .into_iter()
                .partition(|d| d.amount < Decimal::ZERO);
            freed.into_iter().chain(out).chain(costs).collect()
        })
        .collect()
}

/// Months to look ahead: through the latest due month, 2 to 24, and at least
/// 12 when anything recurs.
pub fn horizon(items: &[Planned], current_month: Date) -> usize {
    let latest = items
        .iter()
        .map(|i| {
            let ahead = months_ahead(i.due, current_month) + 1;
            if i.per_month.is_some() {
                ahead.max(12)
            } else {
                ahead
            }
        })
        .max()
        .unwrap_or(0);
    latest.clamp(2, 24)
}

/// Money already set aside for planned wishes.
pub fn earmarked(wishes: &[Wish]) -> Decimal {
    wishes
        .iter()
        .filter(|w| w.status == WishStatus::Planned && w.kind == WishKind::OneOff)
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
            status: WishStatus::Planned,
            bought: None,
            kind: WishKind::OneOff,
            created_at: format!("2026-01-01T00:00:{id}"),
        }
    }

    fn amounts(v: &[Demand]) -> Vec<Decimal> {
        v.iter().map(|x| x.amount).collect()
    }

    #[test]
    fn single_wish_this_month() {
        let (items, fams) = plan_items(
            &[wish("w", None, "150", "2026-11-12", false)],
            month("2026-11-01"),
            &HashMap::new(),
        );
        assert!(fams.is_empty());
        let out = demands(&items, month("2026-11-01"), 2);
        assert_eq!(out.len(), 2);
        assert_eq!(amounts(&out[0]), vec![d("150")]);
        assert!(out[1].is_empty());
    }

    #[test]
    fn spreads_over_months_left() {
        let cur = month("2026-10-01");
        let (items, _) = plan_items(
            &[wish("flights", None, "900", "2026-12-20", true)],
            cur,
            &HashMap::new(),
        );
        let out = demands(&items, cur, 3);
        for m in &out {
            assert_eq!(amounts(m), vec![d("300")]);
        }
        assert!(!out[0][0].on_card && !out[1][0].on_card);
        assert!(out[2][0].on_card);
        assert_eq!(out[0][0].label, "Saving for flights");
        assert_eq!(out[2][0].label, "flights");
        assert_eq!(horizon(&items, cur), 3);
    }

    #[test]
    fn saved_reduces_set_aside() {
        let cur = month("2026-10-01");
        let mut w = wish("flights", None, "900", "2026-12-20", false);
        w.saved = d("300");
        let (items, _) = plan_items(&[w], cur, &HashMap::new());
        let out = demands(&items, cur, 3);
        for m in &out {
            assert_eq!(amounts(m), vec![d("200")]);
        }
    }

    #[test]
    fn due_month_takes_rounding_remainder() {
        let cur = month("2026-10-01");
        let (items, _) = plan_items(
            &[wish("w", None, "100", "2026-12-20", false)],
            cur,
            &HashMap::new(),
        );
        let out = demands(&items, cur, 3);
        assert_eq!(amounts(&out[0]), vec![d("33.33")]);
        assert_eq!(amounts(&out[1]), vec![d("33.33")]);
        assert_eq!(amounts(&out[2]), vec![d("33.34")]);
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
        });
        let ws = [wish("trip", None, "3000", "2026-12-31", false), sub];
        let (items, fams) = plan_items(&ws, cur, &HashMap::new());
        assert_eq!(fams[0].rest, d("2200"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].remaining, d("2200"));
    }

    #[test]
    fn over_cap_flags_and_rest_is_zero() {
        let cur = month("2026-10-01");
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
        let out = demands(&items, cur, 3);
        assert!(out.iter().flatten().all(|x| x.amount > Decimal::ZERO));
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
        let cur = month("2026-10-01");
        let (items, _) = plan_items(
            &[wish("w", None, "50", "2026-09-01", false)],
            cur,
            &HashMap::new(),
        );
        let out = demands(&items, cur, 2);
        assert_eq!(amounts(&out[0]), vec![d("50")]);
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
    fn ordering_set_asides_then_due_by_date_then_created() {
        let cur = month("2026-10-01");
        let a = wish("a", None, "10", "2026-10-25", false);
        let b = wish("b", None, "20", "2026-10-05", false);
        let c = wish("c", None, "60", "2026-11-05", false);
        let (items, _) = plan_items(&[a, b, c], cur, &HashMap::new());
        let out = demands(&items, cur, 2);
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
        assert_eq!(horizon(&items, cur), 24);
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
        let out = demands(&items, cur, 3);
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
        let out = demands(&items, cur, 6);
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
        assert!(demands(&items, cur, 3).iter().all(Vec::is_empty));
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
}
