//! Wishes: planned purchases the owner saves toward, drops or buys.

use finance_types::gateway::{
    BillCreateRequest, BillEditRequest, BillRef, BudgetPlan, CardSettings, Repeat, Wish, WishDraft,
    WishKind, WishPurchase, WishSettings, WishSource, WishStatus,
};
use finance_types::report::Currency;
use finance_types::report::wishes::Cut;
use jiff::civil::Date;
use rust_decimal::Decimal;

use super::bills::{Checks, archive_bill, create_bill, edit_bill};
use super::budget::save_budget_plan;
use super::{Writer, invalid};
use crate::budget::default_budget;
use crate::wishes::baseline::ranking;
use crate::wishes::store::{load_wish_settings, load_wishes};
use crate::{Ctx, Result, Snapshot};

fn find(wishes: &[Wish], id: &str) -> Result<Wish> {
    wishes
        .iter()
        .find(|wish| wish.wish_id == id)
        .cloned()
        .ok_or_else(|| invalid("wish not found"))
}

fn store(w: &Writer<'_>, wish: &Wish) -> Result<()> {
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
                must: false,
                created_at: w.stamp(),
            }
        }
    };
    store(w, &wish)?;
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

/// `settings` with its first card changed. Temporary, like `first()`.
fn with_first_card(
    mut settings: WishSettings,
    change: impl FnOnce(&mut CardSettings),
) -> WishSettings {
    if settings.cards.is_empty() {
        settings = WishSettings::default();
    }
    change(&mut settings.cards[0]);
    settings
}

fn put_settings(w: &Writer<'_>, settings: &WishSettings) -> Result<()> {
    w.put_json(
        "INSERT INTO wish_settings (singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        settings,
        &[],
    )
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
    let bought = Wish {
        status: WishStatus::Bought,
        bought: Some(purchase.clone()),
        ..wish
    };
    store(w, &bought)?;
    if purchase.source == WishSource::Card {
        let month = ctx.resolver.period_for(on, None);
        let settings = load_wish_settings(&w.tx)?;
        let card = settings.first();
        let recorded = card
            .used_month
            .parse::<Date>()
            .ok()
            .map(|d| d.first_of_month());
        let settings = match recorded {
            // Bought before this budget month: already charged.
            _ if month < ctx.current_month() => None,
            Some(m) if m == month => Some(with_first_card(settings, |c| {
                c.used = card.used + purchase.amount;
            })),
            // The recorded month is later: this purchase was charged already.
            Some(m) if m > month => None,
            _ => Some(with_first_card(settings, |c| {
                c.used = purchase.amount;
                c.used_month = month.to_string();
            })),
        };
        if let Some(settings) = settings {
            put_settings(w, &settings)?;
        }
    }
    write_cuts(w, snap, ctx.current_month(), &bought.currency, cuts)?;
    Ok(bought)
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
    let key = month.to_string();
    let mut plan = snap
        .budget_plans
        .iter()
        .find(|p| p.month == key && &p.currency == currency)
        .cloned()
        .unwrap_or_else(|| ranked_default(snap, month, currency));
    for cut in cuts {
        if let Some(group) = plan.groups.iter_mut().find(|g| g.id == cut.group_id) {
            group.amount = (group.amount - (cut.from - cut.to)).max(Decimal::ZERO);
        }
    }
    save_budget_plan(w, snap, plan)?;
    Ok(())
}

/// The default plan for `month`, each group keeping the importance and
/// floor the latest earlier plan gave a group of its name.
fn ranked_default(snap: &Snapshot, month: Date, currency: &Currency) -> BudgetPlan {
    let mut plan = default_budget(month, currency);
    let key = month.to_string();
    let mut earlier: Vec<&BudgetPlan> = snap
        .budget_plans
        .iter()
        .filter(|p| &p.currency == currency && p.month < key)
        .collect();
    earlier.sort_by(|a, b| b.month.cmp(&a.month));
    for group in &mut plan.groups {
        if let Some(ranked) = ranking(earlier.iter().copied(), &group.name) {
            group.importance = ranked.importance;
            group.floor = ranked.floor;
        }
    }
    plan
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

/// Store the card limit and its usage.
pub fn save_wish_settings(w: &Writer<'_>, settings: WishSettings) -> Result<WishSettings> {
    let card = settings.first();
    if card.limit <= Decimal::ZERO
        || card.used < Decimal::ZERO
        || !card.used_month.ends_with("-01")
        || card.used_month.parse::<Date>().is_err()
    {
        return Err(invalid("invalid card settings"));
    }
    put_settings(w, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::fixture::{Db, db};
    use finance_types::gateway::{BudgetGroup, BudgetPlan, Repeat, WishKind};
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
                used: used.parse().unwrap(),
                used_month: month.into(),
                ..WishSettings::default().first()
            }],
        }
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
            load_wish_settings(db.store.connection())
                .unwrap()
                .first()
                .used,
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
            load_wish_settings(db.store.connection())
                .unwrap()
                .first()
                .used,
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
            assert_eq!(now.first().used, Decimal::from(100));
            assert_eq!(now.first().used_month, "2026-10-01");
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
            load_wish_settings(db.store.connection())
                .unwrap()
                .first()
                .used,
            "109.98".parse::<Decimal>().unwrap()
        );
        assert_eq!(
            stored_plan(&db, "2026-11-01").groups[0].amount,
            Decimal::from(100)
        );
    }

    #[test]
    fn cuts_into_a_default_budget_keep_importance() {
        let mut db = db();
        let mut september = going_out("2026-09-01", 120);
        september.groups[0].id = "groceries-old".into();
        september.groups[0].name = "Groceries".into();
        september.groups[0].importance = Some(1);
        september.groups[0].floor = Some(Decimal::from(200));
        db.write(|w, s| save_budget_plan(w, s, september)).unwrap();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        let cut = Cut {
            group_id: "groceries".into(),
            name: "Groceries".into(),
            from: Decimal::from(300),
            to: Decimal::from(250),
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
        let october = stored_plan(&db, "2026-10-01");
        let groceries = october.groups.iter().find(|g| g.id == "groceries").unwrap();
        assert_eq!(groceries.amount, Decimal::from(250));
        assert_eq!(groceries.importance, Some(1));
        assert_eq!(groceries.floor, Some(Decimal::from(200)));
        let food = october.groups.iter().find(|g| g.id == "food-out").unwrap();
        assert_eq!(food.importance, None);
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
