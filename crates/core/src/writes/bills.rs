//! Bills, the owner's recurring plans (Python `web.py` `/subscriptions/*`).
//!
//! Python wrote bills and bill links to Firefly III and kept the category
//! rules and declined matches in its own tables. The ledger now holds the
//! bills (`bills`, `transactions.bill_id`); the rules and declines keep their
//! gateway tables and payload format. Messages are the ones the web UI
//! showed (the Next routes reworded the service's errors).

use std::collections::{BTreeMap, HashSet};

use finance_types::gateway::{
    BILL_FREQUENCIES, BillCreateRequest, BillEditRequest, BillMatchRequest, BillRef, BillSaved,
    SubscriptionCategoriesRequest, SubscriptionCategoryRule,
};
use finance_types::report::subscriptions::{
    BillLinkCandidate, SubscriptionReport, SubscriptionRow,
};
use jiff::Zoned;
use jiff::civil::Date;
use rusqlite::{OptionalExtension, params};
use rust_decimal::Decimal;

use super::{Writer, invalid};
use crate::model::{Kind, Tx};
use crate::subscriptions::{BillWindow, budget_calendar_offset, subscription_report};
use crate::text::casefold;
use crate::{Ctx, Error, Result, Snapshot};

/// A bill changed under the owner, or its category belongs to another bill.
pub const CHANGED: &str =
    "This plan changed or its category is already assigned. Refresh before saving.";
pub const INVALID: &str = "Invalid bill settings.";
pub const UNKNOWN_CATEGORY: &str = "Choose a category that already exists.";
pub const DUPLICATE_NAME: &str = "A bill with this name already exists.";
pub const CATEGORIES_CHANGED: &str =
    "These subscription categories changed. Refresh before saving.";
pub const CATEGORIES_INVALID: &str = "Choose categories that already exist.";
pub const MATCH_CHANGED: &str = "This match changed. Refresh reports before linking.";

/// What the bill writes validate against, read from the snapshot.
#[derive(Debug)]
pub struct Checks<'a> {
    /// Currency codes of the owner's accounts.
    pub currencies: HashSet<String>,
    /// Case-folded categories of every transaction.
    pub categories: HashSet<String>,
    /// Case-folded → stored spelling of every spending category.
    pub expense_categories: BTreeMap<String, String>,
    /// This month's bill report first, then the following months the
    /// screens can show.
    pub views: Vec<SubscriptionReport>,
    /// Transactions as imported (before shared-spending rewrites).
    pub transactions: &'a [Tx],
}

impl<'a> Checks<'a> {
    pub fn new(snap: &'a Snapshot, now: &Zoned) -> Result<Self> {
        let ctx = Ctx::new(snap, now);
        // Python checked calendar months 0 and 1; the screen shows the budget
        // month and the next, which an early cutoff moves one month on.
        let last = budget_calendar_offset(&ctx) + 1;
        let views = (0..=last)
            .map(|offset| subscription_report(&ctx, BillWindow::CalendarMonth(offset)))
            .collect::<Result<_>>()?;
        Ok(Self {
            currencies: snap
                .accounts
                .iter()
                .map(|a| a.currency().code.clone())
                .collect(),
            categories: snap
                .transactions
                .iter()
                .filter_map(|t| t.category.as_deref())
                .map(casefold)
                .collect(),
            expense_categories: crate::groups::expense_categories(&snap.transactions)
                .into_iter()
                .map(|c| (casefold(&c), c))
                .collect(),
            views,
            transactions: &snap.raw_transactions,
        })
    }

    fn stale(&self) -> bool {
        self.views.first().is_none_or(|v| v.stale)
    }

    fn row(&self, bill_id: &str) -> Option<&SubscriptionRow> {
        self.views
            .iter()
            .flat_map(|v| &v.rows)
            .find(|r| r.bill_id == bill_id)
    }

    /// The bill and the suggested payment, while both are still offered.
    fn candidate(
        &self,
        request: &BillMatchRequest,
    ) -> Option<(&SubscriptionRow, &BillLinkCandidate)> {
        if self.stale() {
            return None;
        }
        self.views.iter().flat_map(|v| &v.rows).find_map(|row| {
            (row.bill_id == request.bill_id)
                .then(|| {
                    row.link_candidates
                        .iter()
                        .find(|c| c.transaction_id == request.transaction_id)
                        .map(|c| (row, c))
                })
                .flatten()
        })
    }
}

/// `name` trimmed, when it has 1 to `max` characters.
fn text(value: &str, max: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= max).then(|| value.to_owned())
}

/// Firefly stored amounts with 12 decimals; the forms send at most 6.
fn stored_amount(value: Decimal) -> String {
    format!("{value:.12}")
}

fn valid_amount(value: Decimal) -> bool {
    !value.is_sign_negative() && value.scale() <= 6
}

fn valid_frequency(value: &str) -> bool {
    BILL_FREQUENCIES.contains(&value)
}

fn parse_date(value: &str) -> Option<Date> {
    if value.len() != 10 {
        return None;
    }
    value.parse().ok()
}

fn bill_id(value: &str) -> Option<i64> {
    value.parse().ok().filter(|id| *id > 0)
}

/// A bill with this (case-folded) name other than `except`.
fn name_taken(w: &Writer<'_>, name: &str, except: Option<i64>) -> Result<bool> {
    let key = casefold(name.trim());
    let mut statement = w.tx.prepare("SELECT id, name FROM bills")?;
    let names = statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in names {
        let (id, other) = row?;
        if Some(id) != except && casefold(other.trim()) == key {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rules(w: &Writer<'_>) -> Result<Vec<SubscriptionCategoryRule>> {
    let mut statement =
        w.tx.prepare("SELECT payload_json FROM subscription_category_rules ORDER BY bill_id")?;
    let payloads = statement.query_map([], |r| r.get::<_, String>(0))?;
    payloads
        .map(|p| serde_json::from_str(&p?).map_err(|e| Error::Invalid(e.to_string())))
        .collect()
}

/// Python `save_subscription_category_rule`: store `categories` for the bill
/// if its rule is still at `expected_revision` and no other bill claims one
/// of them; `conflict` is the message otherwise.
fn save_rule(
    w: &Writer<'_>,
    bill_id: &str,
    categories: Vec<String>,
    expected_revision: u32,
    conflict: &str,
) -> Result<SubscriptionCategoryRule> {
    let saved = SubscriptionCategoryRule {
        bill_id: bill_id.to_owned(),
        categories,
        revision: expected_revision + 1,
        updated_at: w.stamp(),
    };
    let requested: HashSet<String> = saved.categories.iter().map(|c| casefold(c)).collect();
    let claimed = rules(w)?.into_iter().any(|rule| {
        rule.bill_id != bill_id
            && rule
                .categories
                .iter()
                .any(|c| requested.contains(&casefold(c)))
    });
    if claimed {
        return Err(invalid(conflict));
    }
    let json = serde_json::to_string(&saved).map_err(|e| Error::Invalid(e.to_string()))?;
    let changed = w.tx.execute(
        "INSERT INTO subscription_category_rules (bill_id, revision, updated_at, payload_json) \
         SELECT ?1, ?2, ?3, ?4 WHERE COALESCE((SELECT revision FROM subscription_category_rules \
         WHERE bill_id = ?1), 0) = ?5 \
         ON CONFLICT(bill_id) DO UPDATE SET revision = excluded.revision, \
         updated_at = excluded.updated_at, payload_json = excluded.payload_json",
        params![
            saved.bill_id,
            saved.revision,
            super::transactions::iso_now(w),
            json,
            expected_revision
        ],
    )?;
    if changed != 1 {
        return Err(invalid(conflict));
    }
    Ok(saved)
}

/// Python wrote timestamp columns with `isoformat()` (`+00:00`).
fn record(w: &Writer<'_>, action: &str, bill_id: &str) -> Result<()> {
    w.record(
        "bills",
        &serde_json::json!({ "action": action, "bill_id": bill_id }),
    )
}

/// Create a recurring plan and assign it its matching category.
pub fn create_bill(
    w: &Writer<'_>,
    checks: &Checks<'_>,
    request: BillCreateRequest,
) -> Result<BillSaved> {
    let (Some(name), Some(category)) =
        (text(&request.name, 128), text(&request.match_category, 128))
    else {
        return Err(invalid(INVALID));
    };
    let (minimum, maximum) = (request.amount_min, request.amount_max);
    let date = parse_date(&request.date);
    if !valid_amount(minimum)
        || !valid_amount(maximum)
        || minimum <= Decimal::ZERO
        || maximum < minimum
        || !valid_frequency(&request.repeat_freq)
        || request.skip > 1
        || !checks.currencies.contains(&request.currency_code)
    {
        return Err(invalid(INVALID));
    }
    let Some(date) = date else {
        return Err(invalid(INVALID));
    };
    let end = match request.end_date.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(value) => match parse_date(value) {
            Some(end) if end >= date => Some(end),
            _ => return Err(invalid(INVALID)),
        },
    };
    let key = casefold(&category);
    if rules(w)?
        .iter()
        .any(|rule| rule.categories.iter().any(|c| casefold(c) == key))
    {
        return Err(invalid(CHANGED));
    }
    if !checks.categories.contains(&key) {
        return Err(invalid(UNKNOWN_CATEGORY));
    }
    if name_taken(w, &name, None)? {
        return Err(invalid(DUPLICATE_NAME));
    }
    // A fresh id, also unused by any rule or decline left from an old bill.
    let id: i64 = w.tx.query_row(
        "SELECT 1 + MAX(COALESCE((SELECT MAX(id) FROM bills), 0), \
         COALESCE((SELECT MAX(CAST(bill_id AS INTEGER)) FROM subscription_category_rules), 0), \
         COALESCE((SELECT MAX(CAST(bill_id AS INTEGER)) FROM declined_bill_matches), 0))",
        [],
        |r| r.get(0),
    )?;
    w.tx.execute(
        "INSERT INTO bills (id, name, currency, amount_min, amount_max, start_date, end_date, \
         repeat_freq, skip, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)",
        params![
            id,
            name,
            request.currency_code,
            stored_amount(minimum),
            stored_amount(maximum),
            date.to_string(),
            end.map(|e| e.to_string()),
            request.repeat_freq,
            request.skip
        ],
    )?;
    let bill_id = id.to_string();
    save_rule(w, &bill_id, vec![category], 0, CHANGED)?;
    record(w, "create", &bill_id)?;
    Ok(BillSaved { bill_id })
}

/// Change a bill's name, amounts, frequency and (optionally) schedule start.
pub fn edit_bill(w: &Writer<'_>, checks: &Checks<'_>, request: BillEditRequest) -> Result<()> {
    if checks.stale() || checks.row(&request.bill_id).is_none() {
        return Err(invalid(CHANGED));
    }
    let Some(id) = bill_id(&request.bill_id) else {
        return Err(invalid(CHANGED));
    };
    let Some(name) = text(&request.name, 128) else {
        return Err(invalid(INVALID));
    };
    let (minimum, maximum) = (request.amount_min, request.amount_max);
    if !valid_amount(minimum)
        || !valid_amount(maximum)
        || maximum < minimum
        || !valid_frequency(&request.repeat_freq)
    {
        return Err(invalid(INVALID));
    }
    let start = match request.date.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(value) => Some(parse_date(value).ok_or_else(|| invalid(INVALID))?),
    };
    if name_taken(w, &name, Some(id))? {
        return Err(invalid(DUPLICATE_NAME));
    }
    let changed = w.tx.execute(
        "UPDATE bills SET name = ?2, amount_min = ?3, amount_max = ?4, repeat_freq = ?5, \
         start_date = COALESCE(?6, start_date) WHERE id = ?1",
        params![
            id,
            name,
            stored_amount(minimum),
            stored_amount(maximum),
            request.repeat_freq,
            start.map(|d| d.to_string())
        ],
    )?;
    if changed != 1 {
        return Err(invalid(CHANGED));
    }
    record(w, "edit", &request.bill_id)
}

/// Stop a bill's future payments; its linked payments stay linked and its
/// categories become free for other bills.
pub fn archive_bill(w: &Writer<'_>, request: BillRef) -> Result<()> {
    let Some(id) = bill_id(&request.bill_id) else {
        return Err(invalid(CHANGED));
    };
    if w.tx
        .execute("UPDATE bills SET active = 0 WHERE id = ?1", [id])?
        != 1
    {
        return Err(invalid(CHANGED));
    }
    let revision: Option<u32> =
        w.tx.query_row(
            "SELECT revision FROM subscription_category_rules WHERE bill_id = ?1",
            [&request.bill_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(revision) = revision {
        save_rule(w, &request.bill_id, Vec::new(), revision, CHANGED)?;
    }
    record(w, "archive", &request.bill_id)
}

/// Set which spending categories count as the bill's payments.
pub fn save_subscription_categories(
    w: &Writer<'_>,
    checks: &Checks<'_>,
    request: SubscriptionCategoriesRequest,
) -> Result<SubscriptionCategoryRule> {
    if request.categories.len() > 32 {
        return Err(invalid(CATEGORIES_INVALID));
    }
    if checks.row(&request.bill_id).is_none() {
        return Err(invalid(CATEGORIES_CHANGED));
    }
    let mut seen = HashSet::new();
    let mut categories = Vec::new();
    for value in &request.categories {
        let key = casefold(value.trim());
        let stored = checks.expense_categories.get(&key);
        match stored {
            Some(stored) if value.trim().chars().count() <= 128 && seen.insert(key.clone()) => {
                categories.push(stored.clone());
            }
            _ => return Err(invalid(CATEGORIES_INVALID)),
        }
    }
    save_rule(
        w,
        &request.bill_id,
        categories,
        request.expected_revision,
        CATEGORIES_CHANGED,
    )
}

/// Remember that a suggested payment is not the bill's.
pub fn decline_bill_match(
    w: &Writer<'_>,
    checks: &Checks<'_>,
    request: BillMatchRequest,
) -> Result<()> {
    if checks.candidate(&request).is_none() {
        return Err(invalid(MATCH_CHANGED));
    }
    w.tx.execute(
        "INSERT OR REPLACE INTO declined_bill_matches (bill_id, transaction_id, updated_at) \
         VALUES (?1, ?2, ?3)",
        params![
            request.bill_id,
            request.transaction_id,
            super::transactions::iso_now(w)
        ],
    )?;
    Ok(())
}

/// Link a suggested payment to the bill, if the transaction is unchanged.
pub fn link_bill_payment(
    w: &Writer<'_>,
    checks: &Checks<'_>,
    request: BillMatchRequest,
) -> Result<()> {
    let Some((row, candidate)) = checks.candidate(&request) else {
        return Err(invalid(MATCH_CHANGED));
    };
    let (Some(bill), Ok(ledger_id)) =
        (bill_id(&row.bill_id), request.transaction_id.parse::<i64>())
    else {
        return Err(invalid(MATCH_CHANGED));
    };
    let unchanged = checks.transactions.iter().any(|t| {
        t.ledger_id == ledger_id
            && t.kind == Kind::Outflow
            && t.subscription_id.is_none()
            && t.amount == candidate.amount
            && t.currency == row.currency
            && t.description.as_deref() == Some(candidate.description.as_str())
    });
    if !unchanged {
        return Err(invalid(MATCH_CHANGED));
    }
    let changed = w.tx.execute(
        "UPDATE transactions SET bill_id = ?1 WHERE id = ?2 AND bill_id IS NULL",
        params![bill, ledger_id],
    )?;
    if changed != 1 {
        return Err(invalid(MATCH_CHANGED));
    }
    record(w, "link", &row.bill_id)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use finance_storage::Store;
    use finance_types::report::Currency;

    use super::*;
    use crate::reporting::tests::tx;

    const AT: &str = "2026-10-20T12:00:00+02:00[Europe/Amsterdam]";

    fn store() -> Store {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store
            .connection()
            .execute_batch(
                "CREATE TABLE subscription_category_rules (bill_id TEXT PRIMARY KEY, \
                 revision INTEGER NOT NULL CHECK(revision > 0), updated_at TEXT NOT NULL, \
                 payload_json TEXT NOT NULL);
                 CREATE TABLE declined_bill_matches (bill_id TEXT NOT NULL, \
                 transaction_id TEXT NOT NULL, updated_at TEXT NOT NULL, \
                 PRIMARY KEY (bill_id, transaction_id));
                 INSERT INTO accounts (id, name, kind) VALUES (1, 'Checking', 'asset'), (2, 'Shop', 'expense');
                 INSERT INTO bills (id, name, currency, amount_min, amount_max, start_date, repeat_freq)
                 VALUES (7, 'Gym', 'EUR', '30.000000000000', '30.000000000000', '2026-09-03', 'monthly');
                 INSERT INTO transactions (id, kind, date, booked_at, description, source_account_id, destination_account_id)
                 VALUES (42, 'withdrawal', '2026-10-03', '2026-10-03T09:00:00Z', 'GYM CLUB', 1, 2);",
            )
            .unwrap();
        store
    }

    fn row(bill_id: &str, revision: u32, candidates: Vec<BillLinkCandidate>) -> SubscriptionRow {
        SubscriptionRow {
            bill_id: bill_id.into(),
            name: "Gym".into(),
            currency: Currency::new("EUR"),
            amount_min: Decimal::from(30),
            amount_max: Decimal::from(30),
            frequency: "monthly".into(),
            expected_dates: vec![],
            last_matched_on: None,
            native_payment_count: 0,
            category_payment_count: 0,
            recorded_spending: Decimal::ZERO,
            last_recorded_on: None,
            category_name_match: false,
            category_match_categories: vec![],
            category_match_configured: revision > 0,
            category_match_revision: revision,
            link_candidates: candidates,
        }
    }

    fn view(rows: Vec<SubscriptionRow>, stale: bool) -> SubscriptionReport {
        SubscriptionReport {
            fetched_at: None,
            through_date: "2026-10-31".into(),
            stale,
            rows,
            expected_costs: vec![],
            remaining_costs: vec![],
            scheduled_payments: vec![],
            unmatched_count: 0,
            evidence_start: "2026-05-01".into(),
            category_matched_names: vec![],
        }
    }

    fn candidate() -> BillLinkCandidate {
        BillLinkCandidate {
            transaction_id: "42".into(),
            occurred_on: "2026-10-03".into(),
            description: "GYM CLUB".into(),
            amount: Decimal::from(30),
            evidence: "Bill name in transaction description".into(),
            category: Some("Sport".into()),
        }
    }

    fn payment() -> Tx {
        let mut t = tx("firefly:42", Kind::Outflow, "30", "EUR", AT);
        t.ledger_id = 42;
        t.description = Some("GYM CLUB".into());
        t.category = Some("Sport".into());
        t
    }

    fn checks(transactions: &[Tx], stale: bool) -> Checks<'_> {
        Checks {
            currencies: HashSet::from(["EUR".to_owned()]),
            categories: ["sport", "streaming"].map(str::to_owned).into(),
            expense_categories: [("sport", "Sport"), ("streaming", "Streaming")]
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .into(),
            views: vec![view(vec![row("7", 0, vec![candidate()])], stale)],
            transactions,
        }
    }

    fn with_writer<T>(store: &mut Store, f: impl FnOnce(&Writer<'_>) -> T) -> T {
        let writer = Writer {
            tx: store.connection_mut().transaction().unwrap(),
            now: AT.parse().unwrap(),
        };
        let value = f(&writer);
        writer.tx.commit().unwrap();
        value
    }

    fn create(name: &str, category: &str, min: &str) -> BillCreateRequest {
        BillCreateRequest {
            name: name.into(),
            match_category: category.into(),
            amount_min: min.parse().unwrap(),
            amount_max: "12.99".parse().unwrap(),
            currency_code: "EUR".into(),
            date: "2026-11-02".into(),
            repeat_freq: "monthly".into(),
            skip: 0,
            end_date: None,
        }
    }

    fn rejected<T: std::fmt::Debug>(result: Result<T>) -> String {
        match result {
            Err(Error::Rejected(message)) => message,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn creating_a_bill_stores_it_with_its_category_rule() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        let saved = with_writer(&mut store, |w| {
            create_bill(w, &checks, create(" Netflix ", "streaming", "12.99"))
        })
        .unwrap();
        assert_eq!(saved.bill_id, "8");
        let conn = store.connection();
        let bill: (String, String, String, String, i64) = conn
            .query_row(
                "SELECT name, amount_min, start_date, repeat_freq, active FROM bills WHERE id = 8",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            bill,
            (
                "Netflix".into(),
                "12.990000000000".into(),
                "2026-11-02".into(),
                "monthly".into(),
                1
            )
        );
        let rule: String = conn
            .query_row(
                "SELECT payload_json FROM subscription_category_rules WHERE bill_id = '8'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let rule: SubscriptionCategoryRule = serde_json::from_str(&rule).unwrap();
        assert_eq!(
            (rule.categories, rule.revision),
            (vec!["streaming".to_owned()], 1)
        );
    }

    #[test]
    fn creating_validates_like_the_service() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        with_writer(&mut store, |w| {
            assert_eq!(
                rejected(create_bill(w, &checks, create("A", "Sport", "0"))),
                INVALID
            );
            assert_eq!(
                rejected(create_bill(w, &checks, create("A", "Sport", "13"))),
                INVALID
            );
            let mut weird = create("A", "Sport", "1");
            weird.repeat_freq = "daily".into();
            assert_eq!(rejected(create_bill(w, &checks, weird)), INVALID);
            let mut every_third = create("A", "Sport", "1");
            every_third.skip = 2;
            assert_eq!(rejected(create_bill(w, &checks, every_third)), INVALID);
            let mut usd = create("A", "Sport", "1");
            usd.currency_code = "USD".into();
            assert_eq!(rejected(create_bill(w, &checks, usd)), INVALID);
            let mut no_date = create("A", "Sport", "1");
            no_date.date = "2026-13-01".into();
            assert_eq!(rejected(create_bill(w, &checks, no_date)), INVALID);
            let mut ends_first = create("A", "Sport", "1");
            ends_first.end_date = Some("2026-11-01".into());
            assert_eq!(rejected(create_bill(w, &checks, ends_first)), INVALID);
            let mut bad_end = create("A", "Sport", "1");
            bad_end.end_date = Some("soon".into());
            assert_eq!(rejected(create_bill(w, &checks, bad_end)), INVALID);
            assert_eq!(
                rejected(create_bill(w, &checks, create("A", "Groceries", "1"))),
                UNKNOWN_CATEGORY
            );
            assert_eq!(
                rejected(create_bill(w, &checks, create("gym", "Sport", "1"))),
                DUPLICATE_NAME
            );
            create_bill(w, &checks, create("Club", "Sport", "1")).unwrap();
            // The category now belongs to Club.
            assert_eq!(
                rejected(create_bill(w, &checks, create("Pool", "SPORT", "1"))),
                CHANGED
            );
        });
    }

    #[test]
    fn editing_changes_the_bill_and_keeps_the_schedule_unless_given() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        let edit = |date: Option<&str>| BillEditRequest {
            bill_id: "7".into(),
            name: "Gym+".into(),
            amount_min: "0".parse().unwrap(),
            amount_max: "35.5".parse().unwrap(),
            repeat_freq: "quarterly".into(),
            date: date.map(str::to_owned),
        };
        with_writer(&mut store, |w| edit_bill(w, &checks, edit(None))).unwrap();
        let read = |store: &Store| -> (String, String, String, String) {
            store
                .connection()
                .query_row(
                    "SELECT name, amount_max, repeat_freq, start_date FROM bills WHERE id = 7",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap()
        };
        assert_eq!(
            read(&store),
            (
                "Gym+".into(),
                "35.500000000000".into(),
                "quarterly".into(),
                "2026-09-03".into()
            )
        );
        with_writer(&mut store, |w| {
            edit_bill(w, &checks, edit(Some("2026-12-01")))
        })
        .unwrap();
        assert_eq!(read(&store).3, "2026-12-01");

        let stale = super::tests::checks(&transactions, true);
        with_writer(&mut store, |w| {
            assert_eq!(rejected(edit_bill(w, &stale, edit(None))), CHANGED);
            let mut other = edit(None);
            other.bill_id = "9".into();
            assert_eq!(rejected(edit_bill(w, &checks, other)), CHANGED);
            let mut backwards = edit(None);
            backwards.amount_min = "40".parse().unwrap();
            assert_eq!(rejected(edit_bill(w, &checks, backwards)), INVALID);
        });
    }

    #[test]
    fn archiving_keeps_the_bill_and_frees_its_categories() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        with_writer(&mut store, |w| {
            save_subscription_categories(
                w,
                &checks,
                SubscriptionCategoriesRequest {
                    bill_id: "7".into(),
                    categories: vec!["sport".into()],
                    expected_revision: 0,
                },
            )
            .unwrap();
            archive_bill(
                w,
                BillRef {
                    bill_id: "7".into(),
                },
            )
            .unwrap();
        });
        let active: bool = store
            .connection()
            .query_row("SELECT active FROM bills WHERE id = 7", [], |r| r.get(0))
            .unwrap();
        assert!(!active);
        let (revision, payload): (u32, String) = store
            .connection()
            .query_row(
                "SELECT revision, payload_json FROM subscription_category_rules WHERE bill_id = '7'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let rule: SubscriptionCategoryRule = serde_json::from_str(&payload).unwrap();
        assert_eq!((revision, rule.categories.len()), (2, 0));
        let missing = with_writer(&mut store, |w| {
            archive_bill(
                w,
                BillRef {
                    bill_id: "99".into(),
                },
            )
        });
        assert_eq!(rejected(missing), CHANGED);
    }

    #[test]
    fn category_rules_check_revisions_and_ownership() {
        let mut store = store();
        let transactions = [payment()];
        let mut checks = checks(&transactions, false);
        checks.views[0].rows.push(row("8", 0, vec![]));
        let request = |bill: &str, categories: &[&str], revision| SubscriptionCategoriesRequest {
            bill_id: bill.into(),
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            expected_revision: revision,
        };
        with_writer(&mut store, |w| {
            let saved =
                save_subscription_categories(w, &checks, request("7", &[" sport "], 0)).unwrap();
            // The stored spelling, not the requested one.
            assert_eq!(
                (saved.categories, saved.revision),
                (vec!["Sport".to_owned()], 1)
            );
            assert_eq!(
                rejected(save_subscription_categories(
                    w,
                    &checks,
                    request("7", &["Streaming"], 0)
                )),
                CATEGORIES_CHANGED
            );
            assert_eq!(
                rejected(save_subscription_categories(
                    w,
                    &checks,
                    request("8", &["Sport"], 0)
                )),
                CATEGORIES_CHANGED
            );
            assert_eq!(
                rejected(save_subscription_categories(
                    w,
                    &checks,
                    request("8", &["Rent"], 0)
                )),
                CATEGORIES_INVALID
            );
            assert_eq!(
                rejected(save_subscription_categories(
                    w,
                    &checks,
                    request("8", &["Streaming", "streaming"], 0)
                )),
                CATEGORIES_INVALID
            );
            assert_eq!(
                rejected(save_subscription_categories(
                    w,
                    &checks,
                    request("5", &[], 0)
                )),
                CATEGORIES_CHANGED
            );
            let off = save_subscription_categories(w, &checks, request("7", &[], 1)).unwrap();
            assert_eq!((off.categories.len(), off.revision), (0, 2));
        });
    }

    #[test]
    fn linking_sets_the_transactions_bill_once() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        let request = || BillMatchRequest {
            bill_id: "7".into(),
            transaction_id: "42".into(),
        };
        with_writer(&mut store, |w| link_bill_payment(w, &checks, request())).unwrap();
        let bill: Option<i64> = store
            .connection()
            .query_row("SELECT bill_id FROM transactions WHERE id = 42", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(bill, Some(7));
        // The snapshot still offers it, but the ledger moved on.
        let again = with_writer(&mut store, |w| link_bill_payment(w, &checks, request()));
        assert_eq!(rejected(again), MATCH_CHANGED);

        let mut changed = payment();
        changed.amount = Decimal::from(31);
        let changed = [changed];
        let checks = super::tests::checks(&changed, false);
        let mut store = super::tests::store();
        let result = with_writer(&mut store, |w| link_bill_payment(w, &checks, request()));
        assert_eq!(rejected(result), MATCH_CHANGED);
    }

    #[test]
    fn declining_needs_a_current_suggestion() {
        let mut store = store();
        let transactions = [payment()];
        let checks = checks(&transactions, false);
        let request = |transaction: &str| BillMatchRequest {
            bill_id: "7".into(),
            transaction_id: transaction.into(),
        };
        with_writer(&mut store, |w| {
            decline_bill_match(w, &checks, request("42"))
        })
        .unwrap();
        let count: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM declined_bill_matches WHERE bill_id = '7' AND transaction_id = '42'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let unknown = with_writer(&mut store, |w| {
            decline_bill_match(w, &checks, request("43"))
        });
        assert_eq!(rejected(unknown), MATCH_CHANGED);
        let stale = super::tests::checks(&transactions, true);
        let result = with_writer(&mut store, |w| decline_bill_match(w, &stale, request("42")));
        assert_eq!(rejected(result), MATCH_CHANGED);
    }
}
