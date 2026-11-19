//! Wishes: planned purchases the owner saves toward, drops or buys.

use finance_types::gateway::{
    BillCreateRequest, BillEditRequest, BillRef, CardCharge, Direction, PlannedPayment, Repeat,
    Wish, WishDraft, WishKind, WishPurchase, WishSettings, WishSource, WishStatus,
};
use finance_types::report::wishes::Cut;
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::Date;
use rust_decimal::Decimal;

use super::bills::{Checks, archive_bill, create_bill, edit_bill};
use super::budget::save_budget_plan;
use super::{Writer, invalid};
use crate::budget::plan_for;
use crate::model::Kind;
use crate::plans::WISH_PLAN_PREFIX;
use crate::wishes::store::{load_wish_settings, load_wishes};
use crate::{Ctx, Result, Snapshot};

fn find(wishes: &[Wish], id: &str) -> Result<Wish> {
    wishes
        .iter()
        .find(|wish| wish.wish_id == id)
        .cloned()
        .ok_or_else(|| invalid("wish not found"))
}

pub(crate) fn store(w: &Writer<'_>, wish: &Wish) -> Result<()> {
    w.put_json(
        "INSERT INTO wishes (wish_id, payload_json) VALUES (?2, ?1) \
         ON CONFLICT(wish_id) DO UPDATE SET payload_json = excluded.payload_json",
        wish,
        &[&wish.wish_id],
    )
}

fn planned(wish: Wish) -> Result<Wish> {
    if wish.status == WishStatus::Planned {
        Ok(wish)
    } else {
        Err(invalid("wish is not planned"))
    }
}

/// A recurring wish's or bill change's end date and bill.
fn check_kind(snap: &Snapshot, kind: &WishKind, due_on: Date) -> Result<()> {
    let ends_on = match kind {
        WishKind::OneOff => return Ok(()),
        WishKind::Recurring { ends_on, .. } => ends_on,
        WishKind::BillChange { bill_id, ends_on } => {
            let bill = snap
                .bills
                .iter()
                .find(|b| &b.id == bill_id && b.active)
                .ok_or_else(|| invalid("bill is unknown"))?;
            // The judging knows these schedules only.
            if crate::wishes::repeat_of(&bill.repeat_freq, bill.skip).is_none() {
                return Err(invalid(
                    "only weekly, two-weekly, monthly or yearly bills can be changed here",
                ));
            }
            ends_on
        }
    };
    if let Some(ends_on) = ends_on {
        let ends_on: Date = ends_on
            .parse()
            .map_err(|_| invalid("end date is invalid"))?;
        if ends_on < due_on {
            return Err(invalid("end date is before the start"));
        }
    }
    Ok(())
}

/// Create a wish, or edit a planned one (keeping its progress).
pub fn save_wish(w: &Writer<'_>, snap: &Snapshot, draft: WishDraft) -> Result<Wish> {
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(invalid("wish needs a name"));
    }
    if matches!(draft.kind, WishKind::BillChange { .. }) {
        // A new amount of 0 cancels the bill.
        if draft.amount < Decimal::ZERO {
            return Err(invalid("the new amount cannot be negative"));
        }
    } else if draft.amount <= Decimal::ZERO {
        return Err(invalid("wish amount must be positive"));
    }
    let Ok(due_on) = draft.due_on.parse::<Date>() else {
        return Err(invalid("wish date is invalid"));
    };
    check_kind(snap, &draft.kind, due_on)?;
    if let WishSource::Account { account_id } = &draft.source
        && !snap.accounts.iter().any(|a| &a.account_id == account_id)
    {
        return Err(invalid("wish account is unknown"));
    }
    let wishes = load_wishes(&w.tx)?;
    if draft.kind != WishKind::OneOff {
        if draft.parent_id.is_some() {
            return Err(invalid("recurring wishes cannot be part of another wish"));
        }
        if let Some(id) = &draft.wish_id
            && wishes.iter().any(|x| x.parent_id.as_deref() == Some(id))
        {
            return Err(invalid("recurring wishes cannot have parts"));
        }
    }
    if let Some(parent_id) = &draft.parent_id {
        let parent =
            planned(find(&wishes, parent_id).map_err(|_| invalid("parent wish is not planned"))?)
                .map_err(|_| invalid("parent wish is not planned"))?;
        if parent.parent_id.is_some() {
            return Err(invalid("sub-wishes cannot have sub-wishes"));
        }
        if parent.kind != WishKind::OneOff {
            return Err(invalid("recurring wishes cannot have parts"));
        }
        if draft.wish_id.as_deref() == Some(parent_id.as_str()) {
            return Err(invalid("a wish cannot be its own parent"));
        }
        if draft.must && !parent.must {
            return Err(invalid(MUST_PARENT));
        }
        if let Some(id) = &draft.wish_id
            && wishes.iter().any(|x| x.parent_id.as_deref() == Some(id))
        {
            return Err(invalid("sub-wishes cannot have sub-wishes"));
        }
    }
    let wish = match &draft.wish_id {
        Some(id) => {
            let old = planned(find(&wishes, id)?)?;
            Wish {
                must: draft.must,
                parent_id: draft.parent_id,
                name: name.to_owned(),
                amount: draft.amount,
                currency: draft.currency,
                due_on: draft.due_on,
                source: draft.source,
                kind: draft.kind,
                ..old
            }
        }
        None => {
            // uuid4().hex: 32 random hex digits.
            let id: String =
                w.tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            Wish {
                wish_id: format!("wish-{id}"),
                parent_id: draft.parent_id,
                name: name.to_owned(),
                amount: draft.amount,
                currency: draft.currency,
                due_on: draft.due_on,
                source: draft.source,
                saved: Decimal::ZERO,
                saved_month: None,
                status: WishStatus::Planned,
                bought: None,
                kind: draft.kind,
                must: draft.must,
                created_at: w.stamp(),
            }
        }
    };
    store(w, &wish)?;
    if !wish.must {
        unmust_children(w, &wishes, &wish.wish_id)?;
    }
    Ok(wish)
}

const MUST_PARENT: &str = "only parts of a must wish can be must";

/// Clear the must flag on every planned sub-wish of `parent_id`.
fn unmust_children(w: &Writer<'_>, wishes: &[Wish], parent_id: &str) -> Result<()> {
    for child in wishes.iter().filter(|x| {
        x.parent_id.as_deref() == Some(parent_id) && x.status == WishStatus::Planned && x.must
    }) {
        store(
            w,
            &Wish {
                must: false,
                ..child.clone()
            },
        )?;
    }
    Ok(())
}

/// Flag a planned wish as must (or not). A sub-wish can be must only under
/// a must parent; clearing a parent clears its planned sub-wishes.
pub fn set_must(w: &Writer<'_>, wish_id: &str, must: bool) -> Result<Wish> {
    let wishes = load_wishes(&w.tx)?;
    let wish = planned(find(&wishes, wish_id)?)?;
    if must && let Some(parent_id) = &wish.parent_id {
        let parent_must = wishes.iter().any(|x| &x.wish_id == parent_id && x.must);
        if !parent_must {
            return Err(invalid(MUST_PARENT));
        }
    }
    let wish = Wish { must, ..wish };
    store(w, &wish)?;
    if !must {
        unmust_children(w, &wishes, wish_id)?;
    }
    Ok(wish)
}

/// Set the total saved so far toward a planned wish, in this budget month.
pub fn record_saved(
    w: &Writer<'_>,
    snap: &Snapshot,
    wish_id: &str,
    saved: Decimal,
) -> Result<Wish> {
    if saved < Decimal::ZERO {
        return Err(invalid("saved amount cannot be negative"));
    }
    let wish = planned(find(&load_wishes(&w.tx)?, wish_id)?)?;
    let wish = Wish {
        saved,
        saved_month: Some(Ctx::new(snap, &w.now).current_month().to_string()),
        ..wish
    };
    store(w, &wish)?;
    Ok(wish)
}

/// Drop a planned wish and its planned sub-wishes.
pub fn drop_wish(w: &Writer<'_>, wish_id: &str) -> Result<()> {
    let wishes = load_wishes(&w.tx)?;
    let wish = planned(find(&wishes, wish_id)?)?;
    store(
        w,
        &Wish {
            status: WishStatus::Dropped,
            ..wish
        },
    )?;
    for child in wishes
        .into_iter()
        .filter(|x| x.parent_id.as_deref() == Some(wish_id) && x.status == WishStatus::Planned)
    {
        store(
            w,
            &Wish {
                status: WishStatus::Dropped,
                ..child
            },
        )?;
    }
    Ok(())
}

fn put_settings(w: &Writer<'_>, settings: &WishSettings) -> Result<()> {
    w.put_json(
        "INSERT INTO wish_settings (singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        settings,
        &[],
    )
}

/// What each card takes of a card purchase: the owner's split, checked, or
/// (none given) the cards filled in list order up to their free limit for
/// `month`, the last card taking any overflow.
fn card_charges(
    settings: &WishSettings,
    purchase: &WishPurchase,
    month: Date,
) -> Result<Vec<CardCharge>> {
    if !purchase.cards.is_empty() {
        // A card that takes nothing is no charge: it must not touch the
        // card's recorded usage.
        let given: Vec<CardCharge> = purchase
            .cards
            .iter()
            .filter(|c| !c.amount.is_zero())
            .cloned()
            .collect();
        let known = given
            .iter()
            .all(|c| settings.cards.iter().any(|s| s.card_id == c.card_id));
        if !known {
            return Err(invalid("unknown card"));
        }
        let total: Decimal = given.iter().map(|c| c.amount).sum();
        if total != purchase.amount || given.iter().any(|c| c.amount < Decimal::ZERO) {
            return Err(invalid("card amounts must add up to the purchase"));
        }
        return Ok(given);
    }
    let mut left = purchase.amount;
    let mut charges = Vec::new();
    let last = settings.cards.len().saturating_sub(1);
    for (i, card) in settings.cards.iter().enumerate() {
        let used = match card.used_month.parse::<Date>() {
            Ok(d) if d.first_of_month() == month => card.used,
            _ => Decimal::ZERO,
        };
        let take = if i == last {
            left
        } else {
            left.min((card.limit - used).max(Decimal::ZERO))
        };
        if take > Decimal::ZERO {
            charges.push(CardCharge {
                card_id: card.card_id.clone(),
                amount: take,
            });
            left -= take;
        }
    }
    Ok(charges)
}

/// Buy a planned wish (on or before today). A card purchase raises the card
/// usage of its budget month, rolling an older recorded month forward, and
/// `cuts` trim the running budget month (the month the view calls month 0).
pub fn mark_bought(
    w: &Writer<'_>,
    snap: &Snapshot,
    wish_id: &str,
    purchase: WishPurchase,
    cuts: &[Cut],
) -> Result<Wish> {
    if purchase.amount <= Decimal::ZERO {
        return Err(invalid("purchase amount must be positive"));
    }
    let on: Date = purchase
        .on
        .parse()
        .map_err(|_| invalid("purchase date is invalid"))?;
    let ctx = Ctx::new(snap, &w.now);
    if on > ctx.today() {
        return Err(invalid("purchase date is in the future"));
    }
    if let WishSource::Account { account_id } = &purchase.source
        && !snap.accounts.iter().any(|a| &a.account_id == account_id)
    {
        return Err(invalid("wish account is unknown"));
    }
    let wish = planned(find(&load_wishes(&w.tx)?, wish_id)?)?;
    let month = ctx.resolver.period_for(on, None);
    let mut settings = load_wish_settings(&w.tx)?;
    let mut purchase = purchase;
    if purchase.source == WishSource::Card {
        purchase.cards = card_charges(&settings, &purchase, month)?;
    } else {
        purchase.cards.clear();
    }
    let bought = Wish {
        status: WishStatus::Bought,
        bought: Some(purchase.clone()),
        ..wish
    };
    store(w, &bought)?;
    store_purchase_plan(w, snap, &bought, &purchase, on)?;
    if purchase.source == WishSource::Card {
        let mut changed = false;
        for charge in &purchase.cards {
            let Some(card) = settings
                .cards
                .iter_mut()
                .find(|c| c.card_id == charge.card_id)
            else {
                continue;
            };
            let recorded = card
                .used_month
                .parse::<Date>()
                .ok()
                .map(|d| d.first_of_month());
            match recorded {
                // Bought before this budget month: already charged.
                _ if month < ctx.current_month() => {}
                Some(m) if m == month => {
                    card.used += charge.amount;
                    changed = true;
                }
                // The recorded month is later: this purchase was charged already.
                Some(m) if m > month => {}
                _ => {
                    card.used = charge.amount;
                    card.used_month = month.to_string();
                    changed = true;
                }
            }
        }
        if changed {
            put_settings(w, &settings)?;
        }
    }
    write_cuts(w, snap, ctx.current_month(), &bought.currency, cuts)?;
    Ok(bought)
}

/// Category a plan for a bought wish names when its payment is not imported yet.
const WISH_PLAN_CATEGORY: &str = "Wishes";

/// Record a bought wish as a one-off plan for the day it was bought, so its
/// payment counts as planned spending rather than unplanned. The plan matches
/// the payment by account (none for a card), amount and date; it takes the
/// category of a payment already imported, if one fits.
fn store_purchase_plan(
    w: &Writer<'_>,
    snap: &Snapshot,
    wish: &Wish,
    purchase: &WishPurchase,
    on: Date,
) -> Result<()> {
    let account_id = match &purchase.source {
        WishSource::Account { account_id } => Some(account_id.clone()),
        WishSource::Card => None,
    };
    let near = |d: Date| (d - on).get_days().abs() <= 7;
    let category = snap
        .raw_transactions
        .iter()
        .filter(|t| {
            t.kind == Kind::Outflow
                && t.amount == purchase.amount
                && t.currency == wish.currency
                && account_id.as_ref().is_none_or(|a| &t.account_id == a)
                && near(t.effective_on())
        })
        .min_by_key(|t| (t.effective_on() - on).get_days().abs())
        .and_then(|t| t.category.clone())
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| WISH_PLAN_CATEGORY.to_owned());
    let plan = PlannedPayment {
        plan_id: format!("{WISH_PLAN_PREFIX}{}", wish.wish_id),
        name: wish.name.clone(),
        match_category: category,
        direction: Direction::Outflow,
        amount: UnitMoney {
            amount: purchase.amount,
            currency: wish.currency.clone(),
        },
        due_on: purchase.on.clone(),
        created_at: w.stamp(),
        reimbursement_for: None,
        pending: false,
        account_id,
        match_description: None,
    };
    w.put_json(
        "INSERT INTO planned_payments (plan_id, payload_json) VALUES (?2, ?1) \
         ON CONFLICT(plan_id) DO UPDATE SET payload_json = excluded.payload_json",
        &plan,
        &[&plan.plan_id],
    )
}

/// Lower `month`'s budget groups by what each cut takes (`from − to`), so
/// cuts that earlier, unbought wishes would make are not written too.
/// Unknown groups are ignored.
fn write_cuts(
    w: &Writer<'_>,
    snap: &Snapshot,
    month: Date,
    currency: &Currency,
    cuts: &[Cut],
) -> Result<()> {
    if cuts.is_empty() {
        return Ok(());
    }
    let mut plan = plan_for(&snap.budget_plans, month, currency);
    for cut in cuts {
        if let Some(group) = plan.groups.iter_mut().find(|g| g.id == cut.group_id) {
            group.amount = (group.amount - (cut.from - cut.to)).max(Decimal::ZERO);
        }
    }
    save_budget_plan(w, snap, plan)?;
    Ok(())
}

/// Start a planned recurring wish (it becomes a bill in `match_category`)
/// or bill change (the bill takes the new amount, or is archived at 0);
/// `cuts` trim this budget month.
pub fn mark_started(
    w: &Writer<'_>,
    snap: &Snapshot,
    wish_id: &str,
    match_category: Option<String>,
    cuts: &[Cut],
) -> Result<Wish> {
    let wish = planned(find(&load_wishes(&w.tx)?, wish_id)?)?;
    match &wish.kind {
        WishKind::OneOff => {
            return Err(invalid("only recurring wishes and bill changes can start"));
        }
        WishKind::Recurring { every, ends_on } => {
            let Some(category) = match_category
                .map(|c| c.trim().to_owned())
                .filter(|c| !c.is_empty())
            else {
                return Err(invalid("choose the bill's category"));
            };
            let (repeat_freq, skip) = match every {
                Repeat::Weekly => ("weekly", 0),
                Repeat::Biweekly => ("weekly", 1),
                Repeat::Monthly => ("monthly", 0),
                Repeat::Yearly => ("yearly", 0),
            };
            let checks = Checks::new(snap, &w.now)?;
            create_bill(
                w,
                &checks,
                BillCreateRequest {
                    name: wish.name.clone(),
                    match_category: category,
                    amount_min: wish.amount,
                    amount_max: wish.amount,
                    currency_code: wish.currency.code.clone(),
                    date: wish.due_on.clone(),
                    repeat_freq: repeat_freq.to_owned(),
                    skip,
                    end_date: ends_on.clone(),
                },
            )?;
        }
        WishKind::BillChange { bill_id, .. } => {
            if wish.amount.is_zero() {
                archive_bill(
                    w,
                    BillRef {
                        bill_id: bill_id.clone(),
                    },
                )?;
            } else {
                let bill = snap
                    .bills
                    .iter()
                    .find(|b| &b.id == bill_id && b.active)
                    .ok_or_else(|| invalid("bill is unknown"))?;
                let checks = Checks::new(snap, &w.now)?;
                edit_bill(
                    w,
                    &checks,
                    BillEditRequest {
                        bill_id: bill_id.clone(),
                        name: bill.name.clone(),
                        amount_min: wish.amount,
                        amount_max: wish.amount,
                        repeat_freq: bill.repeat_freq.clone(),
                        date: None,
                    },
                )?;
            }
        }
    }
    let started = Wish {
        status: WishStatus::Started,
        ..wish
    };
    store(w, &started)?;
    let month = Ctx::new(snap, &w.now).current_month();
    write_cuts(w, snap, month, &started.currency, cuts)?;
    Ok(started)
}

/// Store the cards and their usage. Cards without an id get a fresh one.
pub fn save_wish_settings(w: &Writer<'_>, mut settings: WishSettings) -> Result<WishSettings> {
    if settings.cards.is_empty() {
        return Err(invalid("at least one card is needed"));
    }
    for card in &mut settings.cards {
        let name = card.name.trim();
        if name.is_empty()
            || name.chars().count() > 64
            || card.limit <= Decimal::ZERO
            || card.used < Decimal::ZERO
            || !card.used_month.ends_with("-01")
            || card.used_month.parse::<Date>().is_err()
        {
            return Err(invalid("invalid card settings"));
        }
        card.name = name.to_owned();
        if card.card_id.trim().is_empty() {
            let id: String =
                w.tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            card.card_id = format!("card-{id}");
        }
    }
    let mut ids: Vec<&str> = settings.cards.iter().map(|c| c.card_id.as_str()).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("card ids must be unique"));
    }
    if settings.emergency_months > WishSettings::MAX_EMERGENCY_MONTHS {
        return Err(invalid("the emergency fund covers at most 36 months"));
    }
    let mut seen = std::collections::HashSet::new();
    settings.essential_groups = settings
        .essential_groups
        .iter()
        .map(|g| g.trim().to_owned())
        .filter(|g| !g.is_empty() && g.chars().count() <= 64 && seen.insert(g.to_lowercase()))
        .take(20)
        .collect();
    put_settings(w, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::fixture::{Db, db};
    use finance_types::gateway::{BudgetGroup, BudgetPlan, CardSettings, Repeat, WishKind};
    use finance_types::report::Currency;

    fn draft(name: &str) -> WishDraft {
        WishDraft {
            wish_id: None,
            parent_id: None,
            name: name.into(),
            amount: Decimal::from(900),
            currency: Currency::new("EUR"),
            due_on: "2026-12-01".into(),
            source: WishSource::Card,
            kind: Default::default(),
            must: false,
        }
    }

    fn message<T>(result: Result<T>) -> String {
        result.err().expect("rejected").to_string()
    }

    fn card(id: &str, name: &str, limit: &str, used: &str, month: &str) -> CardSettings {
        CardSettings {
            card_id: id.into(),
            name: name.into(),
            limit: limit.parse().unwrap(),
            used: used.parse().unwrap(),
            used_month: month.into(),
            bill_id: None,
        }
    }

    #[test]
    fn must_cascades_and_is_bounded_by_parent() {
        let mut db = db();
        let parent = db.write(|w, s| save_wish(w, s, draft("Trip"))).unwrap();
        let sub_draft = WishDraft {
            parent_id: Some(parent.wish_id.clone()),
            ..draft("Flights")
        };
        let sub = db.write(|w, s| save_wish(w, s, sub_draft)).unwrap();
        let err = db.write(|w, _| set_must(w, &sub.wish_id, true));
        assert_eq!(message(err), "only parts of a must wish can be must");
        let must_sub = WishDraft {
            parent_id: Some(parent.wish_id.clone()),
            must: true,
            ..draft("Hotel")
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, must_sub))),
            "only parts of a must wish can be must"
        );
        db.write(|w, _| set_must(w, &parent.wish_id, true)).unwrap();
        db.write(|w, _| set_must(w, &sub.wish_id, true)).unwrap();
        db.write(|w, _| set_must(w, &parent.wish_id, false))
            .unwrap();
        let wishes = load_wishes(db.store.connection()).unwrap();
        assert!(wishes.iter().all(|w| !w.must));
    }

    #[test]
    fn edit_unflagging_parent_cascades() {
        let mut db = db();
        let parent = db
            .write(|w, s| {
                save_wish(
                    w,
                    s,
                    WishDraft {
                        must: true,
                        ..draft("Trip")
                    },
                )
            })
            .unwrap();
        let sub = WishDraft {
            parent_id: Some(parent.wish_id.clone()),
            must: true,
            ..draft("Flights")
        };
        db.write(|w, s| save_wish(w, s, sub)).unwrap();
        let edit = WishDraft {
            wish_id: Some(parent.wish_id.clone()),
            must: false,
            ..draft("Trip")
        };
        db.write(|w, s| save_wish(w, s, edit)).unwrap();
        let wishes = load_wishes(db.store.connection()).unwrap();
        assert_eq!(wishes.len(), 2);
        assert!(wishes.iter().all(|w| !w.must));
    }

    #[test]
    fn edit_sub_must_under_optional_parent_is_refused() {
        let mut db = db();
        let parent = db.write(|w, s| save_wish(w, s, draft("Trip"))).unwrap();
        let sub_draft = WishDraft {
            parent_id: Some(parent.wish_id.clone()),
            ..draft("Flights")
        };
        let sub = db.write(|w, s| save_wish(w, s, sub_draft)).unwrap();
        let edit = WishDraft {
            wish_id: Some(sub.wish_id.clone()),
            parent_id: Some(parent.wish_id.clone()),
            must: true,
            ..draft("Flights")
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, edit))),
            "only parts of a must wish can be must"
        );
        let wishes = load_wishes(db.store.connection()).unwrap();
        assert!(wishes.iter().all(|w| !w.must));
    }

    #[test]
    fn set_must_on_non_planned_wish_is_refused() {
        let mut db = db();
        let bought = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &bought.wish_id,
                purchase(100, "2026-10-26", WishSource::Card),
                &[],
            )
        })
        .unwrap();
        let dropped = db.write(|w, s| save_wish(w, s, draft("Lamp"))).unwrap();
        db.write(|w, _| drop_wish(w, &dropped.wish_id)).unwrap();
        for id in [&bought.wish_id, &dropped.wish_id] {
            assert_eq!(
                message(db.write(|w, _| set_must(w, id, true))),
                "wish is not planned"
            );
        }
    }

    #[test]
    fn save_wish_honours_must_on_create() {
        let mut db = db();
        let wish = db
            .write(|w, s| {
                save_wish(
                    w,
                    s,
                    WishDraft {
                        must: true,
                        ..draft("Rent deposit")
                    },
                )
            })
            .unwrap();
        assert!(wish.must);
    }

    #[test]
    fn settings_need_valid_cards_and_mint_ids() {
        let mut db = db();
        let bad = WishSettings {
            cards: vec![],
            ..WishSettings::default()
        };
        assert!(db.write(|w, _| save_wish_settings(w, bad)).is_err());
        let long = WishSettings {
            cards: vec![card("a", "A", "10", "0", "2026-10-01")],
            emergency_months: 37,
            ..WishSettings::default()
        };
        assert!(db.write(|w, _| save_wish_settings(w, long)).is_err());
        let untidy = WishSettings {
            cards: vec![card("a", "A", "10", "0", "2026-10-01")],
            essential_groups: vec![" Groceries ".into(), "".into(), "groceries".into()],
            ..WishSettings::default()
        };
        let saved = db.write(|w, _| save_wish_settings(w, untidy)).unwrap();
        assert_eq!(saved.essential_groups, ["Groceries"]);
        let two = WishSettings {
            cards: vec![
                card("", "Visa", "1000", "0", "2026-10-01"),
                card("mc", "MC", "500", "20", "2026-10-01"),
            ],
            ..WishSettings::default()
        };
        let saved = db.write(|w, _| save_wish_settings(w, two)).unwrap();
        assert!(saved.cards[0].card_id.starts_with("card-") && saved.cards[0].card_id.len() > 5);
        assert_eq!(saved.cards[1].card_id, "mc");
        for bad in [
            card("x", "", "1000", "0", "2026-10-01"),
            card("x", &"n".repeat(65), "1000", "0", "2026-10-01"),
            card("x", "A", "0", "0", "2026-10-01"),
            card("x", "A", "10", "-1", "2026-10-01"),
            card("x", "A", "10", "0", "2026-10-15"),
        ] {
            let s = WishSettings {
                cards: vec![bad],
                ..WishSettings::default()
            };
            assert!(db.write(|w, _| save_wish_settings(w, s)).is_err());
        }
        let dup = WishSettings {
            cards: vec![
                card("a", "A", "10", "0", "2026-10-01"),
                card("a", "B", "10", "0", "2026-10-01"),
            ],
            ..WishSettings::default()
        };
        assert!(db.write(|w, _| save_wish_settings(w, dup)).is_err());
    }

    #[test]
    fn save_then_load_wish() {
        let mut db = db();
        let wish = db
            .write(|w, snap| save_wish(w, snap, draft("Bike")))
            .unwrap();
        assert!(wish.wish_id.starts_with("wish-") && wish.wish_id.len() == 5 + 32);
        assert_eq!(wish.status, WishStatus::Planned);
        assert_eq!(wish.saved, Decimal::ZERO);
        assert_eq!(
            load_wishes(db.store.connection()).unwrap(),
            vec![wish.clone()]
        );

        let mut edit = draft("Better bike");
        edit.wish_id = Some(wish.wish_id.clone());
        let saved = db
            .write(|w, s| record_saved(w, s, &wish.wish_id, Decimal::from(40)))
            .unwrap();
        assert_eq!(saved.saved_month.as_deref(), Some("2026-10-01"));
        let edited = db.write(|w, snap| save_wish(w, snap, edit)).unwrap();
        assert_eq!(edited.name, "Better bike");
        assert_eq!(edited.saved, Decimal::from(40));
        assert_eq!(edited.created_at, wish.created_at);
        assert_eq!(load_wishes(db.store.connection()).unwrap().len(), 1);
    }

    #[test]
    fn rejects_empty_name_and_bad_date() {
        let mut db = db();
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, draft("  ")))),
            "wish needs a name"
        );
        let mut bad = draft("Bike");
        bad.due_on = "soon".into();
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, bad))),
            "wish date is invalid"
        );
        let mut free = draft("Bike");
        free.amount = Decimal::ZERO;
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, free))),
            "wish amount must be positive"
        );
        let mut account = draft("Bike");
        account.source = WishSource::Account {
            account_id: "firefly:999".into(),
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, account))),
            "wish account is unknown"
        );
        let mut known = draft("Bike");
        known.source = WishSource::Account {
            account_id: "firefly:1".into(),
        };
        db.write(|w, s| save_wish(w, s, known)).unwrap();
    }

    #[test]
    fn sub_wish_of_sub_wish_rejected() {
        let mut db = db();
        let parent = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut child = draft("Lock");
        child.parent_id = Some(parent.wish_id.clone());
        let child = db.write(|w, s| save_wish(w, s, child)).unwrap();
        let mut grandchild = draft("Key");
        grandchild.parent_id = Some(child.wish_id.clone());
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, grandchild))),
            "sub-wishes cannot have sub-wishes"
        );
        let mut orphan = draft("Bell");
        orphan.parent_id = Some("wish-nope".into());
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, orphan))),
            "parent wish is not planned"
        );
        // A parent cannot become a child, nor a wish its own parent.
        let mut demote = draft("Bike");
        demote.wish_id = Some(parent.wish_id.clone());
        demote.parent_id = Some(child.wish_id.clone());
        assert!(db.write(|w, s| save_wish(w, s, demote)).is_err());
        let mut itself = draft("Bike");
        itself.wish_id = Some(parent.wish_id.clone());
        itself.parent_id = Some(parent.wish_id.clone());
        assert!(db.write(|w, s| save_wish(w, s, itself)).is_err());
    }

    #[test]
    fn record_saved_checks_amount_and_status() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        assert!(
            db.write(|w, s| record_saved(w, s, &wish.wish_id, Decimal::from(-1)))
                .is_err()
        );
        assert!(
            db.write(|w, s| record_saved(w, s, "wish-nope", Decimal::ONE))
                .is_err()
        );
        db.write(|w, _| drop_wish(w, &wish.wish_id)).unwrap();
        assert_eq!(
            message(db.write(|w, s| record_saved(w, s, &wish.wish_id, Decimal::ONE))),
            "wish is not planned"
        );
    }

    #[test]
    fn drop_parent_drops_children() {
        let mut db = db();
        let parent = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut child = draft("Lock");
        child.parent_id = Some(parent.wish_id.clone());
        db.write(|w, s| save_wish(w, s, child)).unwrap();
        db.write(|w, s| save_wish(w, s, draft("Phone"))).unwrap();
        db.write(|w, _| drop_wish(w, &parent.wish_id)).unwrap();
        let wishes = load_wishes(db.store.connection()).unwrap();
        let dropped = wishes
            .iter()
            .filter(|w| w.status == WishStatus::Dropped)
            .count();
        assert_eq!((wishes.len(), dropped), (3, 2));
    }

    fn purchase(amount: i64, on: &str, source: WishSource) -> WishPurchase {
        WishPurchase {
            amount: Decimal::from(amount),
            on: on.into(),
            source,
            cards: Vec::new(),
        }
    }

    fn settings(used: &str, month: &str) -> WishSettings {
        WishSettings {
            cards: vec![CardSettings {
                card_id: "card-1".into(),
                name: "Credit card".into(),
                limit: Decimal::from(2500),
                used: used.parse().unwrap(),
                used_month: month.into(),
                bill_id: None,
            }],
            ..WishSettings::default()
        }
    }

    fn db_with_cards(cards: &[(&str, &str, &str)]) -> Db {
        let mut db = db();
        let cards = cards
            .iter()
            .map(|(id, limit, used)| CardSettings {
                card_id: (*id).into(),
                name: (*id).into(),
                limit: limit.parse().unwrap(),
                used: used.parse().unwrap(),
                used_month: "2026-10-01".into(),
                bill_id: None,
            })
            .collect();
        db.write(|w, _| {
            save_wish_settings(
                w,
                WishSettings {
                    cards,
                    ..WishSettings::default()
                },
            )
        })
        .unwrap();
        db
    }

    fn charge(id: &str, amount: &str) -> CardCharge {
        CardCharge {
            card_id: id.into(),
            amount: amount.parse().unwrap(),
        }
    }

    #[test]
    fn buying_a_wish_plans_its_payment() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Zoo"))).unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(114, "2026-10-26", WishSource::Card),
                &[],
            )
        })
        .unwrap();
        let plans: Vec<PlannedPayment> = db
            .store
            .connection()
            .prepare("SELECT payload_json FROM planned_payments")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|json| serde_json::from_str(&json.unwrap()).unwrap())
            .collect();
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(plan.plan_id, format!("wish-{}", wish.wish_id));
        assert_eq!(plan.name, "Zoo");
        assert_eq!(plan.amount.amount, Decimal::from(114));
        assert_eq!(plan.due_on, "2026-10-26");
        assert_eq!(plan.direction, Direction::Outflow);
        assert_eq!(plan.account_id, None);
        assert_eq!(plan.match_category, "Wishes");
    }

    #[test]
    fn bought_card_amounts_must_sum() {
        let mut db = db_with_cards(&[("visa", "100", "30"), ("mc", "200", "0")]);
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut bad = purchase(150, "2026-10-26", WishSource::Card);
        bad.cards = vec![charge("visa", "100")];
        assert_eq!(
            message(db.write(|w, s| mark_bought(w, s, &wish.wish_id, bad, &[]))),
            "card amounts must add up to the purchase"
        );
        let mut unknown = purchase(150, "2026-10-26", WishSource::Card);
        unknown.cards = vec![charge("amex", "150")];
        assert_eq!(
            message(db.write(|w, s| mark_bought(w, s, &wish.wish_id, unknown, &[]))),
            "unknown card"
        );
        let auto = purchase(150, "2026-10-26", WishSource::Card);
        let bought = db
            .write(|w, s| mark_bought(w, s, &wish.wish_id, auto, &[]))
            .unwrap();
        assert_eq!(
            bought.bought.unwrap().cards,
            vec![charge("visa", "70"), charge("mc", "80")]
        );
        let s = load_wish_settings(db.store.connection()).unwrap();
        assert_eq!(s.cards[0].used, Decimal::from(100));
        assert_eq!(s.cards[1].used, Decimal::from(80));
    }

    #[test]
    fn bought_with_explicit_split_raises_each_card() {
        let mut db = db_with_cards(&[("visa", "100", "30"), ("mc", "200", "0")]);
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut p = purchase(150, "2026-10-26", WishSource::Card);
        p.cards = vec![charge("visa", "10"), charge("mc", "140")];
        db.write(|w, s| mark_bought(w, s, &wish.wish_id, p, &[]))
            .unwrap();
        let s = load_wish_settings(db.store.connection()).unwrap();
        assert_eq!(s.cards[0].used, Decimal::from(40));
        assert_eq!(s.cards[1].used, Decimal::from(140));
    }

    #[test]
    fn zero_card_charge_leaves_a_stale_card_alone() {
        let mut db = db();
        let cards = vec![
            card("visa", "visa", "1000", "30", "2026-10-01"),
            card("mc", "mc", "1000", "70", "2026-09-01"),
        ];
        db.write(|w, _| {
            save_wish_settings(
                w,
                WishSettings {
                    cards,
                    ..WishSettings::default()
                },
            )
        })
        .unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut p = purchase(150, "2026-10-26", WishSource::Card);
        p.cards = vec![charge("visa", "150"), charge("mc", "0")];
        let bought = db
            .write(|w, s| mark_bought(w, s, &wish.wish_id, p, &[]))
            .unwrap();
        assert_eq!(bought.bought.unwrap().cards, vec![charge("visa", "150")]);
        let s = load_wish_settings(db.store.connection()).unwrap();
        assert_eq!(s.cards[0].used, Decimal::from(180));
        assert_eq!(s.cards[1].used, Decimal::from(70));
        assert_eq!(s.cards[1].used_month, "2026-09-01");
    }

    #[test]
    fn auto_split_overflow_goes_to_last_card() {
        let mut db = db_with_cards(&[("visa", "100", "30"), ("mc", "50", "0")]);
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let bought = db
            .write(|w, s| {
                mark_bought(
                    w,
                    s,
                    &wish.wish_id,
                    purchase(200, "2026-10-26", WishSource::Card),
                    &[],
                )
            })
            .unwrap();
        assert_eq!(
            bought.bought.unwrap().cards,
            vec![charge("visa", "70"), charge("mc", "130")]
        );
    }

    #[test]
    fn mark_bought_on_card_raises_usage() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        db.write(|w, _| save_wish_settings(w, settings("9.98", "2026-10-01")))
            .unwrap();
        let bought = db
            .write(|w, s| {
                mark_bought(
                    w,
                    s,
                    &wish.wish_id,
                    purchase(100, "2026-10-26", WishSource::Card),
                    &[],
                )
            })
            .unwrap();
        assert_eq!(bought.status, WishStatus::Bought);
        assert_eq!(bought.bought.unwrap().amount, Decimal::from(100));
        assert_eq!(
            load_wish_settings(db.store.connection()).unwrap().cards[0].used,
            "109.98".parse::<Decimal>().unwrap()
        );
        assert_eq!(
            message(db.write(|w, s| mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(1, "2026-10-26", WishSource::Card),
                &[]
            ))),
            "wish is not planned"
        );
    }

    #[test]
    fn mark_bought_in_an_earlier_card_month_leaves_usage() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        db.write(|w, _| save_wish_settings(w, settings("9.98", "2026-10-01")))
            .unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-09-28", WishSource::Card),
                &[],
            )
        })
        .unwrap();
        assert_eq!(
            load_wish_settings(db.store.connection()).unwrap().cards[0].used,
            "9.98".parse::<Decimal>().unwrap()
        );
    }

    #[test]
    fn card_purchase_before_this_month_leaves_unset_usage() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-09-28", WishSource::Card),
                &[],
            )
        })
        .unwrap();
        assert_eq!(
            load_wish_settings(db.store.connection()).unwrap(),
            WishSettings::default()
        );
    }

    #[test]
    fn mark_bought_rejects_a_future_date() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        assert_eq!(
            message(db.write(|w, s| mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-10-27", WishSource::Card),
                &[]
            ))),
            "purchase date is in the future"
        );
    }

    #[test]
    fn card_purchase_rolls_old_usage_forward() {
        for old in [settings("9.98", "2026-09-01"), WishSettings::default()] {
            let mut db = db();
            let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
            db.write(|w, _| {
                put_settings(w, &old)?;
                Ok(())
            })
            .unwrap();
            db.write(|w, s| {
                mark_bought(
                    w,
                    s,
                    &wish.wish_id,
                    purchase(100, "2026-10-20", WishSource::Card),
                    &[],
                )
            })
            .unwrap();
            let now = load_wish_settings(db.store.connection()).unwrap();
            assert_eq!(now.cards[0].used, Decimal::from(100));
            assert_eq!(now.cards[0].used_month, "2026-10-01");
        }
    }

    fn going_out(month: &str, amount: i64) -> BudgetPlan {
        BudgetPlan {
            month: month.into(),
            currency: Currency::new("EUR"),
            revision: 0,
            groups: vec![BudgetGroup {
                id: "going-out".into(),
                name: "Going out".into(),
                amount: Decimal::from(amount),
                source_groups: Vec::new(),
                categories: Vec::new(),
                transaction_ids: Vec::new(),
                account_id: None,
                importance: Some(2),
                floor: None,
            }],
        }
    }

    fn stored_plan(db: &Db, month: &str) -> BudgetPlan {
        serde_json::from_str(&db.one::<String>(&format!(
            "SELECT payload_json FROM monthly_budget_plans WHERE month = '{month}'"
        )))
        .unwrap()
    }

    #[test]
    fn card_purchase_after_early_cutoff_counts_in_the_budget_month() {
        let mut db = db();
        // October closed early on the 25th: the 26th belongs to November.
        db.store
            .connection()
            .execute(
                "INSERT INTO month_periods(month, revision, updated_at, payload_json) \
                 VALUES ('2026-10-01', 1, '2026-10-25T10:00:00Z', ?1)",
                [serde_json::json!({
                    "month": "2026-10-01",
                    "starts_at": "2026-10-01T00:00:00+02:00",
                    "cutoff_at": "2026-10-25T10:00:00Z",
                })
                .to_string()],
            )
            .unwrap();
        db.write(|w, s| save_budget_plan(w, s, going_out("2026-11-01", 120)))
            .unwrap();
        db.write(|w, _| save_wish_settings(w, settings("9.98", "2026-11-01")))
            .unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let cut = Cut {
            group_id: "going-out".into(),
            name: "Going out".into(),
            from: Decimal::from(120),
            to: Decimal::from(100),
        };
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-10-26", WishSource::Card),
                &[cut],
            )
        })
        .unwrap();
        assert_eq!(
            load_wish_settings(db.store.connection()).unwrap().cards[0].used,
            "109.98".parse::<Decimal>().unwrap()
        );
        assert_eq!(
            stored_plan(&db, "2026-11-01").groups[0].amount,
            Decimal::from(100)
        );
    }

    #[test]
    fn cuts_into_a_month_without_a_plan_keep_the_owners_last_one() {
        let mut db = db();
        let mut september = going_out("2026-09-01", 120);
        september.groups[0].floor = Some(Decimal::from(40));
        db.write(|w, s| save_budget_plan(w, s, september)).unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let cut = Cut {
            group_id: "going-out".into(),
            name: "Going out".into(),
            from: Decimal::from(120),
            to: Decimal::from(70),
        };
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(50, "2026-10-26", WishSource::Card),
                &[cut],
            )
        })
        .unwrap();
        // October had no plan: the cut lands in September's, not the default.
        let october = stored_plan(&db, "2026-10-01");
        assert_eq!(october.groups.len(), 1);
        let going_out = &october.groups[0];
        assert_eq!(going_out.id, "going-out");
        assert_eq!(going_out.amount, Decimal::from(70));
        assert_eq!(going_out.importance, Some(2));
        assert_eq!(going_out.floor, Some(Decimal::from(40)));
    }

    #[test]
    fn cuts_take_only_this_wishs_share() {
        let mut db = db();
        db.write(|w, s| save_budget_plan(w, s, going_out("2026-10-01", 120)))
            .unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        // An unbought wish judged first took going out from 120 to 70.
        let cut = Cut {
            group_id: "going-out".into(),
            name: "Going out".into(),
            from: Decimal::from(70),
            to: Decimal::from(40),
        };
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(30, "2026-10-26", WishSource::Card),
                &[cut],
            )
        })
        .unwrap();
        assert_eq!(
            stored_plan(&db, "2026-10-01").groups[0].amount,
            Decimal::from(90)
        );
    }

    #[test]
    fn mark_bought_writes_cuts_into_budget() {
        let mut db = db();
        let going_out = BudgetGroup {
            id: "going-out".into(),
            name: "Going out".into(),
            amount: Decimal::from(120),
            source_groups: Vec::new(),
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance: Some(2),
            floor: None,
        };
        let plan = BudgetPlan {
            month: "2026-10-01".into(),
            currency: Currency::new("EUR"),
            revision: 0,
            groups: vec![going_out],
        };
        db.write(|w, s| save_budget_plan(w, s, plan)).unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let cuts = [
            Cut {
                group_id: "going-out".into(),
                name: "Going out".into(),
                from: Decimal::from(120),
                to: Decimal::from(70),
            },
            Cut {
                group_id: "ghost".into(),
                name: "Ghost".into(),
                from: Decimal::from(5),
                to: Decimal::ONE,
            },
        ];
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-10-26", WishSource::Card),
                &cuts,
            )
        })
        .unwrap();
        let stored: BudgetPlan = serde_json::from_str(&db.one::<String>(
            "SELECT payload_json FROM monthly_budget_plans WHERE month = '2026-10-01'",
        ))
        .unwrap();
        assert_eq!(stored.revision, 2);
        assert_eq!(stored.groups.len(), 1);
        assert_eq!(stored.groups[0].amount, Decimal::from(70));
    }

    #[test]
    fn mark_bought_parent_keeps_children_planned() {
        let mut db = db();
        let parent = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let mut child = draft("Lock");
        child.parent_id = Some(parent.wish_id.clone());
        db.write(|w, s| save_wish(w, s, child)).unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &parent.wish_id,
                purchase(
                    900,
                    "2026-10-26",
                    WishSource::Account {
                        account_id: "firefly:1".into(),
                    },
                ),
                &[],
            )
        })
        .unwrap();
        let wishes = load_wishes(db.store.connection()).unwrap();
        let planned = wishes
            .iter()
            .filter(|w| w.status == WishStatus::Planned)
            .count();
        assert_eq!(planned, 1);
    }

    fn add_gym(db: &Db) {
        db.store
            .connection()
            .execute_batch(
                "INSERT INTO bills (id, name, currency, amount_min, amount_max, start_date, repeat_freq) \
                 VALUES (7, 'Gym', 'EUR', '30.000000000000', '30.000000000000', '2026-09-03', 'monthly'), \
                 (8, 'Insurance', 'EUR', '90.000000000000', '90.000000000000', '2026-09-03', 'quarterly'); \
                 INSERT INTO bills (id, name, currency, amount_min, amount_max, start_date, repeat_freq, skip) \
                 VALUES (9, 'Therapy', 'EUR', '80.000000000000', '80.000000000000', '2026-09-03', 'weekly', 1), \
                 (10, 'Haircut', 'EUR', '25.000000000000', '25.000000000000', '2026-09-03', 'monthly', 1); \
                 INSERT INTO import_runs (source, finished_at, stats) \
                 VALUES ('test', '2026-10-26T19:00:00Z', '{}');",
            )
            .unwrap();
    }

    fn recurring(name: &str, every: Repeat) -> WishDraft {
        WishDraft {
            amount: Decimal::from(60),
            due_on: "2026-11-04".into(),
            kind: WishKind::Recurring {
                every,
                ends_on: None,
            },
            ..draft(name)
        }
    }

    fn bill_change(amount: i64) -> WishDraft {
        WishDraft {
            amount: Decimal::from(amount),
            due_on: "2026-11-01".into(),
            kind: WishKind::BillChange {
                bill_id: "7".into(),
                ends_on: None,
            },
            ..draft("Gym")
        }
    }

    fn bill(db: &Db, id: i64) -> (String, String, String, String, i64, i64) {
        db.store
            .connection()
            .query_row(
                "SELECT name, amount_min, amount_max, repeat_freq, skip, active FROM bills WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap()
    }

    #[test]
    fn start_biweekly_creates_weekly_bill_skip_1() {
        let mut db = db();
        let wish = db
            .write(|w, s| save_wish(w, s, recurring("Psychologist", Repeat::Biweekly)))
            .unwrap();
        assert_eq!(
            message(db.write(|w, s| mark_started(w, s, &wish.wish_id, None, &[]))),
            "choose the bill's category"
        );
        let started = db
            .write(|w, s| mark_started(w, s, &wish.wish_id, Some("Groceries".into()), &[]))
            .unwrap();
        assert_eq!(started.status, WishStatus::Started);
        let id: i64 = db.one("SELECT id FROM bills WHERE name = 'Psychologist'");
        assert_eq!(
            bill(&db, id),
            (
                "Psychologist".into(),
                "60.000000000000".into(),
                "60.000000000000".into(),
                "weekly".into(),
                1,
                1
            )
        );
        let start: String = db.one("SELECT start_date FROM bills WHERE name = 'Psychologist'");
        assert_eq!(start, "2026-11-04");
        let snap = db.snap();
        let stored = snap.bills.iter().find(|b| b.id == id.to_string()).unwrap();
        assert_eq!(stored.skip, 1);
        assert_eq!(
            load_wishes(db.store.connection()).unwrap()[0].status,
            WishStatus::Started
        );
        assert_eq!(
            message(db.write(|w, s| mark_started(
                w,
                s,
                &wish.wish_id,
                Some("Groceries".into()),
                &[]
            ))),
            "wish is not planned"
        );
    }

    #[test]
    fn start_keeps_the_end_date() {
        let mut db = db();
        let mut draft = recurring("Course", Repeat::Monthly);
        draft.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: Some("2027-03-04".into()),
        };
        let wish = db.write(|w, s| save_wish(w, s, draft)).unwrap();
        db.write(|w, s| mark_started(w, s, &wish.wish_id, Some("Groceries".into()), &[]))
            .unwrap();
        let end: Option<String> = db.one("SELECT end_date FROM bills WHERE name = 'Course'");
        assert_eq!(end.as_deref(), Some("2027-03-04"));
        let snap = db.snap();
        let bill = snap.bills.iter().find(|b| b.name == "Course").unwrap();
        assert_eq!(bill.end, Some("2027-03-04".parse().unwrap()));
    }

    #[test]
    fn start_rejects_one_off_and_unknown_category() {
        let mut db = db();
        let one_off = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        assert_eq!(
            message(db.write(|w, s| mark_started(w, s, &one_off.wish_id, None, &[]))),
            "only recurring wishes and bill changes can start"
        );
        let wish = db
            .write(|w, s| save_wish(w, s, recurring("Netflix", Repeat::Monthly)))
            .unwrap();
        assert!(
            db.write(|w, s| mark_started(w, s, &wish.wish_id, Some("Nope".into()), &[]))
                .is_err()
        );
        assert_eq!(
            load_wishes(db.store.connection())
                .unwrap()
                .iter()
                .find(|x| x.wish_id == wish.wish_id)
                .unwrap()
                .status,
            WishStatus::Planned
        );
    }

    #[test]
    fn start_bill_change_edits_amount() {
        let mut db = db();
        add_gym(&db);
        let wish = db.write(|w, s| save_wish(w, s, bill_change(45))).unwrap();
        let going_out = BudgetGroup {
            id: "going-out".into(),
            name: "Going out".into(),
            amount: Decimal::from(120),
            source_groups: Vec::new(),
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: None,
            importance: Some(2),
            floor: None,
        };
        let plan = BudgetPlan {
            month: "2026-10-01".into(),
            currency: Currency::new("EUR"),
            revision: 0,
            groups: vec![going_out],
        };
        db.write(|w, s| save_budget_plan(w, s, plan)).unwrap();
        let cuts = [Cut {
            group_id: "going-out".into(),
            name: "Going out".into(),
            from: Decimal::from(120),
            to: Decimal::from(105),
        }];
        let started = db
            .write(|w, s| mark_started(w, s, &wish.wish_id, None, &cuts))
            .unwrap();
        assert_eq!(started.status, WishStatus::Started);
        assert_eq!(
            bill(&db, 7),
            (
                "Gym".into(),
                "45.000000000000".into(),
                "45.000000000000".into(),
                "monthly".into(),
                0,
                1
            )
        );
        let start: String = db.one("SELECT start_date FROM bills WHERE id = 7");
        assert_eq!(start, "2026-09-03");
        let stored: BudgetPlan = serde_json::from_str(&db.one::<String>(
            "SELECT payload_json FROM monthly_budget_plans WHERE month = '2026-10-01'",
        ))
        .unwrap();
        assert_eq!(stored.groups[0].amount, Decimal::from(105));
    }

    #[test]
    fn start_cancel_archives_bill() {
        let mut db = db();
        add_gym(&db);
        let wish = db.write(|w, s| save_wish(w, s, bill_change(0))).unwrap();
        db.write(|w, s| mark_started(w, s, &wish.wish_id, None, &[]))
            .unwrap();
        assert_eq!(bill(&db, 7).5, 0);
        assert_eq!(
            load_wishes(db.store.connection()).unwrap()[0].status,
            WishStatus::Started
        );
    }

    #[test]
    fn recurring_cannot_have_children() {
        let mut db = db();
        let monthly = db
            .write(|w, s| save_wish(w, s, recurring("Netflix", Repeat::Monthly)))
            .unwrap();
        let mut child = draft("Popcorn");
        child.parent_id = Some(monthly.wish_id.clone());
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, child))),
            "recurring wishes cannot have parts"
        );
        let parent = db.write(|w, s| save_wish(w, s, draft("Trip"))).unwrap();
        let mut sub = recurring("Hotel", Repeat::Weekly);
        sub.parent_id = Some(parent.wish_id.clone());
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, sub))),
            "recurring wishes cannot be part of another wish"
        );
        // A parent with parts cannot turn recurring.
        let mut part = draft("Flights");
        part.parent_id = Some(parent.wish_id.clone());
        db.write(|w, s| save_wish(w, s, part)).unwrap();
        let mut turn = recurring("Trip", Repeat::Monthly);
        turn.wish_id = Some(parent.wish_id.clone());
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, turn))),
            "recurring wishes cannot have parts"
        );
    }

    #[test]
    fn recurring_dates_and_bills_are_validated() {
        let mut db = db();
        add_gym(&db);
        let mut bad_end = recurring("Netflix", Repeat::Monthly);
        bad_end.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: Some("later".into()),
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, bad_end))),
            "end date is invalid"
        );
        let mut early_end = recurring("Netflix", Repeat::Monthly);
        early_end.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: Some("2026-11-03".into()),
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, early_end))),
            "end date is before the start"
        );
        let mut same_day = recurring("Netflix", Repeat::Monthly);
        same_day.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: Some("2026-11-04".into()),
        };
        db.write(|w, s| save_wish(w, s, same_day)).unwrap();
        let mut unknown = bill_change(40);
        unknown.kind = WishKind::BillChange {
            bill_id: "99".into(),
            ends_on: None,
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, unknown))),
            "bill is unknown"
        );
        let mut quarterly = bill_change(40);
        quarterly.kind = WishKind::BillChange {
            bill_id: "8".into(),
            ends_on: None,
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, quarterly))),
            "only weekly, two-weekly, monthly or yearly bills can be changed here"
        );
        let mut biweekly = bill_change(40);
        biweekly.kind = WishKind::BillChange {
            bill_id: "9".into(),
            ends_on: None,
        };
        db.write(|w, s| save_wish(w, s, biweekly)).unwrap();
        let mut bimonthly = bill_change(40);
        bimonthly.kind = WishKind::BillChange {
            bill_id: "10".into(),
            ends_on: None,
        };
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, bimonthly))),
            "only weekly, two-weekly, monthly or yearly bills can be changed here"
        );
        let mut free = recurring("Free", Repeat::Monthly);
        free.amount = Decimal::ZERO;
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, free))),
            "wish amount must be positive"
        );
        let mut negative = bill_change(0);
        negative.amount = Decimal::from(-1);
        assert_eq!(
            message(db.write(|w, s| save_wish(w, s, negative))),
            "the new amount cannot be negative"
        );
        db.write(|w, s| save_wish(w, s, bill_change(0))).unwrap();
    }

    #[test]
    fn settings_are_validated() {
        let mut db = db();
        let mut bad = settings("0", "2026-10-01");
        bad.cards[0].limit = Decimal::ZERO;
        assert!(db.write(|w, _| save_wish_settings(w, bad)).is_err());
        assert!(
            db.write(|w, _| save_wish_settings(w, settings("-1", "2026-10-01")))
                .is_err()
        );
        assert!(
            db.write(|w, _| save_wish_settings(w, settings("1", "2026-10-15")))
                .is_err()
        );
        let saved = db
            .write(|w, _| save_wish_settings(w, settings("1", "2026-10-01")))
            .unwrap();
        assert_eq!(load_wish_settings(db.store.connection()).unwrap(), saved);
    }
}
