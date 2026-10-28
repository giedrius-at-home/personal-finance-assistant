//! Wishes: planned purchases the owner saves toward, drops or buys.

use finance_types::gateway::{Wish, WishDraft, WishPurchase, WishSettings, WishSource, WishStatus};
use finance_types::report::wishes::Cut;
use jiff::civil::Date;
use rust_decimal::Decimal;

use super::budget::save_budget_plan;
use super::{Writer, invalid};
use crate::budget::default_budget;
use crate::wishes::store::{load_wish_settings, load_wishes};
use crate::{Result, Snapshot};

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

/// Create a wish, or edit a planned one (keeping its progress).
pub fn save_wish(w: &Writer<'_>, snap: &Snapshot, draft: WishDraft) -> Result<Wish> {
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(invalid("wish needs a name"));
    }
    if draft.amount <= Decimal::ZERO {
        return Err(invalid("wish amount must be positive"));
    }
    if draft.due_on.parse::<Date>().is_err() {
        return Err(invalid("wish date is invalid"));
    }
    if let WishSource::Account { account_id } = &draft.source
        && !snap.accounts.iter().any(|a| &a.account_id == account_id)
    {
        return Err(invalid("wish account is unknown"));
    }
    let wishes = load_wishes(&w.tx)?;
    if let Some(parent_id) = &draft.parent_id {
        let parent =
            planned(find(&wishes, parent_id).map_err(|_| invalid("parent wish is not planned"))?)
                .map_err(|_| invalid("parent wish is not planned"))?;
        if parent.parent_id.is_some() {
            return Err(invalid("sub-wishes cannot have sub-wishes"));
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
                status: WishStatus::Planned,
                bought: None,
                kind: draft.kind,
                created_at: w.stamp(),
            }
        }
    };
    store(w, &wish)?;
    Ok(wish)
}

/// Set the total saved so far toward a planned wish.
pub fn record_saved(w: &Writer<'_>, wish_id: &str, saved: Decimal) -> Result<Wish> {
    if saved < Decimal::ZERO {
        return Err(invalid("saved amount cannot be negative"));
    }
    let wish = planned(find(&load_wishes(&w.tx)?, wish_id)?)?;
    let wish = Wish { saved, ..wish };
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

/// Buy a planned wish; a card purchase in the tracked month raises the card
/// usage, and `cuts` trim the purchase month's budget.
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
        let settings = load_wish_settings(&w.tx)?;
        if settings.card_used_month.get(..7) == Some(&purchase.on[..7]) {
            put_settings(
                w,
                &WishSettings {
                    card_used: settings.card_used + purchase.amount,
                    ..settings
                },
            )?;
        }
    }
    if !cuts.is_empty() {
        let month = on.first_of_month();
        let key = month.to_string();
        let mut plan = snap
            .budget_plans
            .iter()
            .find(|p| p.month == key && p.currency == bought.currency)
            .cloned()
            .unwrap_or_else(|| default_budget(month, &bought.currency));
        for cut in cuts {
            if let Some(group) = plan.groups.iter_mut().find(|g| g.id == cut.group_id) {
                group.amount = cut.to;
            }
        }
        save_budget_plan(w, snap, plan)?;
    }
    Ok(bought)
}

/// Store the card limit and its usage.
pub fn save_wish_settings(w: &Writer<'_>, settings: WishSettings) -> Result<WishSettings> {
    if settings.card_limit <= Decimal::ZERO
        || settings.card_used < Decimal::ZERO
        || !settings.card_used_month.ends_with("-01")
        || settings.card_used_month.parse::<Date>().is_err()
    {
        return Err(invalid("invalid card settings"));
    }
    put_settings(w, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::fixture::db;
    use finance_types::gateway::{BudgetGroup, BudgetPlan};
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
        db.write(|w, _| record_saved(w, &wish.wish_id, Decimal::from(40)))
            .unwrap();
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
            db.write(|w, _| record_saved(w, &wish.wish_id, Decimal::from(-1)))
                .is_err()
        );
        assert!(
            db.write(|w, _| record_saved(w, "wish-nope", Decimal::ONE))
                .is_err()
        );
        db.write(|w, _| drop_wish(w, &wish.wish_id)).unwrap();
        assert_eq!(
            message(db.write(|w, _| record_saved(w, &wish.wish_id, Decimal::ONE))),
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
        }
    }

    fn settings(used: &str, month: &str) -> WishSettings {
        WishSettings {
            card_limit: Decimal::from(2500),
            card_used: used.parse().unwrap(),
            card_used_month: month.into(),
            card_bill_id: None,
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
            load_wish_settings(db.store.connection()).unwrap().card_used,
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
    fn mark_bought_outside_card_month_leaves_usage() {
        let mut db = db();
        let wish = db.write(|w, s| save_wish(w, s, draft("Bike"))).unwrap();
        db.write(|w, _| save_wish_settings(w, settings("9.98", "2026-10-01")))
            .unwrap();
        db.write(|w, s| {
            mark_bought(
                w,
                s,
                &wish.wish_id,
                purchase(100, "2026-11-02", WishSource::Card),
                &[],
            )
        })
        .unwrap();
        assert_eq!(
            load_wish_settings(db.store.connection()).unwrap().card_used,
            "9.98".parse::<Decimal>().unwrap()
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

    #[test]
    fn settings_are_validated() {
        let mut db = db();
        let mut bad = settings("0", "2026-10-01");
        bad.card_limit = Decimal::ZERO;
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
