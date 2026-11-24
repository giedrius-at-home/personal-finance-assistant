//! The Wishes section: planned purchases, whether the coming months can
//! afford them, and the credit card they may lean on.

use std::collections::HashMap;

use finance_types::gateway::{
    CardCharge, CardSettings, EventDetails, Repeat, Wish, WishKind, WishPurchase, WishSettings,
    WishSource, WishStatus,
};
use finance_types::report::Currency;
use finance_types::report::Dashboard;
use finance_types::report::review::SearchRow;
use finance_types::report::wishes::{
    ChangeableBill, Cut, EventOption, Verdict, WishMonth, WishRow, WishesView,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::IgnoredAny;

use crate::api::{NoArgs, invoke, load};
use crate::ds::patterns::{
    BADGE_TIGHT, BOARD_TITLE, CARD_NAME, CHECK_LABEL_START, CHIPS, DIVIDED, Disclosure, EVENT_FORM,
    FACT_ROW, FEEDBACK_LINE, FIGURE, FILTER_ROW, HINT, INLINE_TOOLS, LIST_STACK, METER,
    METER_SCALE, MUTED, NAME_ROW, NESTED, PAIR, PANEL_TITLE, PANEL_TITLE_HELP, SELECT_CHEVRON,
    SELECT_WRAP, SKELETON_CARD, SKELETON_SHORT, STACK, STACK_MD, STACK_XS, STRONG_SM, SUB_ROW,
    TREE_HEAD, TREE_NAME,
};
use crate::ds::{
    BOARD, BOARD_CARD, Badge, BoardColumn, Button, CONTROL, Card, CardTone, Checkbox, Eyebrow,
    Facts, FactsRow, Field, Help, Icon, Lucide, Popup, Size, Skeleton, Stat, Tone, Variant, cn,
};
use crate::pages::Problem;
use crate::pages::payment_pick::{Ask, PaymentPicker};
use crate::{format, state};

/// What an amount box accepts: a plain amount with a point or a comma.
pub(crate) const AMOUNT_PATTERN: &str = "[0-9]+([.,][0-9]{1,2})?";

/// Drops a control that sits beside fields by its label's height
/// (`text-sm` line plus the field's gap), so it lines up with their boxes.
const LABEL_OFFSET: &str = "sm:mt-6.5";

/// Reads an amount box; a comma counts as the decimal point.
pub(crate) fn parse_amount(value: &str) -> Result<Decimal, rust_decimal::Error> {
    value.trim().replace(',', ".").parse()
}

/// An amount the way the verdicts say it: `€70`, `€40.02`.
pub(crate) fn amount(value: Decimal, code: &str) -> String {
    let decimals = if value.fract().is_zero() { 0 } else { 2 };
    format::money_dp(value, code, decimals)
}

/// `2026-11-01` → `November`.
fn month_name(month: &str) -> String {
    format::month_long(month)
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The card statement `day` (`YYYY-MM-DD`) is on, as `YYYY-MM-01`: a
/// statement runs from the 26th to the 25th and is named by the month it
/// ends in, so 27 Oct is on November's.
fn statement_month(day: &str) -> String {
    let num = |range: std::ops::Range<usize>| day.get(range).and_then(|t| t.parse::<u32>().ok());
    let (Some(year), Some(month)) = (num(0..4), num(5..7)) else {
        return day.to_owned();
    };
    let (year, month) = match (num(8..10).unwrap_or(1) >= 26, month) {
        (true, 12) => (year + 1, 1),
        (true, m) => (year, m + 1),
        (false, m) => (year, m),
    };
    format!("{year:04}-{month:02}-01")
}

/// The days statement `month` covers: `2026-11-01` → `26 Oct – 25 Nov`.
fn statement_period(month: &str) -> String {
    let m: usize = month.get(5..7).and_then(|t| t.parse().ok()).unwrap_or(1);
    let before = format!("2000-{:02}", if m == 1 { 12 } else { m - 1 });
    format!(
        "26 {} \u{2013} 25 {}",
        format::month_short(&before),
        format::month_short(month)
    )
}

/// The ids whose verdicts belong to `row`: its own and its sub-wishes'.
fn row_ids(row: &WishRow) -> Vec<&str> {
    std::iter::once(row.wish.wish_id.as_str())
        .chain(row.children.iter().map(|c| c.wish.wish_id.as_str()))
        .collect()
}

/// One group per cut, in first-cut order, at its lowest amount.
fn merged_cuts<'a>(cuts: impl Iterator<Item = &'a Cut>) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for cut in cuts {
        match out.iter_mut().find(|c| c.group_id == cut.group_id) {
            Some(seen) => seen.to = seen.to.min(cut.to),
            None => out.push(cut.clone()),
        }
    }
    out
}

/// One fact per cut: `Going out down to` → `€0`. `when` names the month
/// when it is not the wish's own.
fn cut_facts(cuts: &[Cut], when: Option<&str>, code: &str) -> Vec<Fact> {
    cuts.iter()
        .map(|cut| {
            let label = match when {
                Some(month) => format!("{} down to, {month}", cut.name),
                None => format!("{} down to", cut.name),
            };
            (label, amount(cut.to, code))
        })
        .collect()
}

/// The row's verdicts month by month, from the first month that has one.
fn row_months<'a>(
    row: &WishRow,
    months: &'a [WishMonth],
) -> Vec<(&'a WishMonth, Vec<&'a Verdict>)> {
    let ids = row_ids(row);
    months
        .iter()
        .map(|m| {
            let mine = m
                .verdicts
                .iter()
                .filter(|v| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id)))
                .collect::<Vec<_>>();
            (m, mine)
        })
        .filter(|(_, mine)| !mine.is_empty())
        .collect()
}

fn total(verdicts: &[&Verdict], part: impl Fn(&Verdict) -> Decimal) -> Decimal {
    verdicts.iter().map(|v| part(v)).sum()
}

/// The merged cuts of the first month that has any.
fn first_cuts(by_month: &[(&WishMonth, Vec<&Verdict>)]) -> Vec<Cut> {
    by_month
        .iter()
        .map(|(_, mine)| merged_cuts(mine.iter().flat_map(|v| &v.cuts)))
        .find(|cuts| !cuts.is_empty())
        .unwrap_or_default()
}

/// A labelled value of a row.
pub type Fact = (String, String);

fn fact(label: impl Into<String>, value: impl Into<String>) -> Fact {
    (label.into(), value.into())
}

/// What the row's verdict comes down to, one value per fact, the worst
/// news first. Empty for a wish that is simply affordable: the chip says so.
pub fn verdict_facts(row: &WishRow, months: &[WishMonth]) -> Vec<Fact> {
    if let Some(why) = &row.not_judged {
        return vec![fact("Not judged yet", why.as_str())];
    }
    let by_month = row_months(row, months);
    let code = row.wish.currency.code.as_str();
    let mut facts = if row.wish.kind == WishKind::OneOff {
        let mut facts = skipped_facts(&by_month, code);
        facts.extend(one_off_facts(&by_month, code));
        facts
    } else {
        repeating_facts(&by_month, code)
    };
    if !facts.is_empty() {
        facts.extend(earlier_saves_facts(row, &by_month, code));
    }
    facts
}

/// `Saved first for Bike` → `€300`: what the month set aside for wishes due
/// earlier before this row got its turn, in the first judged month that
/// did so. Money saved for an earlier wish is why a later one comes up
/// short or costs something, so the row says where it went.
fn earlier_saves_facts(
    row: &WishRow,
    by_month: &[(&WishMonth, Vec<&Verdict>)],
    code: &str,
) -> Vec<Fact> {
    let ids = row_ids(row);
    by_month
        .iter()
        .find_map(|(month, _)| {
            let mine = month
                .verdicts
                .iter()
                .position(|v| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id)))?;
            let mut taken: Vec<(String, Decimal)> = Vec::new();
            for v in &month.verdicts[..mine] {
                if !v.set_aside || v.skipped || v.wish_id.is_none() || v.amount <= Decimal::ZERO {
                    continue;
                }
                let name = v.label.strip_prefix("Saving for ").unwrap_or(&v.label);
                match taken.iter_mut().find(|(seen, _)| seen == name) {
                    Some((_, sum)) => *sum += v.amount,
                    None => taken.push((name.to_owned(), v.amount)),
                }
            }
            (!taken.is_empty()).then_some(taken)
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(name, n)| fact(format!("Saved first for {name}"), amount(n, code)))
        .collect()
}

/// `Saving starts` → `December`, `A month from then` → `€300` for the
/// first month whose set-aside was skipped; `Not set aside in` → that
/// month when no later month sets money aside again.
fn skipped_facts(by_month: &[(&WishMonth, Vec<&Verdict>)], code: &str) -> Vec<Fact> {
    let Some(at) = by_month
        .iter()
        .position(|(_, v)| v.iter().any(|v| v.skipped))
    else {
        return Vec::new();
    };
    let next = by_month[at + 1..].iter().find_map(|(m, v)| {
        let sum = v
            .iter()
            .filter(|v| v.set_aside && !v.skipped)
            .map(|v| v.amount)
            .sum::<Decimal>();
        (sum > Decimal::ZERO).then_some((m, sum))
    });
    match next {
        Some((month, n)) => vec![
            fact("Saving starts", month_name(&month.month)),
            fact("A month from then", amount(n, code)),
        ],
        None => vec![fact("Not set aside in", month_name(&by_month[at].0.month))],
    }
}

fn one_off_facts(by_month: &[(&WishMonth, Vec<&Verdict>)], code: &str) -> Vec<Fact> {
    let short: Decimal = by_month.iter().map(|(_, v)| total(v, |v| v.short)).sum();
    if short > Decimal::ZERO {
        return vec![fact("Short by", amount(short, code))];
    }
    let cuts = first_cuts(by_month);
    let from_savings: Decimal = by_month
        .iter()
        .map(|(_, v)| total(v, |v| v.from_savings))
        .sum();
    let mut facts = Vec::new();
    if from_savings > Decimal::ZERO {
        facts.push(fact("From savings", amount(from_savings, code)));
    }
    let on_card: Decimal = by_month.iter().map(|(_, v)| total(v, |v| v.on_card)).sum();
    let first_card = by_month
        .iter()
        .find(|(_, v)| total(v, |v| v.on_card) > Decimal::ZERO);
    if let (Some((month, _)), true) = (first_card, on_card > Decimal::ZERO) {
        let next = month_name(&format::add_months(&month.month, 1));
        let n = amount(on_card, code);
        let next_short: Decimal = by_month
            .iter()
            .map(|(_, v)| total(v, |v| v.next_short))
            .sum();
        let next_cuts = merged_cuts(
            by_month
                .iter()
                .flat_map(|(_, v)| v.iter())
                .flat_map(|v| &v.next_cuts),
        );
        if next_short > Decimal::ZERO {
            let mut facts = vec![
                fact("On the cards", n),
                fact(format!("{next} starts short by"), amount(next_short, code)),
            ];
            facts.extend(cut_facts(&next_cuts, Some(&next), code));
            return facts;
        }
        facts.push(fact("On the cards", n.clone()));
        facts.push(fact(format!("{next} starts lighter by"), n));
        facts.extend(cut_facts(&next_cuts, Some(&next), code));
        facts.extend(cut_facts(&cuts, None, code));
        return facts;
    }
    if from_savings > Decimal::ZERO {
        if !cuts.is_empty() {
            facts.extend(cut_facts(&cuts, None, code));
            return facts;
        }
        let left = by_month
            .iter()
            .rev()
            .find(|(_, v)| total(v, |v| v.from_savings) > Decimal::ZERO)
            .map_or(Decimal::ZERO, |(m, _)| m.savings_left);
        facts.push(fact("Savings left", amount(left, code)));
        return facts;
    }
    if !cuts.is_empty() {
        return cut_facts(&cuts, None, code);
    }
    let drop = by_month.iter().find(|(m, v)| {
        m.free_left < m.free_before && v.iter().any(|v| v.from_free > Decimal::ZERO)
    });
    match drop {
        Some((month, _)) => vec![fact("Free money after", amount(month.free_left, code))],
        None => Vec::new(),
    }
}

/// Each month's short (`YYYY-MM`), in month order, with a card charge's
/// short counted in the month that pays the charge.
fn shorts_by_month(by_month: &[(&WishMonth, Vec<&Verdict>)]) -> Vec<(String, Decimal)> {
    let mut out: Vec<(String, Decimal)> = by_month
        .iter()
        .map(|(m, v)| {
            let key = m.month.get(..7).unwrap_or(&m.month).to_owned();
            (key, total(v, |v| v.short))
        })
        .collect();
    for (m, v) in by_month {
        let next_short = total(v, |v| v.next_short);
        if next_short > Decimal::ZERO {
            let next = format::add_months(&m.month, 1);
            match out.iter_mut().find(|(k, _)| *k == next) {
                Some((_, n)) => *n += next_short,
                None => out.push((next, next_short)),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A recurring wish or bill change, judged over every month it touches.
fn repeating_facts(by_month: &[(&WishMonth, Vec<&Verdict>)], code: &str) -> Vec<Fact> {
    let shorts = shorts_by_month(by_month);
    if let Some(i) = shorts.iter().position(|(_, n)| *n > Decimal::ZERO) {
        let (month, n) = &shorts[i];
        let mut facts = Vec::new();
        if let Some(j) = i.checked_sub(1) {
            facts.push(fact("Fits until", month_name(&shorts[j].0)));
        }
        facts.push(fact("Short from", month_name(month)));
        facts.push(fact("Short by", amount(*n, code)));
        return facts;
    }
    let freed = by_month.iter().find_map(|(m, v)| {
        let sum = total(v, |v| v.amount);
        (sum < Decimal::ZERO).then_some((m, -sum))
    });
    if let Some((month, n)) = freed {
        return vec![
            fact("Frees a month", amount(n, code)),
            fact("From", month_name(&month.month)),
        ];
    }
    let mut cuts = first_cuts(by_month);
    if cuts.is_empty() {
        cuts = by_month
            .iter()
            .map(|(_, mine)| merged_cuts(mine.iter().flat_map(|v| &v.next_cuts)))
            .find(|c| !c.is_empty())
            .unwrap_or_default();
    }
    let card = by_month
        .iter()
        .map(|(_, v)| total(v, |v| v.on_card))
        .find(|n| *n > Decimal::ZERO);
    let mut facts = Vec::new();
    if let Some(n) = card {
        facts.push(fact("On the cards a month", amount(n, code)));
    }
    facts.extend(cut_facts(&cuts, None, code));
    facts
}

/// How a one-off wish gets saved up, for the row's data fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saving {
    /// What this month puts aside.
    pub this_month: Decimal,
    /// `YYYY-MM-01` of the last month that sets money aside.
    pub saved_by: String,
    /// What is still unsaved when the due month comes, and that month,
    /// when the set-asides do not cover the wish in time.
    pub unsaved: Option<(Decimal, String)>,
}

/// The row's saving facts; `None` when nothing is set aside for it.
pub fn saving(row: &WishRow, months: &[WishMonth]) -> Option<Saving> {
    if row.wish.kind != WishKind::OneOff || row.not_judged.is_some() {
        return None;
    }
    let by_month = row_months(row, months);
    let saved_by = by_month
        .iter()
        .rev()
        .find(|(_, v)| {
            v.iter()
                .any(|v| v.set_aside && !v.skipped && v.amount > Decimal::ZERO)
        })
        .map(|(m, _)| m.month.clone())?;
    let bought = |v: &Verdict| !v.set_aside && v.amount > Decimal::ZERO;
    let unsaved: Decimal = by_month
        .iter()
        .flat_map(|(_, v)| v.iter())
        .filter(|v| bought(v))
        .map(|v| v.amount)
        .sum();
    let unsaved = (unsaved > Decimal::ZERO).then(|| {
        let due = by_month
            .iter()
            .find(|(_, v)| v.iter().any(|v| bought(v)))
            .map_or(saved_by.clone(), |(m, _)| m.month.clone());
        (unsaved, due)
    });
    Some(Saving {
        this_month: row.monthly_set_aside,
        saved_by,
        unsaved,
    })
}

/// What "free money" means on this page, with the savings pot the months
/// start from.
/// The savings fields: the account total, what the wishes already saved and
/// the room above that. The pot the months start from already leaves the
/// saved money out, so it is the wiggle room.
fn savings_fields(months: &[WishMonth], must_keep: Decimal) -> Option<(Decimal, Decimal, Decimal)> {
    let first = months.first()?;
    let free = first.savings_before;
    // What the bank accounts are short this month comes out of the room
    // first, as on the Overview.
    let room = free + first.free_before.min(Decimal::ZERO);
    Some((free + must_keep, must_keep, room))
}

/// What the wishes put on the cards this month, paid back on the 1st of
/// the next month, per card when there are several; `None` when the
/// wishes owe nothing. A card's regular spending (its bill) is left out:
/// that is paid for outside the wishes.
fn payback(month: &WishMonth, code: &str) -> Option<Vec<(String, String)>> {
    let owed: Vec<(&str, Decimal)> = month
        .cards
        .iter()
        .map(|c| (c.name.as_str(), c.used - c.regular))
        .filter(|(_, n)| *n > Decimal::ZERO)
        .collect();
    let total: Decimal = owed.iter().map(|(_, n)| *n).sum();
    if total <= Decimal::ZERO {
        return None;
    }
    let next = month_name(&format::add_months(&month.month, 1));
    let mut rows = vec![(format!("Pay back on 1 {next}"), amount(total, code))];
    if owed.len() > 1 {
        rows.extend(
            owed.iter()
                .map(|(name, n)| ((*name).to_owned(), amount(*n, code))),
        );
    }
    Some(rows)
}

/// A card purchase split over the cards in list order: each takes up to
/// what it has left on the statement (usage entered for another statement
/// counts as €0), the last takes the rest. `month` is the `YYYY-MM` of the
/// purchase's statement.
fn fill_cards(total: Decimal, cards: &[CardSettings], month: &str) -> Vec<Decimal> {
    let mut left = total.max(Decimal::ZERO);
    let last = cards.len().saturating_sub(1);
    cards
        .iter()
        .enumerate()
        .map(|(i, card)| {
            let used = if card.used_month.get(..7) == Some(month) {
                card.used
            } else {
                Decimal::ZERO
            };
            let take = if i == last {
                left
            } else {
                left.min((card.limit - used).max(Decimal::ZERO))
            };
            left -= take;
            take
        })
        .collect()
}

/// What a month could not pay, across all its verdicts.
fn month_short(month: &WishMonth) -> Decimal {
    month.verdicts.iter().map(|v| v.short).sum()
}

/// `YYYY-MM-DD` of today, local time.
pub(crate) fn today() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    )
}

/// What `save_wish` takes (`WishDraft` only deserializes).
#[derive(Serialize)]
pub(crate) struct Draft {
    pub(crate) wish_id: Option<String>,
    pub(crate) parent_id: Option<String>,
    pub(crate) name: String,
    pub(crate) amount: Decimal,
    pub(crate) currency: Currency,
    pub(crate) due_on: String,
    pub(crate) source: WishSource,
    pub(crate) kind: WishKind,
    pub(crate) must: bool,
    /// The event's dates, for a wish that is one.
    pub(crate) event: Option<EventDetails>,
}

#[derive(Serialize)]
pub(crate) struct DraftArgs {
    pub(crate) draft: Draft,
}

#[derive(Serialize)]
struct SavedArgs {
    wish_id: String,
    saved: Decimal,
}

#[derive(Serialize)]
struct MustArgs {
    wish_id: String,
    must: bool,
}

#[derive(Serialize)]
pub(crate) struct WishIdArgs {
    pub(crate) wish_id: String,
}

#[derive(Serialize)]
struct MoveArgs {
    wish_id: String,
    event_id: Option<String>,
}

#[derive(Serialize)]
struct EventDatesArgs {
    wish_id: String,
    starts_on: String,
    ends_on: String,
}

#[derive(Serialize)]
struct BoughtArgs {
    wish_id: String,
    purchase: WishPurchase,
}

#[derive(Serialize)]
struct StartArgs {
    wish_id: String,
    match_category: Option<String>,
}

#[derive(Serialize)]
struct SettingsArgs {
    settings: WishSettings,
}

/// A source as a `<select>` value: `card` or `account:<id>`.
fn source_value(source: &WishSource) -> String {
    match source {
        WishSource::Card => "card".into(),
        WishSource::Account { account_id } => format!("account:{account_id}"),
    }
}

fn parse_source(value: &str) -> WishSource {
    match value.strip_prefix("account:") {
        Some(id) if !id.is_empty() => WishSource::Account {
            account_id: id.to_owned(),
        },
        _ => WishSource::Card,
    }
}

fn error_or(error: String, fallback: &str) -> String {
    if error.trim().is_empty() || error == "unknown-error" {
        fallback.to_owned()
    } else {
        error
    }
}

/// `week`, `2 weeks`, `month`, `year`.
fn period(every: Repeat) -> &'static str {
    match every {
        Repeat::Weekly => "week",
        Repeat::Biweekly => "2 weeks",
        Repeat::Monthly => "month",
        Repeat::Yearly => "year",
    }
}

/// The add form's repeat `<select>` value.
fn parse_repeat(value: &str) -> Repeat {
    match value {
        "weekly" => Repeat::Weekly,
        "biweekly" => Repeat::Biweekly,
        "yearly" => Repeat::Yearly,
        _ => Repeat::Monthly,
    }
}

/// A bill the page can name or change.
#[derive(Clone, Debug)]
struct BillOption {
    id: String,
    name: String,
    amount: Decimal,
}

/// The bills a change can be judged on (the view lists them, with their
/// schedule), labelled `Gym · €30 per month`.
fn changeable_bills(
    bills: &[BillOption],
    changeable: &[ChangeableBill],
    code: &str,
) -> Vec<(BillOption, String)> {
    bills
        .iter()
        .filter_map(|b| {
            let every = changeable.iter().find(|c| c.bill_id == b.id)?.every;
            let label = format!(
                "{} · {} per {}",
                b.name,
                amount(b.amount, code),
                period(every)
            );
            Some((b.clone(), label))
        })
        .collect()
}

/// What the page reads from the dashboard.
#[derive(Clone, Debug, Default)]
pub(crate) struct Context {
    /// Spendable accounts: (id, name).
    accounts: Vec<(String, String)>,
    bills: Vec<BillOption>,
    /// Categories a new bill can match: known, and no bill's yet.
    categories: Vec<String>,
    /// The credit cards, in fill order (from the wishes view).
    cards: Vec<CardSettings>,
    /// The events a wish can be put into.
    events: Vec<EventOption>,
}

impl Context {
    /// The events in `code` a wish can go into: (id, name). One that is
    /// over takes only what was already bought.
    fn event_options(&self, code: &str, bought: bool) -> Vec<(String, String)> {
        self.events
            .iter()
            .filter(|e| e.currency.code == code && (bought || !e.over))
            .map(|e| (e.wish_id.clone(), e.name.clone()))
            .collect()
    }

    pub(crate) fn from(report: &Dashboard) -> Self {
        let mut accounts: Vec<(String, String)> = Vec::new();
        for bank in report
            .monthly_allocation
            .iter()
            .flat_map(|m| &m.currencies)
            .flat_map(|c| &c.banks)
            .filter(|b| b.included)
        {
            if !accounts.iter().any(|(id, _)| *id == bank.account_id) {
                accounts.push((bank.account_id.clone(), bank.name.clone()));
            }
        }
        let mut bills: Vec<BillOption> = Vec::new();
        let mut owned: Vec<String> = Vec::new();
        for row in report
            .subscriptions
            .rows
            .iter()
            .chain(report.next_month_subscriptions.iter().flat_map(|s| &s.rows))
        {
            owned.extend(
                row.category_match_categories
                    .iter()
                    .map(|c| c.to_lowercase()),
            );
            if !bills.iter().any(|b| b.id == row.bill_id) {
                bills.push(BillOption {
                    id: row.bill_id.clone(),
                    name: row.name.clone(),
                    amount: row.amount_max,
                });
            }
        }
        let categories = crate::pages::plan::plan_categories(report)
            .into_iter()
            .filter(|c| !owned.contains(&c.to_lowercase()))
            .collect();
        Self {
            accounts,
            bills,
            categories,
            cards: Vec::new(),
            events: Vec::new(),
        }
    }

    fn bill(&self, bill_id: &str) -> Option<&BillOption> {
        self.bills.iter().find(|b| b.id == bill_id)
    }

    fn source_label(&self, source: &WishSource) -> String {
        match source {
            WishSource::Card => "Credit card".into(),
            WishSource::Account { account_id } => self
                .accounts
                .iter()
                .find(|(id, _)| id == account_id)
                .map_or_else(|| "Bank account".into(), |(_, name)| name.clone()),
        }
    }

    fn source_options(&self) -> Vec<(String, String)> {
        std::iter::once(("card".to_owned(), "Credit card".to_owned()))
            .chain(
                self.accounts
                    .iter()
                    .map(|(id, name)| (format!("account:{id}"), name.clone())),
            )
            .collect()
    }
}

/// Screen state that outlives a reload.
#[derive(Clone, Copy)]
pub(crate) struct Ui {
    pub(crate) app: crate::AppState,
    pub(crate) busy: RwSignal<bool>,
    /// Feedback per target: (text, is error).
    pub(crate) messages: RwSignal<HashMap<String, (String, bool)>>,
    pub(crate) name: RwSignal<String>,
    pub(crate) amount: RwSignal<String>,
    pub(crate) due_on: RwSignal<String>,
    pub(crate) source: RwSignal<String>,
    pub(crate) parent: RwSignal<String>,
    /// The add form's "Must happen".
    pub(crate) must: RwSignal<bool>,
    /// The wish whose "Bought" form is open.
    pub(crate) buying: RwSignal<Option<String>>,
    pub(crate) buy_amount: RwSignal<String>,
    pub(crate) buy_on: RwSignal<String>,
    pub(crate) buy_source: RwSignal<String>,
    /// The bought form's amount per card, in card order.
    pub(crate) buy_cards: RwSignal<Vec<String>>,
    /// The wish whose drop is being confirmed.
    pub(crate) dropping: RwSignal<Option<String>>,
    /// The add form's kind: `one_off`, `repeats` or `bill`.
    pub(crate) kind: RwSignal<String>,
    /// `weekly`, `biweekly`, `monthly` or `yearly`.
    pub(crate) every: RwSignal<String>,
    pub(crate) ends_on: RwSignal<String>,
    pub(crate) bill: RwSignal<String>,
    /// The wish whose "Start" form is open.
    pub(crate) starting: RwSignal<Option<String>>,
    pub(crate) start_category: RwSignal<String>,
    /// The wish whose "Plan as event" form is open, and its dates.
    pub(crate) eventing: RwSignal<Option<String>>,
    pub(crate) event_starts: RwSignal<String>,
    pub(crate) event_ends: RwSignal<String>,
    /// The wish whose "Add to event" form is open, and the event chosen.
    pub(crate) joining: RwSignal<Option<String>>,
    pub(crate) join_event: RwSignal<String>,
    /// The board box that is open over the page.
    pub(crate) selected: RwSignal<Option<String>>,
    /// The wish being dragged on the board.
    pub(crate) dragging: RwSignal<Option<String>>,
    /// The wish whose payment is being picked.
    pub(crate) ask: RwSignal<Option<Ask>>,
}

impl Ui {
    pub(crate) fn new() -> Self {
        Self {
            app: state(),
            busy: RwSignal::new(false),
            messages: RwSignal::new(HashMap::new()),
            name: RwSignal::new(String::new()),
            amount: RwSignal::new(String::new()),
            due_on: RwSignal::new(String::new()),
            source: RwSignal::new("card".into()),
            parent: RwSignal::new(String::new()),
            must: RwSignal::new(false),
            buying: RwSignal::new(None),
            buy_amount: RwSignal::new(String::new()),
            buy_on: RwSignal::new(String::new()),
            buy_source: RwSignal::new(String::new()),
            buy_cards: RwSignal::new(Vec::new()),
            dropping: RwSignal::new(None),
            kind: RwSignal::new("one_off".into()),
            every: RwSignal::new("monthly".into()),
            ends_on: RwSignal::new(String::new()),
            bill: RwSignal::new(String::new()),
            starting: RwSignal::new(None),
            start_category: RwSignal::new(String::new()),
            eventing: RwSignal::new(None),
            event_starts: RwSignal::new(String::new()),
            event_ends: RwSignal::new(String::new()),
            joining: RwSignal::new(None),
            join_event: RwSignal::new(String::new()),
            selected: RwSignal::new(None),
            dragging: RwSignal::new(None),
            ask: RwSignal::new(None),
        }
    }

    /// Close every row form.
    pub(crate) fn close_forms(self) {
        self.buying.set(None);
        self.dropping.set(None);
        self.starting.set(None);
        self.eventing.set(None);
        self.joining.set(None);
    }

    pub(crate) fn set_message(self, target: &str, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.messages.update(|m| {
            m.insert(target.to_owned(), (text, error));
        });
    }

    /// Run a write command; on success show `done` and reload every screen.
    pub(crate) fn write<A: Serialize + 'static>(
        self,
        target: String,
        cmd: &'static str,
        args: A,
        done: &'static str,
        after: impl FnOnce() + 'static,
    ) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.set_message(&target, "", false);
        spawn_local(async move {
            match invoke::<IgnoredAny>(cmd, &args).await {
                Ok(_) => {
                    self.set_message(&target, done, false);
                    after();
                    self.app.refresh();
                }
                Err(e) => self.set_message(&target, error_or(e, "Could not save."), true),
            }
            self.busy.set(false);
        });
    }
}

/// The receipt or error of `target`, when there is one.
pub(crate) fn feedback(ui: Ui, target: String) -> impl IntoView {
    move || {
        ui.messages.with(|m| {
            m.get(&target).filter(|(text, _)| !text.is_empty()).map(|(text, error)| {
                let class = cn([
                    FEEDBACK_LINE,
                    if *error { "text-negative-fg" } else { "text-positive-fg" },
                ]);
                view! { <p role=if *error { "alert" } else { "status" } class=class>{text.clone()}</p> }
            })
        })
    }
}

fn select(
    id: String,
    value: RwSignal<String>,
    options: Vec<(String, String)>,
) -> impl IntoView + use<> {
    let options = options
        .into_iter()
        .map(|(v, l)| {
            let selected = v == value.get_untracked();
            view! { <option value=v selected=selected>{l}</option> }
        })
        .collect_view();
    view! {
        <div class=SELECT_WRAP>
            <select
                id=id
                class=cn([CONTROL, "appearance-none pr-9"])
                prop:value=move || value.get()
                on:change=move |e| value.set(event_target_value(&e))
            >
                {options}
            </select>
            <span aria-hidden="true" class=SELECT_CHEVRON>
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

#[component]
pub fn Wishes() -> impl IntoView {
    let ui = Ui::new();
    let revision = ui.app.revision;
    let view_data = load::<WishesView, _>("wishes", move || {
        revision.track();
        NoArgs {}
    });
    move || {
        let ctx = ui.app.report.data.with(|d| {
            d.as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(Context::from)
                .unwrap_or_default()
        });
        view_data.data.with(|data| match data {
            None => view! {
                <div class=STACK>
                    <Skeleton class=SKELETON_SHORT />
                    <Skeleton class=SKELETON_CARD />
                </div>
            }
            .into_any(),
            Some(Err(e)) => view! { <Problem error=e.clone() /> }.into_any(),
            Some(Ok(view)) => page(ui, view, &ctx).into_any(),
        })
    }
}

fn page(ui: Ui, view: &WishesView, ctx: &Context) -> impl IntoView + use<> {
    let months = StoredValue::new(view.months.clone());
    let ctx = StoredValue::new(Context {
        cards: view.settings.cards.clone(),
        events: view.events.clone(),
        ..ctx.clone()
    });
    let currency = view.currency.clone();
    let (open, done): (Vec<WishRow>, Vec<WishRow>) = view
        .wishes
        .iter()
        .cloned()
        .partition(|r| r.wish.status == WishStatus::Planned);
    let parents: Vec<(String, String)> = open
        .iter()
        .filter(|r| r.wish.parent_id.is_none() && r.wish.kind == WishKind::OneOff)
        .map(|r| (r.wish.wish_id.clone(), r.wish.name.clone()))
        .chain(
            view.events
                .iter()
                .filter(|e| e.currency == view.currency && !e.over)
                .map(|e| (e.wish_id.clone(), e.name.clone())),
        )
        .collect();
    let empty = open.is_empty() && done.is_empty();
    let done_count = done.len();
    // The board: every wish a box in the column of its state; the box
    // picked opens over the page with all its controls. A wish dragged to
    // Bought asks for the payment that bought it.
    let mut columns: [(Decimal, Vec<AnyView>); 4] = Default::default();
    let bought = done
        .iter()
        .filter(|r| r.wish.status != WishStatus::Dropped)
        .take(DONE_ON_BOARD);
    for row in open.iter().chain(bought) {
        let column = &mut columns[wish_stage(row) as usize];
        if row.wish.status == WishStatus::Planned {
            column.0 += row.wish.amount;
        }
        column
            .1
            .push(wish_box(ui, row, &ctx.get_value()).into_any());
    }
    let asks = StoredValue::new(
        open.iter()
            .filter(|row| can_buy(row))
            .map(|row| Ask {
                id: row.wish.wish_id.clone(),
                name: row.wish.name.clone(),
                currency: row.wish.currency.code.clone(),
                incoming: false,
                amount: row.wish.amount,
                taken: None,
            })
            .collect::<Vec<_>>(),
    );
    let board = STAGES
        .into_iter()
        .zip(columns)
        .map(|((title, tone), (sum, boxes))| {
            let count = boxes.len();
            let total = (sum > Decimal::ZERO).then(|| amount(sum, &currency.code));
            let bought = title == STAGES[Stage::Done as usize].0;
            let accepts = Signal::derive(move || bought && ui.dragging.with(Option::is_some));
            let on_drop = Callback::new(move |()| {
                let Some(id) = ui.dragging.get_untracked() else {
                    return;
                };
                ui.ask.set(asks.with_value(|all| all.iter().find(|a| a.id == id).cloned()));
            });
            match total {
                Some(total) => view! {
                    <BoardColumn title=title tone=tone count=count total=total accepts=accepts on_drop=on_drop>{boxes}</BoardColumn>
                }
                .into_any(),
                None => view! {
                    <BoardColumn title=title tone=tone count=count accepts=accepts on_drop=on_drop>{boxes}</BoardColumn>
                }
                .into_any(),
            }
        })
        .collect_view();
    let open = StoredValue::new(open);
    let picked = move || {
        let id = ui.selected.get()?;
        let row = open.with_value(|rows| rows.iter().find(|r| r.wish.wish_id == id).cloned())?;
        let title = row.wish.name.clone();
        let close = Callback::new(move |()| {
            ui.close_forms();
            ui.selected.set(None);
        });
        Some(view! {
            <Popup title=title class="w-[min(94vw,60rem)]" on_close=close>
                <div class="grid">{wish_row(ui, row, months, ctx)}</div>
            </Popup>
        })
    };
    let buy = Callback::new(move |(ask, paid): (Ask, SearchRow)| {
        let purchase = WishPurchase {
            amount: paid.amount.amount,
            on: paid.occurred_on.chars().take(10).collect(),
            source: WishSource::Account {
                account_id: paid.account_id,
            },
            cards: Vec::new(),
            transaction_id: Some(paid.transaction_id.clone()),
        };
        ui.write(
            "dropped".into(),
            "mark_wish_bought",
            BoughtArgs {
                wish_id: ask.id,
                purchase,
            },
            "Marked as bought.",
            move || ui.ask.set(None),
        );
    });
    let done_rows = done_list(ui, done, ctx);
    let month_cards = view
        .months
        .iter()
        .map(|m| month_card(m, &currency.code))
        .collect_view();
    let has_months = !view.months.is_empty();
    let savings = savings_fields(&view.months, view.savings_must_keep).map(|(total, saved, room)| {
        let negative = room < Decimal::ZERO;
        let (total, saved, room) = (
            crate::ds::money(total, &currency),
            crate::ds::money(saved, &currency),
            crate::ds::money(room, &currency),
        );
        view! {
            <div class="mt-3 grid grid-cols-3 gap-3">
                <Stat label="Savings">{total}</Stat>
                <Stat label="Saved for wishes">{saved}</Stat>
                <Stat label="Wiggle room" tone=if negative { Tone::Negative } else { Tone::Neutral }>{room}</Stat>
            </div>
        }
    });
    // The cards and the fund are one stored record: each form saves what is
    // typed in the other too, so neither discards the other's edits.
    let shared = SettingsForms {
        rows: RwSignal::new(card_rows(view)),
        months: RwSignal::new(view.settings.emergency_months.to_string()),
        groups: RwSignal::new(view.settings.essential_groups.join(", ")),
        stored: StoredValue::new(view.settings.clone()),
    };
    let strip = card_strip(ui, view, &ctx.get_value(), shared);
    let fund = fund_block(ui, view, shared);
    let form = add_form(
        ui,
        &ctx.get_value(),
        &view.changeable_bills,
        parents,
        currency,
    );
    view! {
        <div class=STACK>
            <section aria-label="Wishes board" class=STACK_MD>
                <div>
                    <Eyebrow>"WISHES"</Eyebrow>
                    <h2 class=BOARD_TITLE>"From someday to bought"</h2>
                </div>
                {savings}
                <p class=HINT>"Open a box for its details. Drag one to Bought to name the payment that bought it."</p>
                {feedback(ui, "dropped".into())}
                <div class=BOARD>{board}</div>
                {empty.then(|| view! { <p class=MUTED>"No wishes yet."</p> })}
            </section>
            {picked}
            <PaymentPicker
                ask=ui.ask
                busy=ui.busy
                on_pick=buy
                otherwise="Newest first, the ones of the same amount on top. Paid by card or not in the bank yet? Close this, open the box and use Bought."
            />
            <Card>
                <Eyebrow>"NEW WISH"</Eyebrow>
                <h2 class=PANEL_TITLE>"Add a wish"</h2>
                {form}
            </Card>
            {(done_count > 0).then(|| view! {
                <Card>
                    <Disclosure class="text-sm" summary_class="font-medium" summary=format!("Bought and dropped ({done_count})")>
                        <div class="mt-2">{done_rows}</div>
                    </Disclosure>
                </Card>
            })}
            {has_months.then(|| view! {
                <Card>
                    <Disclosure summary_class="text-sm font-medium" summary="Month by month">
                        <div class="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">{month_cards}</div>
                    </Disclosure>
                </Card>
            })}
            {strip}
            {fund}
        </div>
    }
}

/// How many done wishes the board's last column shows; the rest are in
/// the list under it.
const DONE_ON_BOARD: usize = 6;

/// Where a wish stands on the way to being bought: the board's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Nothing saved and nothing set aside this month.
    Someday,
    Saving,
    /// Fully saved, not bought yet.
    Ready,
    Done,
}

pub(crate) const STAGES: [(&str, Tone); 4] = [
    ("Someday", Tone::Neutral),
    ("Saving up", Tone::Accent),
    ("Ready to buy", Tone::Positive),
    ("Bought", Tone::Neutral),
];

/// A wish that is bought with one payment: planned, one-off, whole (no
/// parts) and not an event, which is spent over its days.
pub(crate) fn can_buy(row: &WishRow) -> bool {
    let wish = &row.wish;
    wish.status == WishStatus::Planned
        && wish.kind == WishKind::OneOff
        && wish.event.is_none()
        && row.children.is_empty()
}

/// What a row has saved, its sub-wishes included (as the header counts it).
pub(crate) fn row_saved(row: &WishRow) -> Decimal {
    row.wish.saved + row.children.iter().map(|c| c.wish.saved).sum::<Decimal>()
}

pub(crate) fn wish_stage(row: &WishRow) -> Stage {
    let wish = &row.wish;
    if wish.status != WishStatus::Planned {
        Stage::Done
    } else if wish.kind != WishKind::OneOff {
        // A repeating wish or bill change is not saved for: it waits to start.
        Stage::Someday
    } else if wish.amount > Decimal::ZERO && row_saved(row) >= wish.amount {
        Stage::Ready
    } else if row_saved(row) > Decimal::ZERO || row.monthly_set_aside > Decimal::ZERO {
        Stage::Saving
    } else {
        Stage::Someday
    }
}

/// The share of a wish that is saved, 0–100.
pub(crate) fn saved_percent(saved: Decimal, amount: Decimal) -> u32 {
    use rust_decimal::prelude::ToPrimitive;
    if amount <= Decimal::ZERO {
        return 0;
    }
    (saved.max(Decimal::ZERO) / amount * Decimal::ONE_HUNDRED)
        .min(Decimal::ONE_HUNDRED)
        .floor()
        .to_u32()
        .unwrap_or(0)
}

/// A wish as a box on the board: name, amount, how it stands, how much is
/// saved. Picking it opens its controls under the board.
fn wish_box(ui: Ui, row: &WishRow, ctx: &Context) -> impl IntoView + use<> {
    let wish = &row.wish;
    let code = wish.currency.code.clone();
    let id = wish.wish_id.clone();
    let planned = wish.status == WishStatus::Planned;
    let (tone, chip, facts) = if planned {
        let (tone, chip) = if row.not_judged.is_some() {
            (Tone::Outline, "Not judged")
        } else if row.short > Decimal::ZERO {
            (Tone::Negative, "Short")
        } else if row.tradeoffs {
            (Tone::Warning, "With trade-offs")
        } else {
            (Tone::Positive, "Affordable")
        };
        let mut facts = vec![("By".to_owned(), format::date(&wish.due_on))];
        if row.monthly_set_aside > Decimal::ZERO {
            facts.push((
                "This month".to_owned(),
                format!("+{}", amount(row.monthly_set_aside, &code)),
            ));
        }
        (tone, chip, facts)
    } else {
        let (tone, chip, facts) = done_facts(wish, ctx);
        (tone, chip, facts.into_iter().skip(1).take(1).collect())
    };
    let saved = row_saved(row);
    let percent = saved_percent(saved, wish.amount);
    let progress = (planned && wish.kind == WishKind::OneOff).then(|| {
        let label = format!("{} saved", amount(saved, &code));
        view! {
            <span class=STACK_XS>
                <span role="img" aria-label=label.clone() class=METER>
                    <i class="block h-full rounded-pill bg-positive" style:width=format!("{percent}%")></i>
                </span>
                <span class=METER_SCALE>
                    <span>{label}</span>
                    <span class="tabular-nums">{format!("{percent}%")}</span>
                </span>
            </span>
        }
    });
    let cost = wish
        .bought
        .as_ref()
        .filter(|_| !planned)
        .map_or(wish.amount, |p| p.amount);
    let must = (planned && wish.must)
        .then(|| view! { <Badge tone=Tone::Accent class=BADGE_TIGHT>"Must"</Badge> });
    let parts = (!row.children.is_empty()).then(|| {
        let text = crate::pages::plural(row.children.len(), "part");
        view! { <Badge tone=Tone::Outline class=BADGE_TIGHT>{text}</Badge> }
    });
    let pick = id.clone();
    let movable = can_buy(row);
    let drag_id = id.clone();
    let body = view! {
        <span class=NAME_ROW>
            <strong class=CARD_NAME>{wish.name.clone()}</strong>
            <strong class=FIGURE>{amount(cost, &code)}</strong>
        </span>
        <span class="flex flex-wrap items-center gap-1.5">
            {must}
            <Badge tone=tone class=BADGE_TIGHT>{chip}</Badge>
            {parts}
        </span>
        {progress}
        <Facts rows=facts />
    };
    if planned {
        view! {
            <button
                type="button"
                class=cn([
                    BOARD_CARD,
                    "text-left transition-colors hover:border-accent",
                    if movable { "cursor-grab active:cursor-grabbing" } else { "" },
                ])
                draggable=movable.to_string()
                on:dragstart=move |e| {
                    if let Some(data) = e.data_transfer() {
                        let _ = data.set_data("text/plain", &drag_id);
                    }
                    ui.dragging.set(Some(drag_id.clone()));
                }
                on:dragend=move |_| ui.dragging.set(None)
                on:click=move |_| {
                    ui.close_forms();
                    ui.selected.set(Some(pick.clone()));
                }
            >
                {body}
            </button>
        }
        .into_any()
    } else {
        view! { <div class=BOARD_CARD>{body}</div> }.into_any()
    }
}

/// One editable row of the cards form.
#[derive(Clone, Copy)]
struct CardRow {
    /// Stable within the form, for keyed rendering and element ids.
    key: usize,
    card_id: StoredValue<String>,
    name: RwSignal<String>,
    limit: RwSignal<String>,
    used: RwSignal<String>,
    bill: RwSignal<String>,
}

impl CardRow {
    fn new(key: usize, card: &CardSettings) -> Self {
        Self {
            key,
            card_id: StoredValue::new(card.card_id.clone()),
            name: RwSignal::new(card.name.clone()),
            limit: RwSignal::new(card.limit.to_string()),
            used: RwSignal::new(card.used.to_string()),
            bill: RwSignal::new(card.bill_id.clone().unwrap_or_default()),
        }
    }

    fn blank(key: usize) -> Self {
        Self {
            key,
            card_id: StoredValue::new(String::new()),
            name: RwSignal::new(String::new()),
            limit: RwSignal::new(String::new()),
            used: RwSignal::new("0".into()),
            bill: RwSignal::new(String::new()),
        }
    }

    /// The row as settings, with its usage entered for statement `month`; `None`
    /// when a box does not hold an amount.
    fn settings(&self, month: &str) -> Option<CardSettings> {
        let limit = parse_amount(&self.limit.get_untracked()).ok()?;
        let used = parse_amount(&self.used.get_untracked()).ok()?;
        let bill_id = self.bill.get_untracked();
        Some(CardSettings {
            card_id: self.card_id.get_value(),
            name: self.name.get_untracked().trim().to_owned(),
            limit,
            used,
            used_month: month.to_owned(),
            bill_id: (!bill_id.is_empty()).then_some(bill_id),
        })
    }
}

/// The stale notice: which cards still carry a closed statement's usage.
fn stale_notice(view: &WishesView) -> Option<String> {
    if view.stale_cards.is_empty() {
        return None;
    }
    let names: Vec<&str> = view
        .stale_cards
        .iter()
        .map(|id| {
            view.settings
                .cards
                .iter()
                .find(|c| c.card_id == *id)
                .map_or(id.as_str(), |c| c.name.as_str())
        })
        .collect();
    Some(format!(
        "Usage for {} was entered for a statement that has closed; enter this statement's usage.",
        names.join(", ")
    ))
}

/// The card rows of the stored settings (one default card when none).
fn card_rows(view: &WishesView) -> Vec<CardRow> {
    let cards = if view.settings.cards.is_empty() {
        WishSettings::default().cards
    } else {
        view.settings.cards.clone()
    };
    cards
        .iter()
        .enumerate()
        .map(|(i, c)| CardRow::new(i, c))
        .collect()
}

/// What the cards form and the fund form have typed, and what is stored.
#[derive(Clone, Copy)]
struct SettingsForms {
    rows: RwSignal<Vec<CardRow>>,
    months: RwSignal<String>,
    groups: RwSignal<String>,
    stored: StoredValue<WishSettings>,
}

impl SettingsForms {
    /// The cards as typed for `statement`, or `None` while a row is invalid.
    fn cards(self, statement: &str) -> Option<Vec<CardSettings>> {
        self.rows
            .with_untracked(|rows| rows.iter().map(|r| r.settings(statement)).collect())
    }

    /// The fund's months as typed, or `None` while they are invalid.
    fn months(self) -> Option<u32> {
        crate::pages::goals::parse_months(&self.months.get_untracked())
    }

    fn groups(self) -> Vec<String> {
        crate::pages::goals::parse_groups(&self.groups.get_untracked())
    }
}

fn card_strip(
    ui: Ui,
    view: &WishesView,
    ctx: &Context,
    shared: SettingsForms,
) -> impl IntoView + use<> {
    let rows = shared.rows;
    let next_key = StoredValue::new(rows.with_untracked(Vec::len));
    let bills: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "No card bill".to_owned()))
            .chain(ctx.bills.iter().map(|b| (b.id.clone(), b.name.clone())))
            .collect(),
    );
    let stale = stale_notice(view).map(|text| {
        view! { <p role="status" class="mt-2 text-sm text-warning-fg">{text}</p> }
    });
    // Usage is typed for the statement open today (26th to 25th).
    let statement = statement_month(&today());
    let period = StoredValue::new(statement_period(&statement));
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Some(cards) = shared.cards(&statement) else {
            ui.set_message("card", "Enter each card's limit and the amount used.", true);
            return;
        };
        ui.write(
            "card".into(),
            "save_wish_settings",
            SettingsArgs {
                settings: WishSettings {
                    cards,
                    // A half-typed fund field keeps the stored value.
                    emergency_months: shared
                        .months()
                        .unwrap_or_else(|| shared.stored.with_value(|s| s.emergency_months)),
                    essential_groups: shared.groups(),
                },
            },
            "Cards saved.",
            || {},
        );
    };
    let add = move |_| {
        let key = next_key.get_value();
        next_key.set_value(key + 1);
        rows.update(|r| r.push(CardRow::blank(key)));
    };
    let only_one = Signal::derive(move || rows.with(|r| r.len() <= 1));
    let row_view = move |row: CardRow| {
        let key = row.key;
        let (name_id, limit_id, used_id, bill_id) = (
            format!("card-name-{key}"),
            format!("card-limit-{key}"),
            format!("card-used-{key}"),
            format!("card-bill-{key}"),
        );
        let remove = move |_| rows.update(|r| r.retain(|x| x.key != key));
        view! {
            <div class="grid items-start gap-3 border-t border-line pt-3 first:border-t-0 first:pt-0 sm:grid-cols-2 lg:grid-cols-5">
                <Field label="Name" id=name_id.clone()>
                    <input
                        id=name_id
                        type="text"
                        required
                        class=CONTROL
                        prop:value=move || row.name.get()
                        on:input=move |e| row.name.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Limit" id=limit_id.clone()>
                    <input
                        id=limit_id
                        type="text"
                        inputmode="decimal"
                        pattern=AMOUNT_PATTERN
                        required
                        class=CONTROL
                        prop:value=move || row.limit.get()
                        on:input=move |e| row.limit.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Used this statement" id=used_id.clone() hint=format!("{}.", period.get_value())>
                    <input
                        id=used_id
                        type="text"
                        inputmode="decimal"
                        pattern=AMOUNT_PATTERN
                        required
                        class=CONTROL
                        prop:value=move || row.used.get()
                        on:input=move |e| row.used.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Card bill" id=bill_id.clone() hint="The bill the card is paid by.">
                    {select(bill_id, row.bill, bills.get_value())}
                </Field>
                <Button
                    variant=Variant::Ghost
                    class=cn(["justify-self-start", LABEL_OFFSET])
                    disabled=only_one
                    on_click=Callback::new(remove)
                >
                    "Remove"
                </Button>
            </div>
        }
    };
    view! {
        <Card>
            <Eyebrow>"CREDIT CARDS"</Eyebrow>
            <h2 class=PANEL_TITLE_HELP>"What the wishes may lean on"<Help text="Charges fill the cards in this order." /></h2>
            {stale}
            <form class=LIST_STACK on:submit=submit>
                <For each=move || rows.get() key=|r| r.key children=row_view />
                <div class=CHIPS>
                    <Button variant=Variant::Secondary on_click=Callback::new(add)>"Add card"</Button>
                    <Button submit=true disabled=Signal::derive(move || ui.busy.get())>"Save cards"</Button>
                </div>
            </form>
            {feedback(ui, "card".into())}
        </Card>
    }
}

/// The emergency fund: how many months of bare living costs to hold and
/// which spending groups are bare living. It takes what the wishes leave.
fn fund_block(ui: Ui, view: &WishesView, shared: SettingsForms) -> impl IntoView + use<> {
    let (months, groups) = (shared.months, shared.groups);
    // Every group a budget plan or the owner named.
    let group_options = state().report.data.with_untracked(|report| {
        let mut names: Vec<String> = report
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .flat_map(|r| {
                r.monthly_budgets
                    .iter()
                    .flat_map(|b| b.plan.groups.iter().map(|g| g.name.clone()))
                    .chain(r.custom_spending_groups.iter().cloned())
            })
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        names
    });
    let statement = statement_month(&today());
    let unit = view.currency.clone();
    let money = move |value: Decimal| format::money_dp(value, &unit.code, 0);
    let summary = match view.goals.as_ref().and_then(|g| g.fund.as_ref()) {
        Some(fund) => view! {
            <FactsRow
                class="mt-3"
                rows=vec![
                    ("Bills".to_owned(), money(fund.fixed_monthly)),
                    ("Essentials".to_owned(), money(fund.essentials_monthly)),
                    (
                        "Bare living a month".to_owned(),
                        money(fund.fixed_monthly + fund.essentials_monthly),
                    ),
                    ("Target".to_owned(), money(fund.target)),
                    ("Held".to_owned(), money(fund.balance)),
                ]
            />
        }
        .into_any(),
        None if view.settings.emergency_months == 0 => {
            view! { <Badge tone=Tone::Outline class="mt-3">"Switched off"</Badge> }.into_any()
        }
        None => view! {
            <span title="Sized once this month's money plan and a few months of spending history exist.">
                <Badge tone=Tone::Outline class="mt-3">"Not sized yet"</Badge>
            </span>
        }
        .into_any(),
    };
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Some(emergency_months) = shared.months() else {
            ui.set_message("fund", "Months must be a whole number from 0 to 36.", true);
            return;
        };
        ui.write(
            "fund".into(),
            "save_wish_settings",
            SettingsArgs {
                settings: WishSettings {
                    // Half-typed card rows keep the stored cards.
                    cards: shared
                        .cards(&statement)
                        .unwrap_or_else(|| shared.stored.with_value(|s| s.cards.clone())),
                    emergency_months,
                    essential_groups: shared.groups(),
                },
            },
            "Emergency fund saved.",
            || {},
        );
    };
    view! {
        <Card>
            <Eyebrow>"EMERGENCY FUND"</Eyebrow>
            <h2 class=PANEL_TITLE_HELP>"Months of bare living to hold"<Help text="A savings goal without a date: it takes what the wishes leave. Rent changes and new subscriptions planned as wishes move its target. The held amount is the savings pot beyond what is saved for wishes." /></h2>
            {summary}
            <form class="mt-4 grid items-start gap-3 sm:grid-cols-[8rem_1fr_auto]" on:submit=submit>
                <Field label="Months" id="fund-months" hint="0 switches it off.">
                    <input
                        id="fund-months"
                        type="text"
                        inputmode="numeric"
                        pattern="[0-9]{1,2}"
                        required
                        class=CONTROL
                        prop:value=move || months.get()
                        on:input=move |e| months.set(event_target_value(&e))
                    />
                </Field>
                <Field
                    label="Bare living groups"
                    id="fund-groups"
                    hint="The spending groups you cannot live without. Bills always count."
                >
                    <crate::ds::MultiSelect id="fund-groups" label="Bare living groups" value=groups options=group_options />
                </Field>
                <Button
                    submit=true
                    class=LABEL_OFFSET
                    disabled=Signal::derive(move || ui.busy.get())
                >
                    "Save fund"
                </Button>
            </form>
            {feedback(ui, "fund".into())}
        </Card>
    }
}

/// A decimal amount input bound to `value`.
fn number(id: &'static str, value: RwSignal<String>) -> impl IntoView + use<> {
    view! {
        <input
            id=id
            type="text"
            inputmode="decimal"
            pattern=AMOUNT_PATTERN
            required
            placeholder="0.00"
            class=CONTROL
            prop:value=move || value.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

/// A date input bound to `value`.
fn date_input(id: &'static str, value: RwSignal<String>, required: bool) -> impl IntoView + use<> {
    view! {
        <input
            id=id
            type="date"
            required=required
            class=CONTROL
            prop:value=move || value.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

fn add_form(
    ui: Ui,
    ctx: &Context,
    changeable: &[ChangeableBill],
    parents: Vec<(String, String)>,
    currency: Currency,
) -> impl IntoView + use<> {
    let sources = StoredValue::new(ctx.source_options());
    let parent_options: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Nothing".to_owned()))
            .chain(parents)
            .collect(),
    );
    let bills = changeable_bills(&ctx.bills, changeable, &currency.code);
    let bill_options: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Choose a bill".to_owned()))
            .chain(bills.iter().map(|(b, label)| (b.id.clone(), label.clone())))
            .collect(),
    );
    let bill_names = StoredValue::new(bills.into_iter().map(|(b, _)| b).collect::<Vec<_>>());
    let kinds = vec![
        ("one_off".to_owned(), "One-off".to_owned()),
        ("repeats".to_owned(), "Repeats".to_owned()),
        ("bill".to_owned(), "Change a bill".to_owned()),
    ];
    let repeats = StoredValue::new(vec![
        ("weekly".to_owned(), "Every week".to_owned()),
        ("biweekly".to_owned(), "Every 2 weeks".to_owned()),
        ("monthly".to_owned(), "Every month".to_owned()),
        ("yearly".to_owned(), "Every year".to_owned()),
    ]);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(amount) = parse_amount(&ui.amount.get_untracked()) else {
            ui.set_message("add", "Enter an amount.", true);
            return;
        };
        let ends_on = Some(ui.ends_on.get_untracked()).filter(|e| !e.is_empty());
        let mut name = ui.name.get_untracked().trim().to_owned();
        let (kind, parent) = match ui.kind.get_untracked().as_str() {
            "repeats" => (
                WishKind::Recurring {
                    every: parse_repeat(&ui.every.get_untracked()),
                    ends_on,
                },
                None,
            ),
            "bill" => {
                let bill_id = ui.bill.get_untracked();
                let Some(bill) =
                    bill_names.with_value(|b| b.iter().find(|b| b.id == bill_id).cloned())
                else {
                    ui.set_message("add", "Choose a bill.", true);
                    return;
                };
                if name.is_empty() {
                    name = bill.name;
                }
                (WishKind::BillChange { bill_id, ends_on }, None)
            }
            _ => {
                let parent = ui.parent.get_untracked();
                (WishKind::OneOff, (!parent.is_empty()).then_some(parent))
            }
        };
        let source = if matches!(kind, WishKind::BillChange { .. }) {
            WishSource::Card
        } else {
            parse_source(&ui.source.get_untracked())
        };
        let draft = Draft {
            wish_id: None,
            parent_id: parent,
            name,
            amount,
            currency: currency.clone(),
            due_on: ui.due_on.get_untracked(),
            source,
            kind,
            must: ui.must.get_untracked(),
            event: None,
        };
        ui.write(
            "add".into(),
            "save_wish",
            DraftArgs { draft },
            "Wish added.",
            move || {
                for field in [
                    ui.name, ui.amount, ui.due_on, ui.parent, ui.ends_on, ui.bill,
                ] {
                    field.set(String::new());
                }
                ui.must.set(false);
            },
        );
    };
    // The fields that differ by kind.
    let fields = move || {
        match ui.kind.get().as_str() {
        "repeats" => view! {
            <Field label="Each time" id="wish-amount">{number("wish-amount", ui.amount)}</Field>
            <Field label="Repeats" id="wish-every">
                {select("wish-every".into(), ui.every, repeats.get_value())}
            </Field>
            <Field label="First date" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Ends" id="wish-ends" hint="Optional.">{date_input("wish-ends", ui.ends_on, false)}</Field>
            <Field label="Pay with" id="wish-source">
                {select("wish-source".into(), ui.source, sources.get_value())}
            </Field>
        }
        .into_any(),
        "bill" => view! {
            <Field label="Bill" id="wish-bill">
                {select("wish-bill".into(), ui.bill, bill_options.get_value())}
            </Field>
            <Field label="New amount" id="wish-amount" hint="Per payment; 0 cancels the bill.">
                {number("wish-amount", ui.amount)}
            </Field>
            <Field label="From" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Ends" id="wish-ends" hint="Optional.">{date_input("wish-ends", ui.ends_on, false)}</Field>
        }
        .into_any(),
        _ => view! {
            <Field label="Amount" id="wish-amount" hint="For a wish with parts, the most it may cost.">
                {number("wish-amount", ui.amount)}
            </Field>
            <Field label="Date" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Pay with" id="wish-source">
                {select("wish-source".into(), ui.source, sources.get_value())}
            </Field>
            <Field label="Part of" id="wish-parent">
                {select("wish-parent".into(), ui.parent, parent_options.get_value())}
            </Field>
        }
        .into_any(),
    }
    };
    view! {
        <form class="mt-4 grid items-start gap-3 sm:grid-cols-2 lg:grid-cols-3" on:submit=submit>
            <Field label="Kind" id="wish-kind">{select("wish-kind".into(), ui.kind, kinds)}</Field>
            <Field label="Name" id="wish-name">
                <input
                    id="wish-name"
                    required=move || ui.kind.get() != "bill"
                    maxlength="128"
                    placeholder=move || if ui.kind.get() == "bill" { "The bill's name" } else { "e.g. Winter jacket" }
                    class=CONTROL
                    prop:value=move || ui.name.get()
                    on:input=move |e| ui.name.set(event_target_value(&e))
                />
            </Field>
            {fields}
            <label class=CHECK_LABEL_START>
                <Checkbox
                    checked=Signal::derive(move || ui.must.get())
                    on_change=Callback::new(move |v| ui.must.set(v))
                />
                "Must happen"
            </label>
            <Button submit=true variant=Variant::Secondary class="sm:col-start-1" disabled=Signal::derive(move || ui.busy.get())>
                <Lucide icon=Icon::Plus size=15 />
                "Add wish"
            </Button>
        </form>
        {feedback(ui, "add".into())}
    }
}

/// A top-level planned wish, with its sub-wishes.
pub(crate) fn wish_row(
    ui: Ui,
    row: WishRow,
    months: StoredValue<Vec<WishMonth>>,
    ctx: StoredValue<Context>,
) -> impl IntoView + use<> {
    let code = row.wish.currency.code.clone();
    let parent = !row.children.is_empty();
    let parent_must = row.wish.must;
    let in_event = row.wish.event.is_some();
    let family =
        parent.then(|| {
            let over = row.over_cap.then(|| view! {
            <span role="status"><Badge tone=Tone::Warning>"Parts exceed the cap"</Badge></span>
        });
            view! {
                <div class=FACT_ROW>
                    <FactsRow rows=vec![
                        ("Cap".to_owned(), amount(row.wish.amount, &code)),
                        ("Rest".to_owned(), amount(row.rest, &code)),
                    ] />
                    {over}
                </div>
            }
        });
    let set_aside = months.with_value(|m| saving(&row, m)).map(|s| {
        let unit = Currency::new(&code);
        let this_month = crate::ds::money(s.this_month, &unit);
        let (label, when) = match &s.unsaved {
            Some((_, due)) => ("Still unsaved in", format::month_long(due)),
            None => ("Saved up by", format::month_long(&s.saved_by)),
        };
        let gap = s.unsaved.as_ref().map(|(n, _)| {
            let n = crate::ds::money(*n, &unit);
            view! { <Stat label="Unsaved">{n}</Stat> }
        });
        view! {
            <div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
                <Stat label="Put aside this month">{this_month}</Stat>
                <Stat label=label>{when}</Stat>
                {gap}
            </div>
        }
    });
    let children = row
        .children
        .iter()
        .map(|child| {
            if child.wish.status == WishStatus::Planned {
                view! {
                    <div class=NESTED>
                        {wish_body(ui, child, months, ctx, Some(parent_must), in_event)}
                    </div>
                }
                .into_any()
            } else {
                let leave = (in_event && child.wish.status == WishStatus::Bought).then(|| {
                    view! { <div class="flex">{leave_button(ui, &child.wish.wish_id)}</div> }
                        .into_any()
                });
                let body = ctx.with_value(|c| done_body(&child.wish, c, true, leave));
                view! { <div class=NESTED>{body}</div> }.into_any()
            }
        })
        .collect_view();
    let body = wish_body(ui, &row, months, ctx, None, false);
    view! {
        <div class=DIVIDED>
            {body}
            <div class="grid gap-1 pb-3">
                {family}
                {set_aside}
            </div>
            {children}
        </div>
    }
}

/// Name, amount, date, source, verdict and actions of one planned wish.
/// `parent_must` is the parent's must flag for a sub-wish, `None` for a
/// top-level wish: a sub-wish of a must wish is a must too, and only the
/// parent's flag can be toggled. `in_event`: the parent is an event.
fn wish_body(
    ui: Ui,
    row: &WishRow,
    months: StoredValue<Vec<WishMonth>>,
    ctx: StoredValue<Context>,
    parent_must: Option<bool>,
    in_event: bool,
) -> impl IntoView + use<> {
    let sub = parent_must.is_some();
    let wish = row.wish.clone();
    let code = wish.currency.code.clone();
    let facts = months.with_value(|m| verdict_facts(row, m));
    let tone = if row.not_judged.is_some() {
        "text-fg-muted"
    } else if row.short > Decimal::ZERO {
        "text-over-fg"
    } else if row.tradeoffs {
        "text-warning-fg"
    } else {
        "text-positive-fg"
    };
    let source = ctx.with_value(|c| c.source_label(&wish.source));
    let is_event = wish.event.is_some();
    let mut detail = match (&wish.kind, &wish.event) {
        // An event runs over its days and has no one way of being paid.
        (WishKind::OneOff, Some(event)) => vec![
            ("Amount".to_owned(), amount(wish.amount, &code)),
            ("From".to_owned(), format::date(&event.starts_on)),
            ("Until".to_owned(), format::date(&event.ends_on)),
        ],
        (WishKind::OneOff, None) => vec![
            ("Amount".to_owned(), amount(wish.amount, &code)),
            ("By".to_owned(), format::date(&wish.due_on)),
            ("Paid from".to_owned(), source),
        ],
        (WishKind::Recurring { every, .. }, _) => vec![
            (
                format!("Per {}", period(*every)),
                amount(wish.amount, &code),
            ),
            ("From".to_owned(), format::date(&wish.due_on)),
            ("Paid from".to_owned(), source),
        ],
        (WishKind::BillChange { bill_id, .. }, _) => {
            let old = ctx.with_value(|c| c.bill(bill_id).map(|b| amount(b.amount, &code)));
            let mut rows = Vec::new();
            if let Some(old) = old {
                rows.push(("Now".to_owned(), old));
            }
            rows.push(("Becomes".to_owned(), amount(wish.amount, &code)));
            rows.push(("From".to_owned(), format::date(&wish.due_on)));
            rows
        }
    };
    if let WishKind::Recurring {
        ends_on: Some(end), ..
    }
    | WishKind::BillChange {
        ends_on: Some(end), ..
    } = &wish.kind
    {
        detail.push(("Until".to_owned(), format::date(end)));
    }
    let (chip_tone, chip) = if row.not_judged.is_some() {
        (Tone::Outline, "Not judged")
    } else if row.short > Decimal::ZERO {
        (Tone::Negative, "Short")
    } else if row.tradeoffs {
        (Tone::Warning, "With trade-offs")
    } else {
        (Tone::Positive, "Affordable")
    };
    let one_off = wish.kind == WishKind::OneOff;
    let start_text = match &wish.kind {
        WishKind::BillChange { bill_id, ends_on } => {
            let bill = ctx.with_value(|c| c.bill(bill_id).map(|b| (b.name.clone(), b.amount)));
            start_confirm(bill, wish.amount, ends_on.as_deref(), &code)
        }
        _ => String::new(),
    };
    let recurring = matches!(wish.kind, WishKind::Recurring { .. });
    let target = format!("wish:{}", wish.wish_id);
    let suggested = row.suggested_saved;
    let saved = RwSignal::new(suggested.to_string());
    let suggestion_hint = (suggested != wish.saved).then(|| {
        format!(
            "Suggested, +{} this month",
            amount(suggested - wish.saved, &code)
        )
    });
    let id = StoredValue::new(wish.wish_id.clone());
    let save_saved = {
        let target = target.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            let Ok(value) = parse_amount(&saved.get_untracked()) else {
                ui.set_message(&target, "Enter the amount saved.", true);
                return;
            };
            ui.write(
                target.clone(),
                "record_wish_saved",
                SavedArgs {
                    wish_id: id.get_value(),
                    saved: value,
                },
                "Saved amount updated.",
                || {},
            );
        }
    };
    let cards = StoredValue::new(ctx.with_value(|c| c.cards.clone()));
    let split = cards.with_value(|c| c.len() > 1);
    let refill = move || {
        let total = parse_amount(&ui.buy_amount.get_untracked()).unwrap_or_default();
        let on = ui.buy_on.get_untracked();
        let month = statement_month(&on).get(..7).unwrap_or_default().to_owned();
        let fill = cards.with_value(|c| fill_cards(total, c, &month));
        ui.buy_cards
            .set(fill.into_iter().map(|n| n.to_string()).collect());
    };
    let open_buy = {
        let wish = wish.clone();
        move |_| {
            ui.close_forms();
            ui.buy_amount.set(wish.amount.to_string());
            ui.buy_on.set(today());
            ui.buy_source.set(source_value(&wish.source));
            refill();
            ui.buying.set(Some(wish.wish_id.clone()));
        }
    };
    let open_start = move |_| {
        ui.close_forms();
        ui.start_category.set(String::new());
        ui.starting.set(Some(id.get_value()));
    };
    let buying = move || ui.buying.get().as_deref() == Some(id.get_value().as_str());
    let starting = move || ui.starting.get().as_deref() == Some(id.get_value().as_str());
    let start_target = target.clone();
    let start = move || {
        let match_category = if recurring {
            let category = ui.start_category.get_untracked();
            if category.is_empty() {
                ui.set_message(&start_target, "Choose the bill's category.", true);
                return;
            }
            Some(category)
        } else {
            None
        };
        ui.write(
            start_target.clone(),
            "start_wish",
            StartArgs {
                wish_id: id.get_value(),
                match_category,
            },
            "Started.",
            move || ui.starting.set(None),
        );
    };
    let start = StoredValue::new(start);
    let categories: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Choose a category".to_owned()))
            .chain(ctx.with_value(|c| {
                c.categories
                    .iter()
                    .map(|x| (x.clone(), x.clone()))
                    .collect::<Vec<_>>()
            }))
            .collect(),
    );
    let start_category_id = StoredValue::new(format!("start-category-{}", wish.wish_id));
    let dropping = move || ui.dropping.get().as_deref() == Some(id.get_value().as_str());
    let sources = ctx.with_value(Context::source_options);
    let buy_target = target.clone();
    let buy_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = parse_amount(&ui.buy_amount.get_untracked()) else {
            ui.set_message(&buy_target, "Enter what it cost.", true);
            return;
        };
        let source = parse_source(&ui.buy_source.get_untracked());
        let charges = if split && source == WishSource::Card {
            let amounts: Result<Vec<Decimal>, _> = ui
                .buy_cards
                .get_untracked()
                .iter()
                .map(|a| parse_amount(a))
                .collect();
            let charges = amounts.ok().and_then(|amounts| {
                let fits = amounts.len() == cards.with_value(Vec::len)
                    && amounts.iter().all(|a| *a >= Decimal::ZERO)
                    && amounts.iter().sum::<Decimal>() == value;
                fits.then(|| {
                    cards.with_value(|cards| {
                        cards
                            .iter()
                            .zip(amounts)
                            .filter(|(_, a)| *a > Decimal::ZERO)
                            .map(|(c, amount)| CardCharge {
                                card_id: c.card_id.clone(),
                                amount,
                            })
                            .collect::<Vec<_>>()
                    })
                })
            });
            let Some(charges) = charges else {
                ui.set_message(
                    &buy_target,
                    "Card amounts must add up to the purchase.",
                    true,
                );
                return;
            };
            charges
        } else {
            Vec::new()
        };
        let purchase = WishPurchase {
            amount: value,
            on: ui.buy_on.get_untracked(),
            source,
            cards: charges,
            transaction_id: None,
        };
        ui.write(
            buy_target.clone(),
            "mark_wish_bought",
            BoughtArgs {
                wish_id: id.get_value(),
                purchase,
            },
            "Marked as bought.",
            move || ui.buying.set(None),
        );
    };
    let confirm_drop = move |_| {
        ui.write(
            "dropped".into(),
            "drop_wish",
            WishIdArgs {
                wish_id: id.get_value(),
            },
            "Wish dropped.",
            move || ui.dropping.set(None),
        );
    };
    let saved_id = format!("saved-{}", wish.wish_id);
    let buy_amount_id = StoredValue::new(format!("buy-amount-{}", wish.wish_id));
    let buy_on_id = StoredValue::new(format!("buy-on-{}", wish.wish_id));
    let buy_source_id = StoredValue::new(format!("buy-source-{}", wish.wish_id));
    let drop_text = format!("Drop {}? It stops counting against your months.", wish.name);
    let must = wish.must;
    let must_target = format!("must-{}", wish.wish_id);
    let toggle_must = {
        let target = must_target.clone();
        move |_| {
            ui.write(
                target.clone(),
                "set_wish_must",
                MustArgs {
                    wish_id: id.get_value(),
                    must: !must,
                },
                "Saved.",
                || {},
            );
        }
    };
    let buy_sources = StoredValue::new(sources);
    let eventing = move || ui.eventing.get().as_deref() == Some(id.get_value().as_str());
    let due_on = wish.due_on.clone();
    let open_event = move |_| {
        ui.close_forms();
        ui.event_starts.set(due_on.clone());
        ui.event_ends.set(due_on.clone());
        ui.eventing.set(Some(id.get_value()));
    };
    let event_target = target.clone();
    let plan_event = StoredValue::new(move |ev: SubmitEvent| {
        ev.prevent_default();
        ui.write(
            event_target.clone(),
            "plan_wish_as_event",
            EventDatesArgs {
                wish_id: id.get_value(),
                starts_on: ui.event_starts.get_untracked(),
                ends_on: ui.event_ends.get_untracked(),
            },
            "Moved to Events.",
            move || ui.eventing.set(None),
        );
    });
    // The events this wish can become a part of: it has no parts itself.
    let event_options: StoredValue<Vec<(String, String)>> =
        StoredValue::new(if one_off && !sub && !is_event && row.children.is_empty() {
            ctx.with_value(|c| c.event_options(&code, false))
        } else {
            Vec::new()
        });
    let can_join = event_options.with_value(|o| !o.is_empty());
    let joining = move || ui.joining.get().as_deref() == Some(id.get_value().as_str());
    let open_join = move |_| {
        ui.close_forms();
        ui.join_event.set(
            event_options.with_value(|o| o.first().map(|(id, _)| id.clone()).unwrap_or_default()),
        );
        ui.joining.set(Some(id.get_value()));
    };
    let join_target = target.clone();
    let join = StoredValue::new(move |ev: SubmitEvent| {
        ev.prevent_default();
        ui.write(
            join_target.clone(),
            "move_wish",
            MoveArgs {
                wish_id: id.get_value(),
                event_id: Some(ui.join_event.get_untracked()),
            },
            "Added to the event.",
            move || ui.joining.set(None),
        );
    });
    let join_id = StoredValue::new(format!("join-event-{}", wish.wish_id));
    let event_starts_id = StoredValue::new(format!("event-starts-{}", wish.wish_id));
    let event_ends_id = StoredValue::new(format!("event-ends-{}", wish.wish_id));
    view! {
        <div class=SUB_ROW>
            <div class=TREE_HEAD>
                <h3 class=cn([TREE_NAME, if sub { "text-sm" } else { "text-base" }])>
                    {wish.name.clone()}
                    {(must || parent_must == Some(true)).then(|| view! { <Badge tone=Tone::Accent>"Must"</Badge> })}
                    <Badge tone=chip_tone>{chip}</Badge>
                </h3>
                <FactsRow rows=detail />
            </div>
            <FactsRow rows=facts class=tone />
            <div class=FILTER_ROW>
                {one_off.then(move || view! {
                <form class="flex items-end gap-2" on:submit=save_saved>
                    <div class=STACK_XS>
                        <label for=saved_id.clone() class=HINT>
                            "Saved so far"
                            {suggestion_hint.clone().map(|h| view! { <span class="text-accent">{format!(" ({h})")}</span> })}
                        </label>
                        <input
                            id=saved_id
                            type="text"
                            inputmode="decimal"
                            pattern=AMOUNT_PATTERN
                            class=cn([CONTROL, "h-8 w-28"])
                            prop:value=move || saved.get()
                            on:input=move |e| saved.set(event_target_value(&e))
                        />
                    </div>
                    <Button submit=true size=Size::Sm variant=Variant::Secondary disabled=Signal::derive(move || ui.busy.get())>
                        "Save"
                    </Button>
                </form>
                })}
                {if is_event {
                    // An event is spent over its days, not bought at once.
                    ().into_any()
                } else if one_off {
                    view! {
                        <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_buy)>
                            "Bought"
                        </Button>
                    }
                    .into_any()
                } else {
                    view! {
                        <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_start)>
                            "Start"
                        </Button>
                    }
                    .into_any()
                }}
                <Button
                    size=Size::Sm
                    variant=Variant::Ghost
                    on_click=Callback::new(move |_| {
                        ui.close_forms();
                        ui.dropping.set(Some(id.get_value()));
                    })
                >
                    "Drop"
                </Button>
                {(!sub).then(move || view! {
                <Button
                    size=Size::Sm
                    variant=Variant::Ghost
                    disabled=Signal::derive(move || ui.busy.get())
                    on_click=Callback::new(toggle_must)
                >
                    {if must { "Not a must" } else { "Mark must" }}
                </Button>
                })}
                {(one_off && !sub && !is_event).then(move || view! {
                <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(open_event)>
                    "Plan as event"
                </Button>
                })}
                {can_join.then(move || view! {
                <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(open_join)>
                    "Add to event"
                </Button>
                })}
                {(sub && in_event).then(|| leave_button(ui, &id.get_value()))}
            </div>
            <Show when=joining>
                <form class=EVENT_FORM on:submit=move |ev: SubmitEvent| join.with_value(|f| f(ev))>
                    <Field label="Event" id=join_id.get_value()>
                        {select(join_id.get_value(), ui.join_event, event_options.get_value())}
                    </Field>
                    <div class=PAIR>
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Add to event"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.joining.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=eventing>
                <form class=EVENT_FORM on:submit=move |ev: SubmitEvent| plan_event.with_value(|f| f(ev))>
                    <Field label="First day" id=event_starts_id.get_value()>
                        <input
                            id=event_starts_id.get_value()
                            type="date"
                            required
                            class=CONTROL
                            prop:value=move || ui.event_starts.get()
                            on:input=move |e| ui.event_starts.set(event_target_value(&e))
                        />
                    </Field>
                    <Field label="Last day" id=event_ends_id.get_value()>
                        <input
                            id=event_ends_id.get_value()
                            type="date"
                            required
                            class=CONTROL
                            prop:value=move || ui.event_ends.get()
                            on:input=move |e| ui.event_ends.set(event_target_value(&e))
                        />
                    </Field>
                    <div class=PAIR>
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Move to Events"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.eventing.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=buying>
                <form class=EVENT_FORM on:submit=buy_submit.clone()>
                    <Field label="Cost" id=buy_amount_id.get_value()>
                        <input
                            id=buy_amount_id.get_value()
                            type="text"
                            inputmode="decimal"
                            pattern=AMOUNT_PATTERN
                            required
                            class=CONTROL
                            prop:value=move || ui.buy_amount.get()
                            on:input=move |e| {
                                ui.buy_amount.set(event_target_value(&e));
                                refill();
                            }
                        />
                    </Field>
                    <Field label="On" id=buy_on_id.get_value()>
                        <input
                            id=buy_on_id.get_value()
                            type="date"
                            max=today()
                            required
                            class=CONTROL
                            prop:value=move || ui.buy_on.get()
                            on:input=move |e| ui.buy_on.set(event_target_value(&e))
                        />
                    </Field>
                    <Field label="Paid with" id=buy_source_id.get_value()>
                        {select(buy_source_id.get_value(), ui.buy_source, buy_sources.get_value())}
                    </Field>
                    {move || (split && ui.buy_source.get() == "card").then(|| card_fields(ui, cards, &buy_amount_id.get_value()))}
                    <div class=PAIR>
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Mark bought"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.buying.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=move || starting() && recurring>
                <form
                    class="grid items-end gap-2 sm:grid-cols-3"
                    on:submit=move |ev: SubmitEvent| {
                        ev.prevent_default();
                        start.with_value(|f| f());
                    }
                >
                    <Field
                        label="Bill category"
                        id=start_category_id.get_value()
                        hint="Payments in this category mark the new bill paid."
                    >
                        {select(start_category_id.get_value(), ui.start_category, categories.get_value())}
                    </Field>
                    <div class=cn([PAIR, LABEL_OFFSET])>
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Start"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.starting.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=move || starting() && !recurring>
                <div class=INLINE_TOOLS>
                    <span>{start_text.clone()}</span>
                    <Button
                        size=Size::Sm
                        disabled=Signal::derive(move || ui.busy.get())
                        on_click=Callback::new(move |_| start.with_value(|f| f()))
                    >
                        "Start"
                    </Button>
                    <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| ui.starting.set(None))>"Keep"</Button>
                </div>
            </Show>
            <Show when=dropping>
                <div class=INLINE_TOOLS>
                    <span>{drop_text.clone()}</span>
                    <Button size=Size::Sm variant=Variant::Danger on_click=Callback::new(confirm_drop)>"Drop wish"</Button>
                    <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| ui.dropping.set(None))>"Keep"</Button>
                </div>
            </Show>
            {feedback(ui, must_target)}
            {feedback(ui, target)}
        </div>
    }
}

/// The bought form's amount per card, prefilled in fill order.
fn card_fields(ui: Ui, cards: StoredValue<Vec<CardSettings>>, base: &str) -> impl IntoView + use<> {
    let fields = cards
        .get_value()
        .into_iter()
        .enumerate()
        .map(|(i, card)| {
            let id = format!("{base}-card-{i}");
            view! {
                <Field label=format!("On {}", card.name) id=id.clone()>
                    <input
                        id=id
                        type="text"
                        inputmode="decimal"
                        pattern=AMOUNT_PATTERN
                        required
                        class=CONTROL
                        prop:value=move || ui.buy_cards.with(|a| a.get(i).cloned().unwrap_or_default())
                        on:input=move |e| {
                            let value = event_target_value(&e);
                            ui.buy_cards.update(|a| {
                                if let Some(slot) = a.get_mut(i) {
                                    *slot = value;
                                }
                            });
                        }
                    />
                </Field>
            }
        })
        .collect_view();
    view! { <div class="grid gap-2 sm:col-span-4 sm:grid-cols-4">{fields}</div> }
}

/// What starting a bill change asks; with an end date it warns that the
/// old amount does not come back on its own.
fn start_confirm(
    bill: Option<(String, Decimal)>,
    new_amount: Decimal,
    ends_on: Option<&str>,
    code: &str,
) -> String {
    let (name, old) = bill.map_or((None, None), |(n, a)| (Some(n), Some(a)));
    let name = name.unwrap_or_else(|| "the bill".to_owned());
    let ask = if new_amount.is_zero() {
        format!("Archive {name} now? Its payments stop counting from today.")
    } else {
        format!(
            "Change {name} to {} now? The bill keeps its schedule.",
            amount(new_amount, code)
        )
    };
    match ends_on {
        None => ask,
        Some(end) => {
            let back = old.map_or_else(
                || "the old amount".to_owned(),
                |o| format!("the old amount ({})", amount(o, code)),
            );
            format!(
                "{ask} After {}, {back} won't come back on its own: change the bill back then.",
                format::date(end)
            )
        }
    }
}

/// A bought, started or dropped wish as the fields a planned one has:
/// its name, a chip for how it ended, and what it cost. `action` sits
/// under the fields.
fn done_body(wish: &Wish, ctx: &Context, sub: bool, action: Option<AnyView>) -> AnyView {
    let (tone, chip, facts) = done_facts(wish, ctx);
    view! {
        <div class=SUB_ROW>
            <div class=TREE_HEAD>
                <h3 class=cn([TREE_NAME, if sub { "text-sm" } else { "text-base" }])>
                    {wish.name.clone()}
                    <Badge tone=tone>{chip}</Badge>
                </h3>
                <FactsRow rows=facts />
            </div>
            {action}
        </div>
    }
    .into_any()
}

/// The parts of a done wish, below it (they are no longer judged once the
/// parent is settled).
fn done_parts(row: &WishRow, ctx: &Context) -> impl IntoView + use<> {
    row.children
        .iter()
        .map(|child| {
            view! {
                <div class=NESTED>{done_body(&child.wish, ctx, true, None)}</div>
            }
        })
        .collect_view()
}

/// "Back to wishes" for a part of an event.
fn leave_button(ui: Ui, wish_id: &str) -> impl IntoView + use<> {
    let id = StoredValue::new(wish_id.to_owned());
    let leave = move |_| {
        ui.write(
            format!("wish:{}", id.get_value()),
            "move_wish",
            MoveArgs {
                wish_id: id.get_value(),
                event_id: None,
            },
            "Back on Wishes.",
            || {},
        );
    };
    view! {
        <Button
            size=Size::Sm
            variant=Variant::Ghost
            disabled=Signal::derive(move || ui.busy.get())
            on_click=Callback::new(leave)
        >
            "Back to wishes"
        </Button>
    }
}

/// How many done wishes show at first, and how many each scroll adds.
const DONE_PAGE: usize = 10;

/// The done wishes, a page at a time: the next page loads when the end of
/// the list scrolls into view.
fn done_list(ui: Ui, done: Vec<WishRow>, ctx: StoredValue<Context>) -> impl IntoView + use<> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{IntersectionObserver, IntersectionObserverEntry};

    let total = done.len();
    let done = StoredValue::new(done);
    let shown = RwSignal::new(DONE_PAGE.min(total));
    let end = NodeRef::<leptos::html::Div>::new();
    let seen = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
        let visible = entries.iter().any(|e| {
            e.unchecked_into::<IntersectionObserverEntry>()
                .is_intersecting()
        });
        if visible {
            shown.update(|n| *n = (*n + DONE_PAGE).min(total));
        }
    });
    // Kept with the list and dropped with it.
    let watch = StoredValue::new_local(
        IntersectionObserver::new(seen.as_ref().unchecked_ref())
            .ok()
            .map(|observer| (observer, seen)),
    );
    // Watched anew after every page: an end still in view loads the next.
    Effect::new(move |_| {
        shown.track();
        if let Some(end) = end.get() {
            watch.with_value(|w| {
                if let Some((observer, _)) = w {
                    observer.unobserve(&end);
                    observer.observe(&end);
                }
            });
        }
    });
    on_cleanup(move || {
        watch.try_with_value(|w| {
            if let Some((observer, _)) = w {
                observer.disconnect();
            }
        });
    });
    view! {
        {move || {
            done.with_value(|rows| {
                rows.iter()
                    .take(shown.get())
                    .map(|row| done_entry(ui, row, ctx))
                    .collect_view()
            })
        }}
        <Show when=move || shown.get() < total>
            // The button is there for when the scroll is not seen.
            <div node_ref=end class="border-t border-line pt-2">
                <Button
                    size=Size::Sm
                    variant=Variant::Ghost
                    on_click=Callback::new(move |_| shown.update(|n| *n = (*n + DONE_PAGE).min(total)))
                >
                    "Show more"
                </Button>
            </div>
        </Show>
    }
}

/// A done wish on the Wishes screen: one that was bought, and has no
/// parts, can still be put into an event, where it counts as spent.
fn done_entry(ui: Ui, row: &WishRow, ctx: StoredValue<Context>) -> impl IntoView + use<> {
    let wish = &row.wish;
    let options = if wish.status == WishStatus::Bought
        && wish.kind == WishKind::OneOff
        && row.children.is_empty()
    {
        ctx.with_value(|c| c.event_options(&wish.currency.code, true))
    } else {
        Vec::new()
    };
    let parts = ctx.with_value(|c| done_parts(row, c));
    if options.is_empty() {
        let body = ctx.with_value(|c| done_body(wish, c, false, None));
        return view! { <div class=DIVIDED>{body}{parts}</div> }.into_any();
    }
    let options = StoredValue::new(options);
    let id = StoredValue::new(wish.wish_id.clone());
    let target = format!("wish:{}", wish.wish_id);
    let joining = move || ui.joining.get().as_deref() == Some(id.get_value().as_str());
    let open = move |_| {
        ui.close_forms();
        ui.join_event
            .set(options.with_value(|o| o.first().map(|(id, _)| id.clone()).unwrap_or_default()));
        ui.joining.set(Some(id.get_value()));
    };
    let join_target = target.clone();
    let join = StoredValue::new(move |ev: SubmitEvent| {
        ev.prevent_default();
        ui.write(
            join_target.clone(),
            "move_wish",
            MoveArgs {
                wish_id: id.get_value(),
                event_id: Some(ui.join_event.get_untracked()),
            },
            "Added to the event.",
            move || ui.joining.set(None),
        );
    });
    let select_id = StoredValue::new(format!("join-event-{}", wish.wish_id));
    let action = view! {
        <div class=FILTER_ROW>
            <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(open)>"Add to event"</Button>
        </div>
        <Show when=joining>
            <form class=EVENT_FORM on:submit=move |ev: SubmitEvent| join.with_value(|f| f(ev))>
                <Field label="Event" id=select_id.get_value()>
                    {select(select_id.get_value(), ui.join_event, options.get_value())}
                </Field>
                <div class=PAIR>
                    <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Add to event"</Button>
                    <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.joining.set(None))>"Cancel"</Button>
                </div>
            </form>
        </Show>
        {feedback(ui, target)}
    }
    .into_any();
    let body = ctx.with_value(|c| done_body(wish, c, false, Some(action)));
    view! { <div class=DIVIDED>{body}{parts}</div> }.into_any()
}

/// How a done wish ended, and its fields.
fn done_facts(wish: &Wish, ctx: &Context) -> (Tone, &'static str, Vec<(String, String)>) {
    let code = &wish.currency.code;
    let planned = ("Amount".to_owned(), amount(wish.amount, code));
    match (&wish.status, &wish.bought) {
        (WishStatus::Bought, Some(p)) => (
            Tone::Positive,
            "Bought",
            vec![
                ("Cost".to_owned(), amount(p.amount, code)),
                ("On".to_owned(), format::date(&p.on)),
                ("Paid from".to_owned(), ctx.source_label(&p.source)),
            ],
        ),
        (WishStatus::Bought, None) => (Tone::Positive, "Bought", vec![planned]),
        (WishStatus::Started, _) => (
            Tone::Positive,
            "Started",
            vec![planned, ("From".to_owned(), format::date(&wish.due_on))],
        ),
        // A planned part of a wish that is done: no longer judged.
        (WishStatus::Planned, _) => (Tone::Outline, "Not bought", vec![planned]),
        (WishStatus::Dropped, _) => (Tone::Outline, "Dropped", vec![planned]),
    }
}

fn month_card(month: &WishMonth, code: &str) -> impl IntoView + use<> {
    let unit = Currency::new(code);
    let money = |v: Decimal| crate::ds::money(v, &unit);
    let title = format::month_long(&month.month);
    let (before, after, charge, used) = (
        money(month.free_before),
        money(month.free_left),
        money(month.card_charge_in),
        money(month.card_used),
    );
    let (savings_before, savings_after) = (money(month.savings_before), money(month.savings_left));
    let payback = payback(month, code)
        .map(|rows| view! { <Facts rows=rows class="border-t border-line pt-3" /> });
    let short = month_short(month);
    let short = (short > Decimal::ZERO).then(|| {
        view! {
            <span role="status"><Badge tone=Tone::Negative>{format!("Short {}", amount(short, code))}</Badge></span>
        }
    });
    view! {
        <Card tone=CardTone::Muted class="grid gap-3 p-4 sm:p-5">
            <h3 class=STRONG_SM>{title}</h3>
            <div class="grid grid-cols-2 gap-3">
                <Stat label="Free before">{before}</Stat>
                <Stat label="Free after">{after}</Stat>
                <Stat label="Savings before">{savings_before}</Stat>
                <Stat label="Savings after">{savings_after}</Stat>
                <Stat label="Card charge in">{charge}</Stat>
                <Stat label="Card used">{used}</Stat>
            </div>
            {payback}
            {short}
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_share_is_floored_and_capped() {
        let d = |t: &str| t.parse::<Decimal>().unwrap();
        assert_eq!(saved_percent(d("599.62"), d("3000")), 19);
        assert_eq!(saved_percent(d("3100"), d("3000")), 100);
        assert_eq!(saved_percent(d("10"), d("0")), 0);
    }

    fn facts(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(label, value)| ((*label).to_owned(), (*value).to_owned()))
            .collect()
    }
    use finance_types::report::wishes::CardUsed;

    fn d(n: i64, scale: u32) -> Decimal {
        Decimal::new(n, scale)
    }

    fn row(id: &str) -> WishRow {
        WishRow {
            wish: Wish {
                wish_id: id.into(),
                parent_id: None,
                name: "Jacket".into(),
                amount: d(150, 0),
                currency: Currency::new("EUR"),
                due_on: "2026-10-20".into(),
                source: WishSource::Card,
                saved: Decimal::ZERO,
                saved_month: None,
                status: WishStatus::Planned,
                bought: None,
                kind: Default::default(),
                must: false,
                created_at: "2026-10-01T00:00:00Z".into(),
                event: None,
            },
            children: Vec::new(),
            rest: Decimal::ZERO,
            over_cap: false,
            monthly_set_aside: Decimal::ZERO,
            suggested_saved: Decimal::ZERO,
            short: Decimal::ZERO,
            tradeoffs: false,
            not_judged: None,
        }
    }

    fn verdict(id: &str, amount: Decimal) -> Verdict {
        Verdict {
            wish_id: Some(id.into()),
            label: "Jacket".into(),
            amount,
            from_free: Decimal::ZERO,
            from_savings: Decimal::ZERO,
            cuts: Vec::new(),
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
            set_aside: false,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
            must: false,
            skipped: false,
        }
    }

    fn month(free_before: Decimal, free_left: Decimal, verdicts: Vec<Verdict>) -> WishMonth {
        WishMonth {
            month: "2026-10-01".into(),
            free_before,
            free_left,
            savings_before: Decimal::ZERO,
            savings_left: Decimal::ZERO,
            card_charge_in: Decimal::ZERO,
            card_used: d(998, 2),
            cards: Vec::new(),
            verdicts,
        }
    }

    fn month_at(at: &str, verdicts: Vec<Verdict>) -> WishMonth {
        WishMonth {
            month: at.into(),
            ..month(Decimal::ZERO, Decimal::ZERO, verdicts)
        }
    }

    fn cut(id: &str, name: &str, from: i64, to: i64) -> Cut {
        Cut {
            group_id: id.into(),
            name: name.into(),
            from: d(from, 0),
            to: d(to, 0),
        }
    }

    fn card_used(id: &str, name: &str, used: Decimal) -> CardUsed {
        CardUsed {
            regular: Decimal::ZERO,
            card_id: id.into(),
            name: name.into(),
            used,
        }
    }

    #[test]
    fn payback_line_leaves_out_the_cards_regular_spending() {
        let mut m = month_at("2027-01-01", vec![]);
        m.cards = vec![CardUsed {
            regular: d(835, 2),
            ..card_used("visa", "Visa", d(835, 2))
        }];
        m.card_used = d(835, 2);
        assert_eq!(payback(&m, "EUR"), None);
        m.cards[0].used = d(5835, 2);
        m.card_used = d(5835, 2);
        assert_eq!(
            payback(&m, "EUR").unwrap(),
            [("Pay back on 1 February".to_owned(), "€50".to_owned())]
        );
    }

    #[test]
    fn payback_line_names_each_card() {
        let mut m = month_at("2026-11-01", vec![]);
        m.cards = vec![
            card_used("visa", "Visa", d(6000, 2)),
            card_used("mc", "Mastercard", d(9000, 2)),
            card_used("x", "Amex", Decimal::ZERO),
        ];
        m.card_used = d(15000, 2);
        assert_eq!(
            payback(&m, "EUR").unwrap(),
            [
                ("Pay back on 1 December".to_owned(), "€150".to_owned()),
                ("Visa".to_owned(), "€60".to_owned()),
                ("Mastercard".to_owned(), "€90".to_owned()),
            ]
        );
        // Two cards, one used: no breakdown.
        m.cards = vec![
            card_used("visa", "Visa", d(6000, 2)),
            card_used("mc", "Mastercard", Decimal::ZERO),
        ];
        m.card_used = d(6000, 2);
        assert_eq!(
            payback(&m, "EUR").unwrap(),
            [("Pay back on 1 December".to_owned(), "€60".to_owned())]
        );
        m.cards.truncate(1);
        assert_eq!(
            payback(&m, "EUR").unwrap(),
            [("Pay back on 1 December".to_owned(), "€60".to_owned())]
        );
        m.card_used = Decimal::ZERO;
        m.cards.clear();
        assert!(payback(&m, "EUR").is_none());
    }

    fn settings_card(id: &str, limit: i64, used: i64, used_month: &str) -> CardSettings {
        CardSettings {
            card_id: id.into(),
            name: id.into(),
            limit: d(limit, 0),
            used: d(used, 0),
            used_month: used_month.into(),
            bill_id: None,
        }
    }

    #[test]
    fn statement_month_turns_on_the_26th() {
        assert_eq!(statement_month("2026-10-25"), "2026-10-01");
        assert_eq!(statement_month("2026-10-26"), "2026-11-01");
        assert_eq!(statement_month("2026-11-01"), "2026-11-01");
        assert_eq!(statement_month("2026-12-31"), "2027-01-01");
        assert_eq!(statement_period("2026-11-01"), "26 Oct \u{2013} 25 Nov");
        assert_eq!(statement_period("2027-01-01"), "26 Dec \u{2013} 25 Jan");
    }

    #[test]
    fn fill_cards_fills_in_order_and_last_takes_the_rest() {
        let cards = [
            settings_card("visa", 100, 30, "2026-10-01"),
            settings_card("mc", 50, 0, "2026-10-01"),
            settings_card("amex", 10, 0, "2026-10-01"),
        ];
        assert_eq!(
            fill_cards(d(150, 0), &cards, "2026-10"),
            vec![d(70, 0), d(50, 0), d(30, 0)]
        );
        assert_eq!(
            fill_cards(d(40, 0), &cards, "2026-10"),
            vec![d(40, 0), d(0, 0), d(0, 0)]
        );
        // Usage from an earlier statement counts as €0.
        assert_eq!(
            fill_cards(d(100, 0), &cards, "2026-11"),
            vec![d(100, 0), d(0, 0), d(0, 0)]
        );
    }

    #[test]
    fn payback_line_keeps_cents() {
        let mut m = month_at("2026-12-01", vec![]);
        m.cards = vec![
            card_used("visa", "Visa", d(1050, 2)),
            card_used("mc", "Mastercard", d(2, 0)),
        ];
        m.card_used = d(1250, 2);
        assert_eq!(
            payback(&m, "EUR").unwrap(),
            [
                ("Pay back on 1 January".to_owned(), "€12.50".to_owned()),
                ("Visa".to_owned(), "€10.50".to_owned()),
                ("Mastercard".to_owned(), "€2".to_owned()),
            ]
        );
    }

    #[test]
    fn fits_in_free_money_says_free_money_drops() {
        let v = Verdict {
            from_free: d(150, 0),
            ..verdict("w", d(150, 0))
        };
        let months = [month(d(500, 0), d(350, 0), vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Free money after", "€350")])
        );
    }

    #[test]
    fn free_money_drop_keeps_cents() {
        let v = Verdict {
            from_free: d(5998, 2),
            ..verdict("w", d(5998, 2))
        };
        let months = [month(d(100, 0), d(4002, 2), vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Free money after", "€40.02")])
        );
    }

    #[test]
    fn cuts_name_going_out_then_groceries() {
        let v = Verdict {
            cuts: vec![
                cut("g1", "Going out", 120, 0),
                cut("g2", "Groceries", 450, 370),
            ],
            ..verdict("w", d(200, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Going out down to", "€0"), ("Groceries down to", "€370")])
        );
    }

    #[test]
    fn card_use_names_the_next_month() {
        let v = Verdict {
            on_card: d(100, 0),
            ..verdict("w", d(400, 0))
        };
        let later = Verdict {
            on_card: d(30, 0),
            ..verdict("w", d(30, 0))
        };
        let months = [
            month(Decimal::ZERO, Decimal::ZERO, vec![v]),
            month_at("2026-11-01", vec![later]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€130"),
                ("November starts lighter by", "€130")
            ])
        );
    }

    #[test]
    fn card_use_also_names_the_cuts() {
        let v = Verdict {
            cuts: vec![
                cut("g1", "Going out", 120, 0),
                cut("g2", "Groceries", 450, 300),
            ],
            on_card: d(130, 0),
            ..verdict("w", d(400, 0))
        };
        let months = [
            month_at("2026-09-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![v]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€130"),
                ("December starts lighter by", "€130"),
                ("Going out down to", "€0"),
                ("Groceries down to", "€300")
            ])
        );
    }

    fn repeating(id: &str, every: Repeat) -> WishRow {
        let mut r = row(id);
        r.wish.kind = WishKind::Recurring {
            every,
            ends_on: None,
        };
        r
    }

    fn paid(amount: i64) -> Verdict {
        Verdict {
            from_free: d(amount, 0),
            ..verdict("w", d(amount, 0))
        }
    }

    #[test]
    fn recurring_that_fits_is_affordable_every_month() {
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![paid(60)]),
            month_at("2026-12-01", vec![paid(60)]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[])
        );
    }

    #[test]
    fn recurring_cuts_hold_each_month() {
        let v = Verdict {
            cuts: vec![cut("g1", "Going out", 120, 60)],
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![v]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[("Going out down to", "€60")])
        );
    }

    #[test]
    fn recurring_on_the_cards_is_paid_the_following_month() {
        let card = |n| Verdict {
            on_card: d(n, 0),
            ..verdict("w", d(n, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![card(40)]),
            month_at("2026-12-01", vec![card(60)]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[("On the cards a month", "€40")])
        );
        let with_cuts = Verdict {
            cuts: vec![cut("g1", "Going out", 120, 0)],
            ..card(40)
        };
        let months = [month_at("2026-10-01", vec![with_cuts])];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[("On the cards a month", "€40"), ("Going out down to", "€0")])
        );
    }

    #[test]
    fn recurring_fits_until_a_short_month() {
        let short = Verdict {
            short: d(25, 0),
            ..verdict("w", d(60, 0))
        };
        let later = Verdict {
            short: d(60, 0),
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![paid(60)]),
            month_at("2026-12-01", vec![short]),
            month_at("2027-01-01", vec![later]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[
                ("Fits until", "November"),
                ("Short from", "December"),
                ("Short by", "€25")
            ])
        );
    }

    #[test]
    fn recurring_short_from_the_start_is_not_affordable() {
        let short = Verdict {
            short: d(15, 0),
            on_card: d(45, 0),
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![short]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Weekly), &months),
            facts(&[("Short from", "November"), ("Short by", "€15")])
        );
    }

    #[test]
    fn cheaper_bill_frees_money() {
        let mut r = row("w");
        r.wish.kind = WishKind::BillChange {
            bill_id: "7".into(),
            ends_on: None,
        };
        let freed = Verdict {
            from_free: d(-15, 0),
            ..verdict("w", d(-15, 0))
        };
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![freed.clone()]),
            month_at("2026-12-01", vec![freed]),
        ];
        assert_eq!(
            verdict_facts(&r, &months),
            facts(&[("Frees a month", "€15"), ("From", "November")])
        );
    }

    #[test]
    fn freed_money_is_not_a_savings_drop() {
        let freed = Verdict {
            from_free: d(-15, 0),
            ..verdict("w", d(-15, 0))
        };
        let months = [month(d(100, 0), d(115, 0), vec![freed])];
        assert_eq!(verdict_facts(&row("w"), &months), facts(&[]));
    }

    #[test]
    fn short_wins_over_everything() {
        let v = Verdict {
            short: d(50, 0),
            on_card: d(10, 0),
            ..verdict("w", d(50, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Short by", "€50")])
        );
    }

    fn saving_for(id: &str, name: &str, amount: Decimal) -> Verdict {
        Verdict {
            label: format!("Saving for {name}"),
            set_aside: true,
            from_free: amount,
            ..verdict(id, amount)
        }
    }

    #[test]
    fn money_saved_for_an_earlier_wish_is_named() {
        let v = Verdict {
            short: d(50, 0),
            ..verdict("w", d(50, 0))
        };
        let months = [month(
            d(300, 0),
            Decimal::ZERO,
            vec![saving_for("bike", "Bike", d(300, 0)), v],
        )];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Short by", "€50"), ("Saved first for Bike", "€300")])
        );
    }

    #[test]
    fn several_earlier_saves_are_listed_in_order() {
        let v = Verdict {
            cuts: vec![cut("g1", "Going out", 120, 40)],
            ..verdict("w", d(80, 0))
        };
        let months = [month(
            d(400, 0),
            Decimal::ZERO,
            vec![
                saving_for("bike", "Bike", d(300, 0)),
                saving_for("laptop", "Laptop", d(100, 0)),
                v,
            ],
        )];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("Going out down to", "€40"),
                ("Saved first for Bike", "€300"),
                ("Saved first for Laptop", "€100")
            ])
        );
    }

    #[test]
    fn own_skipped_month_names_the_earlier_save() {
        let months = vec![
            month_at(
                "2026-11-01",
                vec![saving_for("bike", "Bike", d(300, 0)), verdict_skipped("w")],
            ),
            month_at("2026-12-01", vec![verdict_set_aside("w", d(30000, 2))]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("Saving starts", "December"),
                ("A month from then", "€300"),
                ("Saved first for Bike", "€300")
            ])
        );
    }

    #[test]
    fn earlier_saves_stay_quiet_when_affordable_or_after_this_wish() {
        let v = Verdict {
            from_free: d(50, 0),
            ..verdict("w", d(50, 0))
        };
        let months = [month(
            d(400, 0),
            d(400, 0),
            vec![saving_for("bike", "Bike", d(300, 0)), v.clone()],
        )];
        assert_eq!(verdict_facts(&row("w"), &months), facts(&[]));
        let later = Verdict {
            short: d(50, 0),
            ..v
        };
        let months = [month(
            d(400, 0),
            Decimal::ZERO,
            vec![later, saving_for("bike", "Bike", d(300, 0))],
        )];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Short by", "€50")])
        );
    }

    #[test]
    fn skipped_earlier_save_took_nothing() {
        let v = Verdict {
            short: d(50, 0),
            ..verdict("w", d(50, 0))
        };
        let months = [month(
            Decimal::ZERO,
            Decimal::ZERO,
            vec![verdict_skipped("bike"), v],
        )];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Short by", "€50")])
        );
    }

    #[test]
    fn saving_names_the_month_it_is_saved_by() {
        let mut r = row("w");
        r.monthly_set_aside = d(300, 0);
        let months = vec![
            month_at("2026-10-01", vec![verdict_set_aside("w", d(300, 0))]),
            month_at("2026-11-01", vec![verdict_set_aside("w", d(300, 0))]),
            month_at("2026-12-01", vec![verdict_set_aside("w", d(100, 0))]),
        ];
        assert_eq!(
            saving(&r, &months),
            Some(Saving {
                this_month: d(300, 0),
                saved_by: "2026-12-01".into(),
                unsaved: None,
            })
        );
    }

    #[test]
    fn saving_says_what_stays_unsaved_at_the_due_month() {
        let mut r = row("w");
        r.monthly_set_aside = d(300, 0);
        let bought = Verdict {
            from_free: d(200, 0),
            ..verdict("w", d(200, 0))
        };
        let months = vec![
            month_at("2026-10-01", vec![verdict_set_aside("w", d(300, 0))]),
            month_at("2026-11-01", vec![verdict_skipped("w")]),
            month_at("2026-12-01", vec![bought]),
        ];
        assert_eq!(
            saving(&r, &months),
            Some(Saving {
                this_month: d(300, 0),
                saved_by: "2026-10-01".into(),
                unsaved: Some((d(200, 0), "2026-12-01".into())),
            })
        );
    }

    #[test]
    fn saving_after_a_skipped_month_still_names_the_date() {
        let months = vec![
            month_at("2026-10-01", vec![verdict_skipped("w")]),
            month_at("2026-11-01", vec![verdict_set_aside("w", d(150, 0))]),
        ];
        assert_eq!(
            saving(&row("w"), &months),
            Some(Saving {
                this_month: Decimal::ZERO,
                saved_by: "2026-11-01".into(),
                unsaved: None,
            })
        );
    }

    #[test]
    fn saving_is_absent_without_set_asides() {
        let months = [month(
            Decimal::ZERO,
            Decimal::ZERO,
            vec![verdict("w", d(150, 0))],
        )];
        assert_eq!(saving(&row("w"), &months), None);
        assert_eq!(saving(&repeating("w", Repeat::Monthly), &months), None);
    }

    #[test]
    fn nothing_taken_is_plainly_affordable() {
        let other = Verdict {
            from_free: d(80, 0),
            ..verdict("other", d(80, 0))
        };
        let months = [month(d(100, 0), d(20, 0), vec![other])];
        assert_eq!(verdict_facts(&row("w"), &months), facts(&[]));
    }

    #[test]
    fn card_charge_next_month_cannot_pay_names_the_short() {
        let v = Verdict {
            on_card: d(500, 0),
            next_short: d(500, 0),
            ..verdict("w", d(500, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€500"),
                ("November starts short by", "€500")
            ])
        );
    }

    #[test]
    fn card_charge_short_also_names_its_cuts() {
        let v = Verdict {
            on_card: d(500, 0),
            next_short: d(380, 0),
            next_cuts: vec![cut("g1", "Going out", 120, 0)],
            ..verdict("w", d(500, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€500"),
                ("November starts short by", "€380"),
                ("Going out down to, November", "€0")
            ])
        );
    }

    #[test]
    fn card_charge_paid_by_cuts_names_them() {
        let v = Verdict {
            on_card: d(100, 0),
            next_cuts: vec![cut("g1", "Going out", 120, 20)],
            ..verdict("w", d(100, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€100"),
                ("November starts lighter by", "€100"),
                ("Going out down to, November", "€20")
            ])
        );
    }

    #[test]
    fn recurring_card_charge_short_shows_the_month_after() {
        let card = |n, next_short| Verdict {
            on_card: d(n, 0),
            next_short: d(next_short, 0),
            ..verdict("w", d(n, 0))
        };
        let months = [
            month_at("2026-10-01", vec![card(60, 0)]),
            month_at("2026-11-01", vec![card(60, 20)]),
            month_at("2026-12-01", vec![card(60, 60)]),
        ];
        assert_eq!(
            verdict_facts(&repeating("w", Repeat::Monthly), &months),
            facts(&[
                ("Fits until", "November"),
                ("Short from", "December"),
                ("Short by", "€20")
            ])
        );
    }

    #[test]
    fn month_short_sums_every_verdict() {
        let charge = Verdict {
            wish_id: None,
            short: d(30, 0),
            ..verdict("x", d(30, 0))
        };
        let tv = Verdict {
            short: d(12, 0),
            ..verdict("w", d(12, 0))
        };
        assert_eq!(
            month_short(&month(Decimal::ZERO, Decimal::ZERO, vec![charge, tv])),
            d(42, 0)
        );
    }

    #[test]
    fn bill_change_with_an_end_warns_on_start() {
        let gym = Some(("Gym".to_owned(), d(30, 0)));
        assert_eq!(
            start_confirm(gym.clone(), d(45, 0), None, "EUR"),
            "Change Gym to €45 now? The bill keeps its schedule."
        );
        let text = start_confirm(gym, d(45, 0), Some("2027-03-01"), "EUR");
        assert!(text.starts_with("Change Gym to €45 now? The bill keeps its schedule. After "));
        assert!(text.ends_with(
            ", the old amount (€30) won't come back on its own: change the bill back then."
        ));
    }

    #[test]
    fn unjudged_wish_says_why() {
        let mut r = row("w");
        r.not_judged = Some("too far ahead".into());
        assert_eq!(
            verdict_facts(&r, &[]),
            facts(&[("Not judged yet", "too far ahead")])
        );
    }

    #[test]
    fn bill_picker_lists_judgeable_bills_with_their_schedule() {
        let bill = |id: &str, name: &str| BillOption {
            id: id.into(),
            name: name.into(),
            amount: d(80, 0),
        };
        let bills = [bill("9", "Therapy"), bill("8", "Insurance")];
        let changeable = [ChangeableBill {
            bill_id: "9".into(),
            every: Repeat::Biweekly,
        }];
        let options = changeable_bills(&bills, &changeable, "EUR");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].1, "Therapy · €80 per 2 weeks");
    }

    #[test]
    fn a_card_counts_what_its_sub_wishes_saved() {
        let mut parent = row("trip");
        let mut dinner = row("dinner");
        dinner.wish.saved = d(300, 0);
        parent.children.push(dinner);
        assert_eq!(row_saved(&parent), d(300, 0));
        assert_eq!(wish_stage(&parent), Stage::Ready);
    }

    #[test]
    fn parent_counts_its_sub_wishes() {
        let mut parent = row("trip");
        parent.children.push(row("flights"));
        let v = Verdict {
            on_card: d(900, 0),
            ..verdict("flights", d(900, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&parent, &months),
            facts(&[
                ("On the cards", "€900"),
                ("November starts lighter by", "€900")
            ])
        );
    }

    fn verdict_skipped(id: &str) -> Verdict {
        Verdict {
            set_aside: true,
            skipped: true,
            ..verdict(id, Decimal::ZERO)
        }
    }

    fn verdict_set_aside(id: &str, amount: Decimal) -> Verdict {
        Verdict {
            set_aside: true,
            from_free: amount,
            ..verdict(id, amount)
        }
    }

    #[test]
    fn skipped_month_is_said_then_catch_up() {
        let months = vec![
            month_at("2026-11-01", vec![verdict_skipped("w")]),
            month_at("2026-12-01", vec![verdict_set_aside("w", d(30000, 2))]),
            month_at("2027-01-01", vec![verdict_set_aside("w", d(30000, 2))]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Saving starts", "December"), ("A month from then", "€300")])
        );
    }

    #[test]
    fn skipped_month_then_due_month_keeps_the_verdict() {
        let paid = Verdict {
            from_free: d(300, 0),
            ..verdict("w", d(300, 0))
        };
        let months = vec![
            month_at("2026-11-01", vec![verdict_skipped("w")]),
            month_at("2026-12-01", vec![paid]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("Not set aside in", "November")])
        );
    }

    #[test]
    fn skipped_month_comes_before_the_rest_of_the_verdict() {
        let charged = Verdict {
            on_card: d(100, 0),
            ..verdict("w", d(100, 0))
        };
        let months = vec![
            month_at("2026-11-01", vec![verdict_skipped("w")]),
            month_at("2026-12-01", vec![verdict_set_aside("w", d(50, 0))]),
            month_at("2027-01-01", vec![charged]),
        ];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("Saving starts", "December"),
                ("A month from then", "€50"),
                ("On the cards", "€100"),
                ("February starts lighter by", "€100")
            ])
        );
    }

    #[test]
    fn card_sentence_says_cards() {
        let v = Verdict {
            on_card: d(566, 0),
            next_short: d(3048, 2),
            ..verdict("w", d(566, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("On the cards", "€566"),
                ("November starts short by", "€30.48")
            ])
        );
    }

    fn with_savings(at: &str, savings_left: Decimal, verdicts: Vec<Verdict>) -> WishMonth {
        WishMonth {
            savings_before: savings_left + d(500, 0),
            savings_left,
            ..month_at(at, verdicts)
        }
    }

    #[test]
    fn savings_sentence_names_amount_and_remainder() {
        let v = Verdict {
            from_savings: d(120, 0),
            ..verdict("w", d(120, 0))
        };
        let months = [with_savings("2026-10-01", d(30, 0), vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("From savings", "€120"), ("Savings left", "€30")])
        );
    }

    #[test]
    fn savings_then_cuts() {
        let v = Verdict {
            from_savings: d(120, 0),
            cuts: vec![cut("g1", "Going out", 120, 40)],
            ..verdict("w", d(200, 0))
        };
        let months = [with_savings("2026-10-01", Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[("From savings", "€120"), ("Going out down to", "€40")])
        );
    }

    #[test]
    fn savings_then_cards() {
        let v = Verdict {
            from_savings: d(120, 0),
            on_card: d(80, 0),
            ..verdict("w", d(200, 0))
        };
        let months = [with_savings("2026-10-01", Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_facts(&row("w"), &months),
            facts(&[
                ("From savings", "€120"),
                ("On the cards", "€80"),
                ("November starts lighter by", "€80")
            ])
        );
    }

    #[test]
    fn savings_fields_add_the_saved_part_back_onto_the_pot() {
        let months = [with_savings("2026-10-01", d(250, 0), Vec::new())];
        assert_eq!(
            savings_fields(&months, d(624, 0)),
            Some((d(1374, 0), d(624, 0), d(750, 0)))
        );
        let short = WishMonth {
            free_before: d(-50, 0),
            ..with_savings("2026-10-01", d(250, 0), Vec::new())
        };
        assert_eq!(
            savings_fields(&[short], d(624, 0)),
            Some((d(1374, 0), d(624, 0), d(700, 0)))
        );
        assert_eq!(savings_fields(&[], d(624, 0)), None);
    }
}
