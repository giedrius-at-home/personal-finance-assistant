//! Transactions tab writes that are not category-review decisions: a
//! transaction's category (`/transactions/category`), category
//! consolidation (`/categories/consolidate/*`), shared-spending marks
//! (`/transactions/personal`), owner details (`/review/{id}/details`) and the
//! automatic-category audit (`/daily-review/audit`).
//!
//! Python wrote categories to Firefly; here they go to the ledger
//! (`transactions.category_id`, a new `categories` row for a new name) and
//! the search index is rebuilt for the changed row.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use finance_types::gateway::{DailyReviewEntry, GroupMapping, OwnerDetails, SavedDetails};
use finance_types::report::review::{
    Audited, CategoryChange, CategorySaved, Consolidation, ConsolidationApplied,
    ConsolidationPreview, DetailsSave, PersonalChoice, PersonalLink, PersonalSaved,
    PersonalTransaction, PersonalTransactions,
};
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use super::{Writer, invalid};
use crate::model::{Kind, Tx};
use crate::personal::{PERSONAL_REVIEW_START, link_candidates, paid_back};
use crate::text::{casefold, is_country};
use crate::{Error, Result, Snapshot};

/// Review policy (`config/review-policy.json` → `enrichment`, `categories`).
pub const OWNER_TAGS_LIMIT: usize = 20;
pub const TAG_LENGTH_LIMIT: usize = 80;
pub const DETAIL_LENGTH_LIMIT: usize = 200;
pub const NOTES_LENGTH_LIMIT: usize = 2000;
pub const CONSOLIDATION_LIMIT: usize = 100;

/// Python's `max_length` counts characters, not bytes.
fn chars(text: &str) -> usize {
    text.chars().count()
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `datetime.now(UTC).isoformat()`, the format of the gateway tables' own
/// timestamp columns (payloads use [`Writer::stamp`]).
pub(crate) fn iso_now(w: &Writer<'_>) -> String {
    w.now
        .timestamp()
        .strftime("%Y-%m-%dT%H:%M:%S%.6f+00:00")
        .to_string()
}

// ---------------------------------------------------------------------------
// Ledger categories

/// The ledger no longer holds the category the owner saw (Firefly's
/// `FireflyCategoryConflictError`).
#[derive(Debug)]
pub(crate) struct Conflict;

/// The id of the category called exactly `name`, created when new.
fn category_id(w: &Writer<'_>, name: &str) -> Result<i64> {
    let found: Option<i64> =
        w.tx.query_row(
            "SELECT id FROM categories WHERE name = ?1 ORDER BY id LIMIT 1",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    match found {
        Some(id) => Ok(id),
        None => {
            w.tx.execute("INSERT INTO categories (name) VALUES (?1)", [name])?;
            Ok(w.tx.last_insert_rowid())
        }
    }
}

/// Description, source and destination account, category, notes, tags.
type SearchParts = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// Rebuild the search text of one transaction exactly as the ledger import
/// wrote it.
fn reindex(w: &Writer<'_>, id: i64) -> Result<()> {
    let row: Option<SearchParts> = w
        .tx
        .query_row(
            "SELECT t.description, sa.name, da.name, c.name, t.notes, t.tags FROM transactions AS t \
             LEFT JOIN accounts AS sa ON sa.id = t.source_account_id \
             LEFT JOIN accounts AS da ON da.id = t.destination_account_id \
             LEFT JOIN categories AS c ON c.id = t.category_id WHERE t.id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((description, source, destination, category, notes, tags)) = row else {
        return Ok(());
    };
    let tags: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
    let mut text = vec![description.as_str()];
    text.extend(source.as_deref());
    text.extend(destination.as_deref());
    text.extend(category.as_deref());
    text.extend(notes.as_deref());
    text.extend(tags.iter().map(String::as_str));
    w.tx.execute("DELETE FROM transactions_search WHERE rowid = ?1", [id])?;
    w.tx.execute(
        "INSERT INTO transactions_search (rowid, text) VALUES (?1, ?2)",
        params![id, text.join(" \u{2022} ")],
    )?;
    Ok(())
}

/// Firefly's `set_category`: apply `category` if the ledger still holds
/// `expected`; an already-applied category is a success.
pub(crate) fn set_ledger_category(
    w: &Writer<'_>,
    id: i64,
    expected: Option<&str>,
    category: Option<&str>,
) -> Result<std::result::Result<(), Conflict>> {
    let current: Option<Option<String>> =
        w.tx.query_row(
            "SELECT c.name FROM transactions AS t LEFT JOIN categories AS c \
             ON c.id = t.category_id WHERE t.id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(current) = current else {
        return Ok(Err(Conflict));
    };
    if current.as_deref() == category {
        return Ok(Ok(()));
    }
    if current.as_deref() != expected {
        return Ok(Err(Conflict));
    }
    let category_id = category.map(|name| category_id(w, name)).transpose()?;
    w.tx.execute(
        "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
        params![category_id, id],
    )?;
    reindex(w, id)?;
    Ok(Ok(()))
}

// ---------------------------------------------------------------------------
// One transaction's category

const CATEGORY_CHANGED: &str = "This payment changed. Refresh and try again.";

/// `/transactions/category`: set one categorisable transaction's category
/// if it still has the category the owner saw.
pub fn set_transaction_category(
    w: &Writer<'_>,
    snap: &Snapshot,
    change: CategoryChange,
) -> Result<CategorySaved> {
    let category = change.category.trim().to_owned();
    let expected = change.expected_category.as_deref().map(str::trim);
    if !(1..=128).contains(&chars(&change.transaction_id))
        || expected.is_some_and(|e| chars(e) > 128)
        || category.is_empty()
        || chars(&category) > 128
    {
        return Err(invalid("Choose a valid category."));
    }
    // The review inbox sends canonical ids; other callers send ledger ids.
    let Some(t) = snap.raw_transactions.iter().find(|t| {
        t.ledger_id.to_string() == change.transaction_id
            || t.transaction_id == change.transaction_id
    }) else {
        return Err(invalid(CATEGORY_CHANGED));
    };
    if t.kind == Kind::Transfer || t.category.as_deref() != expected {
        return Err(invalid(CATEGORY_CHANGED));
    }
    if set_ledger_category(w, t.ledger_id, expected, Some(&category))?.is_err() {
        return Err(invalid(CATEGORY_CHANGED));
    }
    // Firefly's webhook made the sync resolve the daily review right away.
    let changed = HashMap::from([(t.ledger_id, Some(category.clone()))]);
    super::review::reconcile_daily_review(w, snap, &changed)?;
    Ok(CategorySaved {
        saved: true,
        category,
    })
}

// ---------------------------------------------------------------------------
// Category consolidation

const CONSOLIDATION_INVALID: &str = "Cannot consolidate this selection. Choose two current categories with 1–100 affected transactions.";
const CONSOLIDATION_CHANGED: &str = "Transactions changed. Refresh and preview again.";

fn consolidation_plan<'a>(
    snap: &'a Snapshot,
    request: &Consolidation,
) -> Result<(Vec<&'a Tx>, String)> {
    let (source, target) = (request.source.as_str(), request.target.as_str());
    if !(1..=128).contains(&chars(source)) || !(1..=128).contains(&chars(target)) {
        return Err(invalid("Choose two current categories."));
    }
    let names: HashSet<&str> = snap
        .raw_transactions
        .iter()
        .filter_map(|t| t.category.as_deref())
        .filter(|c| !c.is_empty())
        .collect();
    if source == target || !names.contains(source) || !names.contains(target) {
        return Err(invalid(CONSOLIDATION_INVALID));
    }
    let mut selected: Vec<&Tx> = snap
        .raw_transactions
        .iter()
        .filter(|t| t.category.as_deref() == Some(source))
        .collect();
    selected.sort_by(|a, b| a.transaction_id.cmp(&b.transaction_id));
    if selected.is_empty() || selected.len() > CONSOLIDATION_LIMIT {
        return Err(invalid(CONSOLIDATION_INVALID));
    }
    let rows: Vec<String> = selected
        .iter()
        .map(|t| {
            format!(
                "{}|{}|{}|{}|{}",
                t.transaction_id,
                t.kind.as_str(),
                t.amount,
                t.currency.code,
                t.description.as_deref().unwrap_or("")
            )
        })
        .collect();
    let payload = serde_json::json!([source, target, rows]).to_string();
    Ok((selected, sha256_hex(&payload)))
}

/// `/categories/consolidate/preview`.
pub fn preview_consolidation(
    snap: &Snapshot,
    request: &Consolidation,
) -> Result<ConsolidationPreview> {
    let (selected, token) = consolidation_plan(snap, request)?;
    Ok(ConsolidationPreview {
        source: request.source.clone(),
        target: request.target.clone(),
        count: selected.len(),
        token,
    })
}

/// `/categories/consolidate/apply`: move every transaction of the source
/// category to the target, if nothing changed since the preview.
pub fn apply_consolidation(
    w: &Writer<'_>,
    snap: &Snapshot,
    request: Consolidation,
) -> Result<ConsolidationApplied> {
    let Some(token) = request.token.as_deref() else {
        return Err(invalid("Preview the consolidation first."));
    };
    let (selected, expected) = consolidation_plan(snap, &request)?;
    if token != expected {
        return Err(invalid(CONSOLIDATION_CHANGED));
    }
    for t in &selected {
        // A conflict undoes the whole move (the write rolls back), as a
        // review batch does: never half the transactions moved.
        if set_ledger_category(w, t.ledger_id, Some(&request.source), Some(&request.target))?
            .is_err()
        {
            return Err(invalid(CONSOLIDATION_CHANGED));
        }
    }
    Ok(ConsolidationApplied {
        applied: selected.len(),
        total: selected.len(),
        complete: true,
        // The ledger is the source: nothing waits for a refresh.
        refreshed: true,
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Owner details

/// The saved details of `key`, read inside the write.
fn load_details(w: &Writer<'_>, key: &str) -> Result<SavedDetails> {
    let row: Option<(u32, String)> =
        w.tx.query_row(
            "SELECT revision, payload_json FROM transaction_details WHERE source_key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        None => Ok(SavedDetails::default()),
        Some((revision, payload)) => Ok(SavedDetails {
            revision,
            values: serde_json::from_str(&payload)
                .map_err(|e| Error::Invalid(format!("transaction details: {e}")))?,
        }),
    }
}

/// Save `values` if nobody saved since `expected_revision`.
fn store_details(
    w: &Writer<'_>,
    key: &str,
    values: OwnerDetails,
    expected_revision: u32,
) -> Result<std::result::Result<SavedDetails, Conflict>> {
    if load_details(w, key)?.revision != expected_revision {
        return Ok(Err(Conflict));
    }
    let saved = SavedDetails {
        revision: expected_revision + 1,
        values,
    };
    w.put_json(
        "INSERT INTO transaction_details VALUES (?2, ?3, ?1) ON CONFLICT(source_key) \
         DO UPDATE SET revision = excluded.revision, payload_json = excluded.payload_json",
        &saved.values,
        &[&key, &saved.revision],
    )?;
    Ok(Ok(saved))
}

/// Trim what the site's form schema trims.
fn trimmed(mut values: OwnerDetails) -> OwnerDetails {
    for tag in &mut values.tags {
        tag.label = tag.label.trim().to_owned();
        tag.parent = tag.parent.trim().to_owned();
    }
    values.bulk_payment_name = values.bulk_payment_name.trim().to_owned();
    values.personal_expense_category = values.personal_expense_category.trim().to_owned();
    values
}

/// The site's `detailValuesSchema` (shape and policy limits).
fn form_valid(values: &OwnerDetails) -> bool {
    static COUNTRY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([A-Z]{2})?$").expect("valid regex"));
    values.tags.len() <= OWNER_TAGS_LIMIT
        && values.tags.iter().all(|t| {
            (1..=TAG_LENGTH_LIMIT).contains(&chars(&t.label))
                && chars(&t.parent) <= TAG_LENGTH_LIMIT
        })
        && [
            &values.merchant_name,
            &values.merchant_location,
            &values.payment_method,
        ]
        .iter()
        .all(|v| chars(v) <= DETAIL_LENGTH_LIMIT)
        && COUNTRY.is_match(&values.from_country)
        && COUNTRY.is_match(&values.to_country)
        && chars(&values.notes) <= NOTES_LENGTH_LIMIT
        && ["", "component", "settlement"].contains(&values.bulk_payment_role.as_str())
        && chars(&values.bulk_payment_name) <= 128
        && ["", "return", "payment", "ignore"].contains(&values.personal_expense_role.as_str())
        && chars(&values.personal_expense_category) <= 128
        && values.bulk_payment_role.is_empty() == values.bulk_payment_name.is_empty()
}

/// The Python `OwnerDetails` model rules beyond the form: known countries,
/// and a personal role with its category.
fn model_valid(values: &OwnerDetails) -> bool {
    [&values.from_country, &values.to_country]
        .iter()
        .all(|c| c.is_empty() || is_country(c))
        && values.bulk_payment_role.is_empty() == values.bulk_payment_name.is_empty()
        && ["", "ignore"].contains(&values.personal_expense_role.as_str())
            == values.personal_expense_category.trim().is_empty()
}

/// `/review/{id}/details`: save the owner's details & tags on the
/// transaction a proposal is about.
pub fn save_review_details(w: &Writer<'_>, save: DetailsSave) -> Result<SavedDetails> {
    static ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\w-]{1,128}$").expect("valid regex"));
    let values = trimmed(save.values);
    if !ID.is_match(&save.proposal_id) || !form_valid(&values) {
        return Err(invalid(format!(
            "Check the details: country codes need two uppercase letters; use at most {OWNER_TAGS_LIMIT} tags."
        )));
    }
    let payload: Option<String> =
        w.tx.query_row(
            "SELECT payload_json FROM category_proposals WHERE proposal_id = ?1",
            [&save.proposal_id],
            |r| r.get(0),
        )
        .optional()?;
    let source = payload
        .and_then(|p| {
            serde_json::from_str::<finance_types::report::review::CategoryProposal>(&p).ok()
        })
        .map(|p| p.source);
    let (Some(source), true) = (source, model_valid(&values)) else {
        return Err(invalid("Could not save details. Refresh before retrying."));
    };
    let key = format!(
        "[\"{}\", \"{}\", \"{}\"]",
        source.system, source.entity, source.external_id
    );
    match store_details(w, &key, values, save.revision)? {
        Ok(saved) => Ok(saved),
        Err(Conflict) => Err(invalid(
            "Details changed elsewhere. Reload before saving; your current edits are still here.",
        )),
    }
}

// ---------------------------------------------------------------------------
// Shared spending & money back

/// A bank memo that names a person or a payment request; never decides
/// a category or direction.
pub fn personal_candidate(description: Option<&str>) -> bool {
    static CUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(?:from|to)\s+[A-ZÀ-ž][\wÀ-ž'-]+\s+[A-ZÀ-ž]|\b(?:tikkie|betaalverzoek|incoming transfer|outgoing transfer)\b",
        )
        .expect("valid regex")
    });
    CUE.is_match(description.unwrap_or(""))
}

/// `GET /transactions/personal`: payments since the shared-spending start
/// with a bank-memo cue or a saved mark, or every payment whose memo
/// contains `query`.
pub fn personal_transactions(snap: &Snapshot, query: &str) -> Result<PersonalTransactions> {
    if chars(query) > 128 {
        return Err(invalid("Search is too long."));
    }
    let query = casefold(query.trim());
    let spending = &snap.transactions;
    let repaid = paid_back(spending);
    let by_id: HashMap<&str, &Tx> = spending
        .iter()
        .map(|t| (t.transaction_id.as_str(), t))
        .collect();
    let link_view = |t: &Tx| PersonalLink {
        transaction_id: t.ledger_id.to_string(),
        description: t
            .description
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "Bank transaction".into()),
        occurred_on: t.booked_on().to_string(),
        amount: t.money(),
        category: t.category.clone(),
        paid_back: repaid
            .get(t.transaction_id.as_str())
            .copied()
            .unwrap_or_default(),
    };
    let mut sorted: Vec<&Tx> = snap.raw_transactions.iter().collect();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.occurred_at.timestamp()));
    let mut rows = Vec::new();
    for t in sorted {
        if t.kind == Kind::Transfer || t.booked_on() < PERSONAL_REVIEW_START {
            continue;
        }
        let saved = snap.details.get(&t.source_key());
        let role = saved.map_or("", |s| s.values.personal_expense_role.as_str());
        if !query.is_empty() {
            if !casefold(t.description.as_deref().unwrap_or("")).contains(&query) {
                continue;
            }
        } else if role.is_empty() && !personal_candidate(t.description.as_deref()) {
            continue;
        }
        let incoming = matches!(t.kind, Kind::Inflow | Kind::Refund);
        // The money-back as budgets see it, with its link applied.
        let counted = by_id.get(t.transaction_id.as_str()).copied();
        let linked = counted
            .and_then(|c| c.repays.as_deref())
            .and_then(|origin| by_id.get(origin))
            .map(|o| link_view(o));
        let candidates = if incoming && role != "ignore" && role != "payment" {
            link_candidates(counted.unwrap_or(t), spending, &repaid, 3)
                .into_iter()
                .map(link_view)
                .collect()
        } else {
            Vec::new()
        };
        rows.push(PersonalTransaction {
            linked,
            link_candidates: candidates,
            transaction_id: t.ledger_id.to_string(),
            description: t
                .description
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "Bank transaction".into()),
            occurred_on: t.booked_on().to_string(),
            amount: t.money(),
            incoming,
            category: t.category.clone(),
            role: role.to_owned(),
            assigned_category: saved
                .map(|s| s.values.personal_expense_category.clone())
                .unwrap_or_default(),
            revision: saved.map_or(0, |s| s.revision),
        });
    }
    // Unreviewed rows first so archived choices never crowd them out of the cap.
    rows.sort_by_key(|r| !r.role.is_empty());
    let total = rows.len();
    rows.truncate(100);
    Ok(PersonalTransactions {
        items: rows,
        total,
        since: PERSONAL_REVIEW_START.to_string(),
    })
}

const PERSONAL_CHANGED: &str = "This transaction changed. Refresh before saving.";
const PERSONAL_LINK_INVALID: &str = "Link a purchase in the same currency, at most 60 days earlier, that this money back fits into.";
const PERSONAL_INVALID: &str =
    "Could not save this choice. Check the money direction and category.";

/// `POST /transactions/personal`: save (or clear) the owner's shared-spending
/// mark on one payment.
pub fn save_personal_choice(
    w: &Writer<'_>,
    snap: &Snapshot,
    choice: PersonalChoice,
) -> Result<PersonalSaved> {
    static AMOUNT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\d+(\.\d+)?$").expect("valid regex"));
    let role = choice.role.as_str();
    if !(1..=128).contains(&chars(&choice.transaction_id))
        || !["return", "payment", "ignore", "clear"].contains(&role)
        || chars(choice.category.trim()) > 128
        || !AMOUNT.is_match(&choice.expected_amount)
        || chars(&choice.expected_amount) > 32
        || chars(&choice.expected_currency) != 3
    {
        return Err(invalid("Choose a valid payment type and category."));
    }
    let Some(t) = snap
        .raw_transactions
        .iter()
        .find(|t| t.ledger_id.to_string() == choice.transaction_id)
    else {
        return Err(invalid(PERSONAL_CHANGED));
    };
    if t.kind == Kind::Transfer || t.booked_on() < PERSONAL_REVIEW_START {
        return Err(invalid(PERSONAL_CHANGED));
    }
    let category = if matches!(role, "clear" | "ignore") {
        String::new()
    } else {
        choice.category.trim().to_owned()
    };
    let amount: Decimal = choice
        .expected_amount
        .parse()
        .map_err(|_| invalid(PERSONAL_INVALID))?;
    if amount <= Decimal::ZERO || (!matches!(role, "clear" | "ignore") && category.is_empty()) {
        return Err(invalid(PERSONAL_INVALID));
    }
    let incoming = matches!(t.kind, Kind::Inflow | Kind::Refund);
    if (role == "return" && !incoming) || (role == "payment" && incoming) {
        return Err(invalid(PERSONAL_INVALID));
    }
    if amount != t.amount || choice.expected_currency != t.currency.code {
        return Err(invalid(PERSONAL_CHANGED));
    }
    let link = choice.link.trim();
    if !link.is_empty() {
        let purchase = snap
            .transactions
            .iter()
            .find(|p| p.ledger_id.to_string() == link);
        let others: Decimal = snap
            .transactions
            .iter()
            .filter(|o| o.kind == Kind::Refund && o.transaction_id != t.transaction_id)
            .filter(|o| purchase.is_some_and(|p| o.repays.as_deref() == Some(&p.transaction_id)))
            .map(|o| o.amount)
            .sum();
        let fits = purchase.is_some_and(|p| {
            role == "return"
                && p.amount - others >= t.amount
                && p.kind == Kind::Outflow
                && p.currency == t.currency
                && (-2..=crate::personal::LINK_LOOKBACK_DAYS)
                    .contains(&crate::outlook::days_between(p.booked_on(), t.booked_on()))
        });
        if !fits {
            return Err(invalid(PERSONAL_LINK_INVALID));
        }
    }
    let key = t.source_key();
    let mut values = load_details(w, &key)?.values;
    values.personal_expense_link = if role == "return" {
        link.to_owned()
    } else {
        String::new()
    };
    values.personal_expense_role = if role == "clear" {
        String::new()
    } else {
        role.to_owned()
    };
    values.personal_expense_category = category.clone();
    if !model_valid(&values) {
        return Err(invalid(PERSONAL_INVALID));
    }
    let saved = match store_details(w, &key, values, choice.revision)? {
        Ok(saved) => saved,
        Err(Conflict) => return Err(invalid(PERSONAL_CHANGED)),
    };
    // The two everyday choices get a spending group of their own name.
    if ["Groceries", "Eating out"].contains(&category.as_str())
        && !snap.group_mappings.iter().any(|m| m.category == category)
    {
        save_group_mapping(
            w,
            &GroupMapping {
                category: category.clone(),
                group: Some(category),
                revision: 0,
            },
        )?;
    }
    Ok(PersonalSaved {
        saved: true,
        revision: saved.revision,
    })
}

/// Append a reviewed group revision; a stale revision is a conflict.
fn save_group_mapping(w: &Writer<'_>, mapping: &GroupMapping) -> Result<()> {
    let changed = w.tx.execute(
        "INSERT INTO spending_group_history (category, revision, group_name, reviewed_at) \
         SELECT ?1, ?2, ?3, ?4 WHERE COALESCE((SELECT MAX(revision) \
         FROM spending_group_history WHERE category = ?1), 0) = ?5",
        params![
            mapping.category,
            mapping.revision + 1,
            mapping.group,
            iso_now(w),
            mapping.revision
        ],
    )?;
    if changed != 1 {
        return Err(invalid(PERSONAL_CHANGED));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Automatic-category audit

/// `/daily-review/audit`: the owner checked an automatically categorised
/// transaction.
pub fn audit_daily_review(w: &Writer<'_>, transaction_id: &str) -> Result<Audited> {
    if !(1..=128).contains(&chars(transaction_id)) {
        return Err(invalid("Invalid inventory item."));
    }
    let payload: Option<String> =
        w.tx.query_row(
            "SELECT payload_json FROM daily_review_entries WHERE transaction_id = ?1",
            [transaction_id],
            |r| r.get(0),
        )
        .optional()?;
    let entry: Option<DailyReviewEntry> = payload.and_then(|p| serde_json::from_str(&p).ok());
    let Some(mut entry) =
        entry.filter(|e| e.resolution.as_deref() == Some("automatic") && e.resolved_at.is_some())
    else {
        return Err(invalid(
            "This inventory item changed. Refresh and try again.",
        ));
    };
    if entry.audited_at.is_none() {
        entry.audited_at = Some(w.stamp());
        super::review::save_entry(w, &entry)?;
    }
    Ok(Audited { audited: true })
}

#[cfg(test)]
#[path = "transactions_tests.rs"]
mod tests;
