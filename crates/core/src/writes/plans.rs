//! One-off planned payments and expected income (the site's
//! `/api/finance/plans` route → Python `/planned-payments/create` and
//! `/planned-payments/archive`, `SQLiteDerivedState.add_planned_payment` /
//! `archive_planned_payment`).

use std::str::FromStr;

use finance_types::gateway::{Direction, PlannedPayment, PlannedPaymentRequest};
use finance_types::report::UnitMoney;
use finance_types::report::plans::PlannedPaymentView;
use jiff::civil::Date;
use rusqlite::params;
use rust_decimal::Decimal;

use super::{Writer, invalid, ledger_id};
use crate::model::{Account, Kind, Tx};
use crate::{Result, Snapshot};

/// The route's schema check failed (the site's 400 text).
const MALFORMED: &str = "Invalid planned payment.";
/// The service refused the values (the site's 422 text).
const REFUSED: &str = "Check the amount, date and currency.";

/// `^\d+(\.\d{1,6})?$`.
fn amount_format(text: &str) -> bool {
    let (whole, fraction) = match text.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (text, None),
    };
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction
            .is_none_or(|f| (1..=6).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()))
}

fn length_between(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.chars().count())
}

/// Python `date.fromisoformat` on the route's `z.iso.date()` value.
fn iso_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    (bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-')
        .then(|| text.parse::<Date>().ok())
        .flatten()
}

/// Validate `request` the way the route and the service did and build the
/// stored plan; `plan_id` and `created_at` are supplied by the caller.
pub fn build_planned_payment(
    request: &PlannedPaymentRequest,
    accounts: &[Account],
    transactions: &[Tx],
    plans: &[PlannedPayment],
    plan_id: String,
    created_at: String,
) -> Result<PlannedPayment> {
    let name = request.name.trim();
    let match_category = request.match_category.trim();
    let reimbursement_for = request
        .reimbursement_for
        .as_deref()
        .filter(|r| !r.is_empty());
    let match_description = request
        .match_description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let shape_ok = length_between(name, 1, 128)
        && length_between(match_category, 1, 128)
        && amount_format(&request.amount)
        && request.currency.len() == 3
        && request.currency.bytes().all(|b| b.is_ascii_uppercase())
        && iso_date(&request.due_on).is_some()
        && length_between(&request.account_id, 1, 128)
        && reimbursement_for.is_none_or(|r| r.chars().count() <= 128)
        && match_description.is_none_or(|d| length_between(d, 1, 128));
    if !shape_ok {
        return Err(invalid(MALFORMED));
    }
    let currency = accounts
        .iter()
        .map(Account::currency)
        .find(|c| c.code == request.currency)
        .cloned()
        .ok_or_else(|| invalid(REFUSED))?;
    if !accounts.iter().any(|a| a.account_id == request.account_id) {
        return Err(invalid(REFUSED));
    }
    let amount = Decimal::from_str(&request.amount).map_err(|_| invalid(REFUSED))?;
    // Money: no more fractional digits than the currency has.
    if amount.normalize().scale() > u32::from(currency.minor_unit) || amount <= Decimal::ZERO {
        return Err(invalid(REFUSED));
    }
    if let Some(original_id) = reimbursement_for {
        let original = transactions
            .iter()
            .find(|t| t.transaction_id == original_id);
        let allocated: Decimal = plans
            .iter()
            .filter(|p| p.reimbursement_for.as_deref() == Some(original_id))
            .map(|p| p.amount.amount)
            .sum();
        let valid = request.direction == Direction::Inflow
            && original.is_some_and(|t| {
                t.kind == Kind::Outflow && t.currency == currency && allocated + amount <= t.amount
            });
        if !valid {
            return Err(invalid(REFUSED));
        }
    }
    Ok(PlannedPayment {
        plan_id,
        name: name.to_owned(),
        match_category: match_category.to_owned(),
        direction: request.direction,
        amount: UnitMoney { amount, currency },
        due_on: request.due_on.clone(),
        created_at,
        reimbursement_for: reimbursement_for.map(str::to_owned),
        pending: false,
        account_id: Some(request.account_id.clone()),
        match_description: match_description.map(str::to_owned),
        paid_by: None,
    })
}

/// Store a new planned payment and return it.
pub fn create_planned_payment(
    w: &Writer<'_>,
    snap: &Snapshot,
    request: &PlannedPaymentRequest,
) -> Result<PlannedPayment> {
    // uuid4().hex: 32 random hex digits.
    let plan_id: String =
        w.tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
    let plan = build_planned_payment(
        request,
        &snap.accounts,
        &snap.raw_transactions,
        &snap.planned_payments,
        plan_id,
        w.stamp(),
    )?;
    w.put_json(
        "INSERT INTO planned_payments (plan_id, payload_json) VALUES (?2, ?1)",
        &plan,
        &[&plan.plan_id],
    )?;
    Ok(plan)
}

/// Archive an active planned payment; its matched transactions are kept.
pub fn archive_planned_payment(w: &Writer<'_>, plan_id: &str) -> Result<()> {
    if !length_between(plan_id, 1, 64) {
        return Err(invalid(MALFORMED));
    }
    let stamp = super::transactions::iso_now(w);
    let changed = w.tx.execute(
        "UPDATE planned_payments SET archived_at = ?1 WHERE plan_id = ?2 AND archived_at IS NULL",
        params![stamp, plan_id],
    )?;
    if changed != 1 {
        return Err(invalid("This entry was already removed. Refresh."));
    }
    Ok(())
}

/// The payment `id` names, if it may pay `plan` by hand: it runs the
/// plan's way in the plan's currency and pays no bill and no plan in
/// `views` (the reconciled plans) but this one.
pub(crate) fn payment_to_tie(
    plan: &PlannedPayment,
    id: &str,
    transactions: &[Tx],
    views: &[PlannedPaymentView],
) -> Result<String> {
    let id = ledger_id(transactions, id);
    let paid = transactions
        .iter()
        .find(|t| t.transaction_id == id)
        .ok_or_else(|| invalid("Payment not found. Refresh."))?;
    let right_way = match plan.direction {
        Direction::Outflow => paid.kind == Kind::Outflow,
        Direction::Inflow => matches!(paid.kind, Kind::Inflow | Kind::Refund),
    };
    if !right_way || paid.currency != plan.amount.currency {
        return Err(invalid(match plan.direction {
            Direction::Outflow => "Pick a payment in the plan's currency.",
            Direction::Inflow => "Pick money received in the plan's currency.",
        }));
    }
    if paid.subscription_id.is_some() {
        return Err(invalid("That payment already pays a bill."));
    }
    // Whether tied by hand or matched: it is the other plan's.
    if let Some(other) = views.iter().find(|v| {
        v.plan_id != plan.plan_id && v.transaction_id.as_deref() == Some(&paid.transaction_id)
    }) {
        return Err(invalid(format!(
            "That payment already pays {}.",
            other.name
        )));
    }
    Ok(paid.transaction_id.clone())
}

/// Tie a payment to a plan by hand (it then paid the plan, whatever its
/// category, amount or date), or with `None` leave the plan to matching
/// again.
pub fn tie_planned_payment(
    w: &Writer<'_>,
    snap: &Snapshot,
    plan_id: &str,
    transaction_id: Option<&str>,
) -> Result<PlannedPayment> {
    let mut plan = snap
        .planned_payments
        .iter()
        .find(|p| p.plan_id == plan_id)
        .cloned()
        .ok_or_else(|| invalid("This entry was already removed. Refresh."))?;
    plan.paid_by = match transaction_id {
        None => None,
        Some(id) => {
            let views = crate::plans::planned_payment_views(&crate::Ctx::new(snap, &w.now));
            Some(payment_to_tie(&plan, id, &snap.transactions, &views)?)
        }
    };
    let json = serde_json::to_string(&plan).map_err(|e| crate::Error::Invalid(e.to_string()))?;
    // The snapshot can be older than the table: a plan archived since is gone.
    let changed = w.tx.execute(
        "UPDATE planned_payments SET payload_json = ?1 WHERE plan_id = ?2 AND archived_at IS NULL",
        rusqlite::params![json, plan.plan_id],
    )?;
    if changed != 1 {
        return Err(invalid("This entry was already removed. Refresh."));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::Currency;
    use jiff::Zoned;
    use rusqlite::Connection;

    fn account(id: &str) -> Account {
        Account {
            account_id: id.into(),
            ledger_id: 1,
            name: "Daily".into(),
            balance: UnitMoney {
                amount: Decimal::from(100),
                currency: Currency::eur(),
            },
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    fn purchase(id: &str, amount: &str, kind: Kind) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 7,
            account_id: "firefly:1".into(),
            counterparty_account_id: None,
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: now(),
            purchased_on: None,
            description: Some("Bike".into()),
            counterparty_name: None,
            group_id: None,
            category: Some("Bike".into()),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn now() -> Zoned {
        "2026-10-26T20:00:00+00:00[UTC]".parse().unwrap()
    }

    fn request() -> PlannedPaymentRequest {
        PlannedPaymentRequest {
            name: "  Bicycle repair ".into(),
            match_category: " Bike ".into(),
            direction: Direction::Outflow,
            amount: "45.50".into(),
            currency: "EUR".into(),
            due_on: "2026-10-30".into(),
            account_id: "firefly:1".into(),
            reimbursement_for: Some(String::new()),
            match_description: None,
        }
    }

    fn build(
        r: &PlannedPaymentRequest,
        txs: &[Tx],
        plans: &[PlannedPayment],
    ) -> Result<PlannedPayment> {
        build_planned_payment(
            r,
            &[account("firefly:1")],
            txs,
            plans,
            "abc".into(),
            "2026-10-26T20:00:00.000000Z".into(),
        )
    }

    fn message(result: Result<PlannedPayment>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn builds_a_trimmed_plan() {
        let plan = build(&request(), &[], &[]).unwrap();
        assert_eq!(plan.name, "Bicycle repair");
        assert_eq!(plan.match_category, "Bike");
        assert_eq!(plan.amount.amount.to_string(), "45.50");
        assert_eq!(plan.account_id.as_deref(), Some("firefly:1"));
        assert_eq!(plan.reimbursement_for, None);
        assert!(!plan.pending);
    }

    #[test]
    fn rejects_malformed_payloads() {
        for change in [
            |r: &mut PlannedPaymentRequest| r.name = "  ".into(),
            |r: &mut PlannedPaymentRequest| r.amount = "1e3".into(),
            |r: &mut PlannedPaymentRequest| r.amount = "-5".into(),
            |r: &mut PlannedPaymentRequest| r.amount = "5.".into(),
            |r: &mut PlannedPaymentRequest| r.currency = "eur".into(),
            |r: &mut PlannedPaymentRequest| r.due_on = "2026-10-3".into(),
            |r: &mut PlannedPaymentRequest| r.due_on = "2026-02-30".into(),
            |r: &mut PlannedPaymentRequest| r.account_id = String::new(),
        ] {
            let mut r = request();
            change(&mut r);
            assert_eq!(message(build(&r, &[], &[])), MALFORMED, "{r:?}");
        }
    }

    #[test]
    fn rejects_values_the_service_refused() {
        for change in [
            |r: &mut PlannedPaymentRequest| r.amount = "0".into(),
            |r: &mut PlannedPaymentRequest| r.amount = "1.005".into(),
            |r: &mut PlannedPaymentRequest| r.currency = "USD".into(),
            |r: &mut PlannedPaymentRequest| r.account_id = "firefly:9".into(),
        ] {
            let mut r = request();
            change(&mut r);
            assert_eq!(message(build(&r, &[], &[])), REFUSED, "{r:?}");
        }
        // Trailing zeros beyond the minor unit are fine, as with Python's Money.
        let mut r = request();
        r.amount = "1.500000".into();
        assert!(build(&r, &[], &[]).is_ok());
    }

    #[test]
    fn checks_reimbursements() {
        let bike = purchase("firefly:7", "100.00", Kind::Outflow);
        let mut r = request();
        r.reimbursement_for = Some("firefly:7".into());
        // Only expected income can reimburse.
        assert_eq!(
            message(build(&r, std::slice::from_ref(&bike), &[])),
            REFUSED
        );
        r.direction = Direction::Inflow;
        r.amount = "60".into();
        let first = build(&r, std::slice::from_ref(&bike), &[]).unwrap();
        assert_eq!(first.reimbursement_for.as_deref(), Some("firefly:7"));
        // Never more than the purchase in total.
        assert_eq!(
            message(build(
                &r,
                std::slice::from_ref(&bike),
                std::slice::from_ref(&first)
            )),
            REFUSED
        );
        r.amount = "40".into();
        assert!(build(&r, std::slice::from_ref(&bike), &[first]).is_ok());
        // Unknown purchase, or one that is not an outflow.
        assert_eq!(message(build(&r, &[], &[])), REFUSED);
        let income = purchase("firefly:7", "100.00", Kind::Inflow);
        assert_eq!(message(build(&r, &[income], &[])), REFUSED);
    }

    fn table() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE planned_payments (plan_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL, archived_at TEXT)",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_payment_is_tied_only_when_it_is_free_and_runs_the_plans_way() {
        let plan = build(&request(), &[], &[]).unwrap();
        let bike = purchase("firefly:7", "61.20", Kind::Outflow);
        let tie = |txs: &[Tx], views: &[PlannedPaymentView]| {
            payment_to_tie(&plan, "7", txs, views).map_err(|e| e.to_string())
        };
        // Any amount and category, named by its bare ledger id.
        assert_eq!(tie(std::slice::from_ref(&bike), &[]).unwrap(), "firefly:7");
        assert!(tie(&[], &[]).is_err());
        // Money received does not pay a payment to make.
        assert!(tie(&[purchase("firefly:7", "61.20", Kind::Inflow)], &[]).is_err());
        let mut billed = bike.clone();
        billed.subscription_id = Some("bill".into());
        assert_eq!(
            tie(&[billed], &[]).unwrap_err(),
            "That payment already pays a bill."
        );
        // Another plan has it, matched or tied.
        let other = PlannedPayment {
            plan_id: "other".into(),
            name: "Chair".into(),
            paid_by: Some("firefly:7".into()),
            ..plan.clone()
        };
        let views = crate::plans::reconcile_planned_payments(
            std::slice::from_ref(&other),
            &[],
            std::slice::from_ref(&bike),
            &now(),
            &std::collections::HashMap::new(),
        );
        assert_eq!(
            tie(std::slice::from_ref(&bike), &views).unwrap_err(),
            "That payment already pays Chair."
        );
    }

    #[test]
    fn stores_and_archives() {
        let mut conn = table();
        let plan = build(&request(), &[], &[]).unwrap();
        {
            let w = Writer {
                tx: conn.transaction().unwrap(),
                now: now(),
            };
            w.put_json(
                "INSERT INTO planned_payments (plan_id, payload_json) VALUES (?2, ?1)",
                &plan,
                &[&plan.plan_id],
            )
            .unwrap();
            archive_planned_payment(&w, "abc").unwrap();
            assert_eq!(
                archive_planned_payment(&w, "abc").unwrap_err().to_string(),
                "This entry was already removed. Refresh."
            );
            w.tx.commit().unwrap();
        }
        let (payload, archived): (String, String) = conn
            .query_row(
                "SELECT payload_json, archived_at FROM planned_payments",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(archived, "2026-10-26T20:00:00.000000+00:00");
        let stored: PlannedPayment = serde_json::from_str(&payload).unwrap();
        assert_eq!(stored, plan);
        assert!(payload.contains("\"amount\":{\"amount\":\"45.50\""));
    }
}
