//! Bills: expected pay dates, payments and remaining costs (Python subscriptions.py).
//!
//! Python read Firefly's bill list, where Firefly had already expanded each
//! bill into pay dates and paid dates. The ledger keeps the bill itself, so
//! both are derived here the way Firefly III did: pay dates step from the
//! bill's start date by its frequency (clamped to month ends, every
//! `skip + 1`-th period), and paid dates are the bookings linked to the bill.

use std::collections::{BTreeMap, HashMap, HashSet};

use finance_types::gateway::{CardSettings, SubscriptionCategoryRule};
use finance_types::report::subscriptions::{
    BillLinkCandidate, ScheduledBillPayment, SubscriptionReport, SubscriptionRow,
};
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::Date;
use jiff::{Span, Timestamp, ToSpan, Zoned};
use rust_decimal::Decimal;

use crate::load::Bill;
use crate::model::{Kind, Tx};
use crate::period::shift_month;
use crate::text::casefold;
use crate::{Ctx, Error, Result};

/// A bill counts as paid ("matched") when a linked booking is this recent;
/// Firefly reported paid dates from a window starting this many days back.
pub const PAID_LOOKBACK_DAYS: i64 = 31;
/// Bills read longer ago than this may have changed at the bank's side.
pub const STALE_AFTER_HOURS: i64 = 48;
/// Payment evidence (counts, spending, candidates) covers this many months
/// before the current one.
pub const EVIDENCE_MONTHS: i32 = 5;

/// Which stretch of days a report covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillWindow {
    /// A whole calendar month, `0` = this month (up to 3 months ahead).
    CalendarMonth(u8),
    /// From today to the day before the next payday (30 days if unknown).
    UntilPayday(Option<Date>),
}

/// How far the budget's current month runs ahead of the calendar (after an
/// early month cutoff), so its bills come from a later calendar month.
pub fn budget_calendar_offset(ctx: &Ctx) -> u8 {
    let today = ctx.today();
    let current = ctx.current_month();
    let months = (i32::from(current.year()) - i32::from(today.year())) * 12
        + i32::from(current.month())
        - i32::from(today.month());
    u8::try_from(months.clamp(0, 2)).unwrap_or(0)
}

/// Every expected pay date of `bill` from its start through `through`
/// (Firefly's schedule; the bill's end date is not applied here).
pub fn schedule_dates(bill: &Bill, through: Date) -> Vec<Date> {
    let months = match bill.repeat_freq.as_str() {
        "monthly" => Some(1),
        "quarterly" => Some(3),
        "half-year" => Some(6),
        "yearly" => Some(12),
        _ => None,
    };
    let anchor = bill.start;
    let step_size = i64::from(bill.skip) + 1;
    let mut out = Vec::new();
    for index in 0..10_000_i64 {
        let step = index * step_size;
        let due = if bill.repeat_freq == "weekly" {
            anchor.checked_add(Span::new().weeks(step)).ok()
        } else if let Some(months) = months {
            let month = shift_month(
                anchor.first_of_month(),
                i32::try_from(step * months).unwrap_or(i32::MAX),
            );
            month
                .with()
                .day(anchor.day().min(month.days_in_month()))
                .build()
                .ok()
        } else {
            None
        };
        match due {
            Some(due) if due <= through => out.push(due),
            _ => break,
        }
    }
    out
}

/// The booking days of transactions linked to `bill_id` in `from..=to`
/// (Firefly's `paid_dates`).
pub fn linked_payment_dates(transactions: &[Tx], bill_id: &str, from: Date, to: Date) -> Vec<Date> {
    transactions
        .iter()
        .filter(|t| t.subscription_id.as_deref() == Some(bill_id))
        .map(Tx::booked_on)
        .filter(|d| from <= *d && *d <= to)
        .collect()
}

fn fallback_cycle_days(frequency: &str) -> i64 {
    // "half-yearly" (not Firefly's "half-year") as in the Python service.
    match casefold(frequency).as_str() {
        "daily" => 1,
        "weekly" => 7,
        "half-yearly" => 184,
        "yearly" => 366,
        _ => 31,
    }
}

fn ascii_word(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// `word` (or its plural) as a whole word in `text`.
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before_ok = !text[..at].chars().next_back().is_some_and(ascii_word);
        let mut rest = text[at + word.len()..].chars();
        let next = rest.next();
        let after_ok = !next.is_some_and(ascii_word)
            || (next == Some('s') && !rest.next().is_some_and(ascii_word));
        before_ok && after_ok
    })
}

/// A named merchant or category signal; amount alone is not evidence.
fn bill_match_evidence(
    name: &str,
    t: &Tx,
    category_keys: &HashSet<String>,
) -> Option<&'static str> {
    if let Some(category) = &t.category
        && category_keys.contains(&casefold(category.trim()))
    {
        return Some("Assigned transaction category");
    }
    let name = casefold(name);
    let words: Vec<&str> = name
        .split(|c: char| !ascii_word(c))
        .filter(|w| w.len() >= 3 && !["klarna", "monthly", "bill", "payment"].contains(w))
        .collect();
    let description = casefold(t.description.as_deref().unwrap_or(""));
    if !words.is_empty() && words.iter().all(|w| has_word(&description, w)) {
        return Some("Bill name in transaction description");
    }
    let squashed: String = description.chars().filter(|c| ascii_word(*c)).collect();
    if words.len() > 1 && squashed.contains(&words.concat()) {
        return Some("Bill name in transaction description");
    }
    None
}

fn days(n: i64) -> Span {
    n.days()
}

fn add_days(day: Date, n: i64) -> Date {
    day.checked_add(days(n)).unwrap_or(day)
}

/// The bills due in `window`, which are paid, and what is left to pay.
///
/// `fetched_at` is the last ledger import (when the bills were read) and the
/// report is stale when that is more than two days old.
pub fn subscription_report(ctx: &Ctx, window: BillWindow) -> Result<SubscriptionReport> {
    let snap = ctx.snap;
    let inputs = Inputs {
        bills: &snap.bills,
        transactions: &snap.transactions,
        raw_transactions: &snap.raw_transactions,
        subscription_rules: &snap.subscription_rules,
        declined_bill_matches: &snap.declined_bill_matches,
        last_import: snap.last_import,
    };
    report(&inputs, &ctx.now, window)
}

/// A card's linked bill charges what the owner typed as used on the
/// statement: the statement ending in `used_month` is charged in the month
/// after, so that month's unpaid payment of the bill, and the totals that
/// count it, take the typed amount instead of the bill's own.
pub fn apply_card_usage(report: &mut SubscriptionReport, cards: &[CardSettings]) {
    for card in cards {
        let Some(bill_id) = card.bill_id.as_deref() else {
            continue;
        };
        let Some(charged) = format!("{}-01", card.used_month.get(..7).unwrap_or_default())
            .parse::<Date>()
            .ok()
            .map(|m| shift_month(m, 1))
        else {
            continue;
        };
        for payment in &mut report.scheduled_payments {
            let due_then = payment
                .due_on
                .parse::<Date>()
                .is_ok_and(|d| d.first_of_month() == charged);
            if payment.bill_id != bill_id || payment.paid_on.is_some() || !due_then {
                continue;
            }
            let delta = card.used - payment.amount.amount;
            payment.amount.amount = card.used;
            for total in report
                .expected_costs
                .iter_mut()
                .chain(report.remaining_costs.iter_mut())
                .filter(|t| t.currency == payment.amount.currency)
            {
                total.amount += delta;
            }
        }
    }
}

/// The part of the snapshot a bill report reads.
struct Inputs<'a> {
    bills: &'a [Bill],
    transactions: &'a [Tx],
    /// Before owner-marked shared spending unlinks bills.
    raw_transactions: &'a [Tx],
    subscription_rules: &'a [SubscriptionCategoryRule],
    declined_bill_matches: &'a HashSet<(String, String)>,
    last_import: Option<Timestamp>,
}

fn report(snap: &Inputs, now: &Zoned, window: BillWindow) -> Result<SubscriptionReport> {
    let today = now.date();
    let now = now.timestamp();
    let this_month = today.first_of_month();
    let evidence_start = shift_month(this_month, -EVIDENCE_MONTHS);
    let (window_start, end) = match window {
        BillWindow::CalendarMonth(offset) => {
            if offset > 3 {
                return Err(Error::Invalid(
                    "calendar month offset must be between zero and three".into(),
                ));
            }
            let start = shift_month(this_month, i32::from(offset));
            (start, start.last_of_month())
        }
        BillWindow::UntilPayday(Some(payday)) => (today, add_days(payday, -1)),
        BillWindow::UntilPayday(None) => (today, add_days(today, 30)),
    };
    let stale = snap.last_import.is_none_or(|fetched| {
        fetched > now
            || now.duration_since(fetched) > jiff::SignedDuration::from_hours(STALE_AFTER_HOURS)
    });

    let rules: BTreeMap<&str, _> = snap
        .subscription_rules
        .iter()
        .map(|r| (r.bill_id.as_str(), r))
        .collect();
    if rules.len() != snap.subscription_rules.len() {
        return Err(Error::Invalid(
            "subscription category rules must have unique bill ids".into(),
        ));
    }
    let mut explicit_owners: HashMap<String, &str> = HashMap::new();
    for rule in snap.subscription_rules {
        for category in &rule.categories {
            let key = casefold(category);
            if explicit_owners
                .get(&key)
                .is_some_and(|b| *b != rule.bill_id)
            {
                return Err(Error::Invalid(
                    "a category cannot match multiple subscriptions".into(),
                ));
            }
            explicit_owners.insert(key, &rule.bill_id);
        }
    }
    let mut unique_names: HashMap<String, Option<&str>> = HashMap::new();
    for bill in snap.bills {
        let key = casefold(bill.name.trim());
        let owner = (!unique_names.contains_key(&key)).then_some(bill.id.as_str());
        unique_names.insert(key, owner);
    }
    // An owner rule names the categories; otherwise a category with the
    // bill's (unique, unclaimed) name counts.
    let category_keys = |bill: &Bill| -> HashSet<String> {
        if let Some(rule) = rules.get(bill.id.as_str()) {
            return rule.categories.iter().map(|c| casefold(c)).collect();
        }
        let key = casefold(bill.name.trim());
        if unique_names.get(&key).copied().flatten() == Some(bill.id.as_str())
            && !explicit_owners.contains_key(&key)
        {
            HashSet::from([key])
        } else {
            HashSet::new()
        }
    };

    // The Python projection listed transactions by canonical id as text,
    // which decides candidate order among same-day payments.
    let mut evidence: Vec<&Tx> = snap
        .transactions
        .iter()
        .filter(|t| {
            t.kind == Kind::Outflow
                && t.occurred_at.timestamp() <= now
                && evidence_start <= t.booked_on()
                && t.booked_on() <= today
        })
        .collect();
    evidence.sort_by(|a, b| a.transaction_id.cmp(&b.transaction_id));
    let external_id = |t: &Tx| t.ledger_id.to_string();
    let declined = |bill: &Bill, t: &Tx| {
        snap.declined_bill_matches
            .contains(&(bill.id.clone(), external_id(t)))
    };
    let paid_from = add_days(today, -PAID_LOOKBACK_DAYS);

    let mut rows = Vec::new();
    let mut totals: BTreeMap<String, (Currency, Decimal)> = BTreeMap::new();
    let mut remaining: BTreeMap<String, (Currency, Decimal)> = BTreeMap::new();
    let mut scheduled = Vec::new();
    let mut category_matched_names: HashSet<String> = HashSet::new();
    for bill in snap.bills {
        let expires = bill.end;
        if !bill.active || expires.is_some_and(|e| e < today) {
            continue;
        }
        let rule = rules.get(bill.id.as_str()).copied();
        let keys = category_keys(bill);
        let currency = bill.currency.clone();
        let minor = u32::from(currency.minor_unit);
        let (amount_min, amount_max) = (
            bill.amount_min.round_dp(minor),
            bill.amount_max.round_dp(minor),
        );
        let in_range = |t: &Tx| amount_min <= t.amount && t.amount <= amount_max;
        let all_due = schedule_dates(bill, end);
        let dates: Vec<Date> = all_due
            .iter()
            .copied()
            .filter(|d| window_start <= *d && *d <= end && expires.is_none_or(|e| *d <= e))
            .collect();
        let paid = linked_payment_dates(snap.raw_transactions, &bill.id, paid_from, today);
        let native: Vec<&Tx> = evidence
            .iter()
            .copied()
            .filter(|t| t.subscription_id.as_deref() == Some(&bill.id) && t.currency == currency)
            .collect();
        let category_evidence: Vec<&Tx> = evidence
            .iter()
            .copied()
            .filter(|t| {
                t.subscription_id.is_none()
                    && t.currency == currency
                    && t.category
                        .as_ref()
                        .is_some_and(|c| keys.contains(&casefold(c.trim())))
                    && !declined(bill, t)
            })
            .collect();
        let category_transactions: Vec<&Tx> = category_evidence
            .iter()
            .copied()
            .filter(|t| in_range(t))
            .collect();
        category_matched_names.extend(
            category_transactions
                .iter()
                .filter_map(|t| t.category.clone()),
        );
        let automatic_category_dates: Vec<Date> = if rule.is_some() {
            category_transactions
                .iter()
                .map(|t| t.booked_on())
                .collect()
        } else {
            Vec::new()
        };
        let recorded: Vec<&Tx> = native
            .iter()
            .chain(&category_transactions)
            .copied()
            .collect();
        let linked_descriptions: HashSet<String> = snap
            .transactions
            .iter()
            .filter(|t| t.subscription_id.as_deref() == Some(&bill.id))
            .filter_map(|t| t.description.as_deref())
            .map(|d| casefold(d.trim()))
            .collect();
        let category_ids: HashSet<&str> = category_evidence
            .iter()
            .map(|t| t.transaction_id.as_str())
            .collect();
        let mut candidates = Vec::new();
        for t in &evidence {
            if declined(bill, t)
                || t.subscription_id.is_some()
                || t.currency != currency
                || (!in_range(t)
                    && !(rule.is_some() && category_ids.contains(t.transaction_id.as_str())))
            {
                continue;
            }
            let found = bill_match_evidence(&bill.name, t, &keys).or_else(|| {
                t.description
                    .as_deref()
                    .filter(|d| linked_descriptions.contains(&casefold(d.trim())))
                    .map(|_| "Same description as a linked bill payment")
            });
            if let Some(found) = found {
                candidates.push(BillLinkCandidate {
                    transaction_id: external_id(t),
                    occurred_on: t.booked_on().to_string(),
                    description: t
                        .description
                        .clone()
                        .unwrap_or_else(|| "Transaction description unavailable".into()),
                    amount: t.amount,
                    evidence: found.into(),
                    category: t.category.clone(),
                });
            }
        }
        let mut shown = candidates.clone();
        shown.sort_by(|a, b| b.occurred_on.cmp(&a.occurred_on));
        shown.truncate(8);
        rows.push((
            dates.clone(),
            SubscriptionRow {
                bill_id: bill.id.clone(),
                name: bill.name.clone(),
                currency: currency.clone(),
                amount_min,
                amount_max,
                frequency: bill.repeat_freq.clone(),
                expected_dates: dates.iter().map(Date::to_string).collect(),
                last_matched_on: paid.iter().max().map(Date::to_string),
                native_payment_count: native.len(),
                category_payment_count: category_transactions.len(),
                recorded_spending: recorded.iter().map(|t| t.amount).sum(),
                last_recorded_on: recorded
                    .iter()
                    .map(|t| t.booked_on())
                    .max()
                    .map(|d| d.to_string()),
                category_name_match: !category_transactions.is_empty(),
                category_match_categories: rule.map(|r| r.categories.clone()).unwrap_or_default(),
                category_match_configured: rule.is_some(),
                category_match_revision: rule.map_or(0, |r| r.revision),
                link_candidates: shown,
            },
        ));
        if dates.is_empty() {
            continue;
        }
        let count = Decimal::from(dates.len());
        totals
            .entry(currency.code.clone())
            .or_insert((currency.clone(), Decimal::ZERO))
            .1 += count * amount_max;
        let payment_days: Vec<Date> = paid
            .iter()
            .copied()
            .chain(native.iter().map(|t| t.booked_on()))
            .chain(automatic_category_dates.iter().copied())
            .collect();
        for due in dates {
            let previous = all_due
                .iter()
                .copied()
                .filter(|d| *d < due)
                .max()
                .unwrap_or_else(|| add_days(due, -fallback_cycle_days(&bill.repeat_freq)));
            let in_cycle = |d: Date| previous < d && d <= due;
            let mut matches: Vec<Date> = payment_days
                .iter()
                .copied()
                .filter(|d| in_cycle(*d))
                .collect();
            // A category the owner assigned can be paid in parts: sum the
            // whole billing cycle, not an arbitrary subset of payments.
            let parts: Vec<&Tx> = category_evidence
                .iter()
                .copied()
                .filter(|t| in_cycle(t.booked_on()))
                .collect();
            let parts_total: Decimal = parts.iter().map(|t| t.amount).sum();
            if rule.is_some()
                && parts.len() > 1
                && amount_min <= parts_total
                && parts_total <= amount_max
            {
                matches.extend(parts.iter().map(|t| t.booked_on()));
            }
            let paid_on = matches.into_iter().max();
            scheduled.push(ScheduledBillPayment {
                bill_id: bill.id.clone(),
                name: bill.name.clone(),
                due_on: due.to_string(),
                paid_on: paid_on.map(|d| d.to_string()),
                amount: UnitMoney {
                    amount: amount_max,
                    currency: currency.clone(),
                },
                candidate_ids: candidates
                    .iter()
                    .filter(|c| c.occurred_on.parse().is_ok_and(in_cycle))
                    .map(|c| c.transaction_id.clone())
                    .collect(),
            });
            if paid_on.is_none() {
                remaining
                    .entry(currency.code.clone())
                    .or_insert((currency.clone(), Decimal::ZERO))
                    .1 += amount_max;
            }
        }
    }
    rows.sort_by(|(a_dates, a), (b_dates, b)| {
        let key = |d: &Vec<Date>| {
            if d.is_empty() {
                vec![Date::MAX]
            } else {
                d.clone()
            }
        };
        (key(a_dates), &a.name).cmp(&(key(b_dates), &b.name))
    });
    scheduled.sort_by(|a, b| (&a.due_on, &a.name).cmp(&(&b.due_on, &b.name)));
    let money = |m: BTreeMap<String, (Currency, Decimal)>| {
        m.into_values()
            .map(|(currency, amount)| UnitMoney { amount, currency })
            .collect()
    };
    let mut names: Vec<String> = category_matched_names.into_iter().collect();
    names.sort_by_key(|n| casefold(n));
    let rows: Vec<SubscriptionRow> = rows.into_iter().map(|(_, r)| r).collect();
    Ok(SubscriptionReport {
        fetched_at: snap.last_import.map(|t| t.to_string()),
        through_date: end.to_string(),
        stale,
        unmatched_count: rows.iter().filter(|r| r.last_matched_on.is_none()).count(),
        rows,
        expected_costs: money(totals),
        remaining_costs: money(remaining),
        scheduled_payments: scheduled,
        evidence_start: evidence_start.to_string(),
        category_matched_names: names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn card_usage_replaces_the_bill_amount_in_the_charged_month() {
        let eur = Currency::new("EUR");
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        let payment = |bill: &str, due: &str, paid: Option<&str>| ScheduledBillPayment {
            bill_id: bill.into(),
            name: bill.into(),
            due_on: due.into(),
            paid_on: paid.map(Into::into),
            amount: UnitMoney {
                amount: d("400"),
                currency: eur.clone(),
            },
            candidate_ids: Vec::new(),
        };
        let total = |amount: &str| {
            vec![UnitMoney {
                amount: d(amount),
                currency: eur.clone(),
            }]
        };
        let mut report = SubscriptionReport {
            fetched_at: None,
            through_date: "2026-12-31".into(),
            stale: false,
            rows: Vec::new(),
            expected_costs: total("1200"),
            remaining_costs: total("800"),
            scheduled_payments: vec![
                payment("card", "2026-12-01", None),
                payment("rent", "2026-12-01", None),
                payment("card", "2026-12-15", Some("2026-12-15")),
            ],
            unmatched_count: 0,
            evidence_start: "2026-01-01".into(),
            category_matched_names: Vec::new(),
        };
        let card = |month: &str| CardSettings {
            card_id: "card-1".into(),
            name: "Credit card".into(),
            limit: d("2500"),
            used: d("9.98"),
            used_month: month.into(),
            bill_id: Some("card".into()),
        };
        // October's statement is charged in November: nothing here changes.
        apply_card_usage(&mut report, &[card("2026-10-01")]);
        assert_eq!(report.expected_costs, total("1200"));
        // November's statement is charged in December, on the unpaid payment.
        apply_card_usage(&mut report, &[card("2026-11-01")]);
        assert_eq!(report.scheduled_payments[0].amount.amount, d("9.98"));
        assert_eq!(report.scheduled_payments[1].amount.amount, d("400"));
        assert_eq!(report.scheduled_payments[2].amount.amount, d("400"));
        assert_eq!(report.expected_costs, total("809.98"));
        assert_eq!(report.remaining_costs, total("409.98"));
    }

    fn now() -> Zoned {
        "2026-09-09T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap()
    }

    fn bill(id: &str, name: &str, min: &str, max: &str) -> Bill {
        Bill {
            id: id.into(),
            name: name.into(),
            currency: Currency::eur(),
            amount_min: min.parse().unwrap(),
            amount_max: max.parse().unwrap(),
            start: date(2026, 8, 10),
            end: None,
            repeat_freq: "monthly".into(),
            skip: 0,
            active: true,
        }
    }

    fn synthetic() -> Bill {
        bill("synthetic-bill", "Synthetic subscription", "5.00", "7.00")
    }

    fn pay(id: i64, category: Option<&str>, amount: &str, on: &str) -> Tx {
        Tx {
            transaction_id: format!("firefly:{id}"),
            ledger_id: id,
            account_id: "firefly:1".into(),
            counterparty_account_id: None,
            kind: Kind::Outflow,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: format!("{on}T12:00:00[Europe/Amsterdam]").parse().unwrap(),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: category.map(Into::into),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
            repays: None,
        }
    }

    fn linked(id: i64, bill_id: &str, amount: &str, on: &str) -> Tx {
        let mut t = pay(id, None, amount, on);
        t.subscription_id = Some(bill_id.into());
        t
    }

    fn described(mut t: Tx, description: &str) -> Tx {
        t.description = Some(description.into());
        t
    }

    fn rule(bill_id: &str, categories: &[&str]) -> SubscriptionCategoryRule {
        SubscriptionCategoryRule {
            bill_id: bill_id.into(),
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            revision: 1,
            updated_at: "2026-09-01T00:00:00Z".into(),
        }
    }

    struct Fixture {
        bills: Vec<Bill>,
        txs: Vec<Tx>,
        rules: Vec<SubscriptionCategoryRule>,
        declined: HashSet<(String, String)>,
        last_import: Option<Timestamp>,
    }

    impl Fixture {
        fn new(bills: Vec<Bill>, txs: Vec<Tx>) -> Self {
            Self {
                bills,
                txs,
                rules: Vec::new(),
                declined: HashSet::new(),
                last_import: Some(now().timestamp()),
            }
        }

        fn rules(mut self, rules: Vec<SubscriptionCategoryRule>) -> Self {
            self.rules = rules;
            self
        }

        fn run_at(&self, at: &Zoned, window: BillWindow) -> Result<SubscriptionReport> {
            let inputs = Inputs {
                bills: &self.bills,
                transactions: &self.txs,
                raw_transactions: &self.txs,
                subscription_rules: &self.rules,
                declined_bill_matches: &self.declined,
                last_import: self.last_import,
            };
            report(&inputs, at, window)
        }

        fn run(&self, window: BillWindow) -> SubscriptionReport {
            self.run_at(&now(), window).unwrap()
        }
    }

    fn until(payday: Date) -> BillWindow {
        BillWindow::UntilPayday(Some(payday))
    }

    fn eur(amount: &str) -> Vec<UnitMoney> {
        vec![UnitMoney {
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
        }]
    }

    #[test]
    fn forecast_uses_the_schedule_and_linked_payments() {
        let f = Fixture::new(
            vec![synthetic()],
            vec![linked(1, "synthetic-bill", "6.00", "2026-08-10")],
        );
        let report = f.run(until(date(2026, 9, 25)));
        assert!(!report.stale);
        assert_eq!(report.expected_costs, eur("7"));
        assert_eq!(report.rows[0].expected_dates, ["2026-09-10"]);
        assert_eq!(
            report.rows[0].last_matched_on.as_deref(),
            Some("2026-08-10")
        );
        assert_eq!(report.unmatched_count, 0);
        assert_eq!(report.evidence_start, "2026-04-01");
        // The August payment belongs to the August cycle.
        assert_eq!(report.scheduled_payments[0].paid_on, None);
        assert!(f.run(until(date(2026, 9, 10))).expected_costs.is_empty());
    }

    #[test]
    fn remaining_before_payday_needs_direct_payment_evidence() {
        let f = Fixture::new(
            vec![
                bill("rent", "Rent", "750", "750"),
                bill("chat", "Chat GPT", "21.99", "21.99"),
            ],
            vec![
                linked(1, "rent", "750", "2026-09-08"),
                pay(2, Some("Chat GPT"), "21.99", "2026-09-08"),
            ],
        );
        let report = f.run(until(date(2026, 9, 11)));
        assert_eq!(report.expected_costs, eur("771.99"));
        assert_eq!(report.remaining_costs, eur("21.99"));
        let paid: Vec<_> = report
            .scheduled_payments
            .iter()
            .map(|p| (p.name.as_str(), p.paid_on.as_deref()))
            .collect();
        assert_eq!(paid, [("Chat GPT", None), ("Rent", Some("2026-09-08"))]);
    }

    #[test]
    fn explicit_category_rule_marks_the_bill_paid() {
        let f = Fixture::new(
            vec![synthetic()],
            vec![pay(1, Some("Streaming"), "6.00", "2026-09-08")],
        )
        .rules(vec![rule("synthetic-bill", &["Streaming"])]);
        let report = f.run(until(date(2026, 9, 25)));
        let row = &report.rows[0];
        assert!(row.category_match_configured);
        assert_eq!(row.category_match_categories, ["Streaming"]);
        assert_eq!(row.category_payment_count, 1);
        assert_eq!(
            report.scheduled_payments[0].paid_on.as_deref(),
            Some("2026-09-08")
        );
        assert!(report.remaining_costs.is_empty());
    }

    #[test]
    fn empty_rule_disables_the_bill_name_fallback() {
        let f = Fixture::new(
            vec![synthetic()],
            vec![pay(1, Some("Synthetic subscription"), "6.00", "2026-09-08")],
        )
        .rules(vec![rule("synthetic-bill", &[])]);
        let report = f.run(until(date(2026, 9, 25)));
        assert_eq!(report.rows[0].category_payment_count, 0);
        assert_eq!(report.scheduled_payments[0].paid_on, None);
    }

    #[test]
    fn a_category_cannot_belong_to_two_bills() {
        let f = Fixture::new(vec![synthetic()], vec![]).rules(vec![
            rule("synthetic-bill", &["Streaming"]),
            rule("other", &["streaming"]),
        ]);
        assert!(f.run_at(&now(), BillWindow::UntilPayday(None)).is_err());
        assert!(f.run_at(&now(), BillWindow::CalendarMonth(4)).is_err());
    }

    #[test]
    fn inactive_expired_unmatched_and_stale_are_explicit() {
        let mut inactive = synthetic();
        inactive.active = false;
        let mut expired = synthetic();
        expired.end = Some(date(2026, 9, 8));
        assert!(
            Fixture::new(vec![inactive, expired], vec![])
                .run(BillWindow::UntilPayday(None))
                .rows
                .is_empty()
        );
        let mut f = Fixture::new(vec![synthetic()], vec![]);
        let report = f.run(BillWindow::UntilPayday(None));
        assert_eq!(report.unmatched_count, 1);
        let later: Zoned = "2026-09-12T14:00:00+02:00[Europe/Amsterdam]"
            .parse()
            .unwrap();
        assert!(
            f.run_at(&later, BillWindow::UntilPayday(None))
                .unwrap()
                .stale
        );
        f.last_import = None;
        assert!(f.run(BillWindow::UntilPayday(None)).stale);
    }

    #[test]
    fn native_and_category_evidence_never_double_count() {
        let category = Some("Synthetic subscription");
        let mut income = pay(4, category, "6.00", "2026-09-08");
        income.kind = Kind::Inflow;
        let mut both = pay(3, category, "6.00", "2026-09-08");
        both.subscription_id = Some("synthetic-bill".into());
        let f = Fixture::new(
            vec![synthetic()],
            vec![
                linked(1, "synthetic-bill", "6.00", "2026-09-08"),
                pay(2, category, "6.00", "2026-09-08"),
                both,
                income,
                pay(5, category, "6.00", "2026-09-10"),
                pay(6, category, "6.00", "2026-03-31"),
            ],
        );
        let report = f.run(BillWindow::UntilPayday(None));
        let row = &report.rows[0];
        assert_eq!(
            (row.native_payment_count, row.category_payment_count),
            (2, 1)
        );
        assert_eq!(row.recorded_spending, Decimal::from(18));
        assert_eq!(row.last_recorded_on.as_deref(), Some("2026-09-08"));
        assert!(row.category_name_match);
        assert_eq!(report.category_matched_names, ["Synthetic subscription"]);
    }

    #[test]
    fn duplicate_bill_names_disable_category_name_matching() {
        let mut twin = synthetic();
        twin.id = "second".into();
        let f = Fixture::new(
            vec![synthetic(), twin],
            vec![pay(1, Some("Synthetic subscription"), "6.00", "2026-09-08")],
        );
        let report = f.run(BillWindow::UntilPayday(None));
        assert!(report.rows.iter().all(|r| r.category_payment_count == 0));
        assert!(report.category_matched_names.is_empty());
    }

    #[test]
    fn link_candidates_need_name_and_amount_evidence() {
        let charge = "Synthetic subscription charge";
        let f = Fixture::new(
            vec![synthetic()],
            vec![
                described(pay(1, None, "6.00", "2026-09-08"), charge),
                described(
                    pay(2, Some("Synthetic subscription"), "6.00", "2026-09-08"),
                    "Opaque",
                ),
                described(pay(3, None, "6.00", "2026-09-08"), "Unrelated shop"),
                described(pay(4, None, "12.00", "2026-09-08"), charge),
                described(linked(5, "synthetic-bill", "6.00", "2026-09-08"), charge),
            ],
        );
        let ids: Vec<_> = f.run(BillWindow::UntilPayday(None)).rows[0]
            .link_candidates
            .iter()
            .map(|c| c.transaction_id.clone())
            .collect();
        assert_eq!(ids, ["1", "2"]);
    }

    #[test]
    fn plural_merchant_matches_without_combining_klarna_and_coffee() {
        let f = Fixture::new(
            vec![
                bill("cinema", "Cinema", "23", "23"),
                bill("coffee", "Klarna Coffee", "35", "35"),
            ],
            vec![
                described(
                    pay(1, Some("Vue"), "23", "2026-09-08"),
                    "Vue Cinemas subscription",
                ),
                described(
                    pay(2, Some("Coffee"), "35", "2026-09-08"),
                    "Klarna*UAB Kavos Dra",
                ),
            ],
        );
        let ids: Vec<_> = f
            .run(BillWindow::UntilPayday(None))
            .rows
            .iter()
            .flat_map(|r| r.link_candidates.iter().map(|c| c.transaction_id.clone()))
            .collect();
        assert_eq!(ids, ["1"]);
    }

    #[test]
    fn confirmed_description_suggests_future_payments() {
        let memo = "Klarna*UAB Kavos Dra";
        let f = Fixture::new(
            vec![bill("klarna", "Klarna", "35", "35")],
            vec![
                described(linked(1, "klarna", "35", "2026-09-01"), memo),
                described(pay(2, None, "35", "2026-09-08"), memo),
                described(pay(3, None, "35", "2026-09-08"), "Klarna*Other Merchant"),
            ],
        );
        let row = &f.run(BillWindow::UntilPayday(None)).rows[0];
        assert_eq!(row.link_candidates.len(), 1);
        assert_eq!(row.link_candidates[0].transaction_id, "2");
        assert_eq!(
            row.link_candidates[0].evidence,
            "Same description as a linked bill payment"
        );
    }

    #[test]
    fn owner_category_can_be_paid_in_parts_within_one_cycle() {
        let investments = bill("synthetic-bill", "Investments", "60", "60");
        let parts = vec![
            pay(1, Some("Investments"), "50", "2026-09-08"),
            pay(2, Some("Investments"), "10", "2026-09-08"),
            pay(3, Some("Investments"), "60", "2026-07-08"),
        ];
        let rules = vec![rule("synthetic-bill", &["Investments"])];
        let f = Fixture::new(vec![investments.clone()], parts.clone()).rules(rules.clone());
        let report = f.run(until(date(2026, 9, 25)));
        assert_eq!(
            report.scheduled_payments[0].paid_on.as_deref(),
            Some("2026-09-08")
        );
        assert!(report.remaining_costs.is_empty());
        let partial = Fixture::new(vec![investments], parts[..1].to_vec())
            .rules(rules)
            .run(until(date(2026, 9, 25)));
        assert_eq!(partial.scheduled_payments[0].paid_on, None);
        assert_eq!(partial.scheduled_payments[0].candidate_ids, ["1"]);
        assert_eq!(partial.remaining_costs, eur("60"));
    }

    #[test]
    fn declined_match_does_not_count_as_payment() {
        let mut f = Fixture::new(
            vec![synthetic()],
            vec![pay(7, Some("Streaming"), "6.00", "2026-09-08")],
        )
        .rules(vec![rule("synthetic-bill", &["Streaming"])]);
        f.declined.insert(("synthetic-bill".into(), "7".into()));
        let report = f.run(until(date(2026, 9, 25)));
        assert_eq!(report.rows[0].category_payment_count, 0);
        assert!(report.rows[0].link_candidates.is_empty());
        assert_eq!(report.scheduled_payments[0].paid_on, None);
    }

    #[test]
    fn calendar_window_covers_a_whole_later_month() {
        let f = Fixture::new(vec![synthetic()], vec![]);
        let current = f.run(BillWindow::CalendarMonth(0));
        assert_eq!(current.through_date, "2026-09-30");
        assert_eq!(current.rows[0].expected_dates, ["2026-09-10"]);
        let next = f.run(BillWindow::CalendarMonth(1));
        assert_eq!(next.through_date, "2026-10-31");
        assert_eq!(next.rows[0].expected_dates, ["2026-10-10"]);
    }

    #[test]
    fn schedule_respects_skips_and_month_ends() {
        let mut b = synthetic();
        b.start = date(2026, 1, 31);
        b.skip = 1;
        assert_eq!(
            schedule_dates(&b, date(2026, 6, 30)),
            [date(2026, 1, 31), date(2026, 3, 31), date(2026, 5, 31)]
        );
        b.repeat_freq = "weekly".into();
        b.skip = 0;
        assert_eq!(
            schedule_dates(&b, date(2026, 2, 14)),
            [date(2026, 1, 31), date(2026, 2, 7), date(2026, 2, 14)]
        );
        b.repeat_freq = "yearly".into();
        b.start = date(2024, 2, 29);
        assert_eq!(
            schedule_dates(&b, date(2025, 12, 31)),
            [date(2024, 2, 29), date(2025, 2, 28)]
        );
    }

    #[test]
    fn whole_words_allow_a_plural() {
        assert!(has_word("vue cinemas subscription", "cinema"));
        assert!(!has_word("cinematic", "cinema"));
        assert!(!has_word("xcinema", "cinema"));
        assert!(has_word("cinema", "cinema"));
    }
}
