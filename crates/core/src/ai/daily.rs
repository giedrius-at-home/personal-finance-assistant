//! Daily review and automatic approval (Python `daily_review.py`
//! `DailyReviewRunner`): keep the daily inbox current and save Jev
//! suggestions at the policy threshold (≥ 0.90) as the ledger category, when
//! the owner turned automatic approval on.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{DailyReviewEntry, DailyReviewPolicy};
use finance_types::report::review::{CategorisationMethod, CategoryProposal, ProposalStatus};
use jiff::SignedDuration;
use jiff::tz::TimeZone;

use super::text::{has_merchant_description, normalize_merchant};
use crate::model::{Kind, Tx};
use crate::review::latest_proposals;
use crate::service::Service;
use crate::writes::review::{
    approve, batch_token, get_pending, reconcile_daily_review, save_entry,
};
use crate::writes::transactions::set_ledger_category;
use crate::writes::{Writer, utc_stamp};
use crate::{Error, Result, Snapshot};

/// `_merchant_category_history`: confirmed categories per normalised merchant.
fn merchant_history(transactions: &[Tx]) -> HashMap<String, Vec<&str>> {
    let mut index: HashMap<String, Vec<&str>> = HashMap::new();
    for t in transactions {
        let Some(category) = t.category.as_deref() else {
            continue;
        };
        if t.kind == Kind::Transfer || !has_merchant_description(t.description.as_deref()) {
            continue;
        }
        index
            .entry(normalize_merchant(t.description.as_deref().unwrap_or("")))
            .or_default()
            .push(category);
    }
    index
}

/// The proposals a pass may save, each with its (new or open) entry.
pub fn eligible(
    policy: &DailyReviewPolicy,
    transactions: &[Tx],
    proposals: &[CategoryProposal],
    entries: &[DailyReviewEntry],
    deferred: &HashSet<String>,
    now: &jiff::Zoned,
) -> Result<Vec<(DailyReviewEntry, CategoryProposal)>> {
    let history = merchant_history(transactions);
    let latest: HashMap<&str, &CategoryProposal> = proposals
        .iter()
        .filter(|p| p.source.system == "firefly_iii" && p.source.entity == "transaction")
        .map(|p| (p.source.external_id.as_str(), p))
        .collect();
    let entries: HashMap<&str, &DailyReviewEntry> = entries
        .iter()
        .map(|e| (e.transaction_id.as_str(), e))
        .collect();
    let zone = TimeZone::get(&policy.timezone).unwrap_or_else(|_| now.time_zone().clone());
    let mut found = Vec::new();
    for t in transactions {
        let entry = entries.get(t.transaction_id.as_str());
        if entry.is_some_and(|e| e.resolved_at.is_some() || e.auto_error.is_some()) {
            continue;
        }
        let Some(proposal) = latest.get(t.ledger_id.to_string().as_str()) else {
            continue;
        };
        let merchant = normalize_merchant(t.description.as_deref().unwrap_or(""));
        let ok = !deferred.contains(&t.source_key())
            && proposal.status == ProposalStatus::Pending
            && proposal.proposed_category.is_some()
            && proposal
                .confidence
                .is_some_and(|c| c >= policy.auto_threshold)
            && proposal
                .model_version
                .as_deref()
                .is_some_and(|m| m.starts_with("jev-"))
            && proposal.method == CategorisationMethod::Model
            && has_merchant_description(t.description.as_deref())
            && t.kind != Kind::Transfer
            && t.category.is_none()
            && proposal.original_category.is_none()
            && history
                .get(&merchant)
                .into_iter()
                .flatten()
                .all(|c| Some(*c) == proposal.proposed_category.as_deref());
        if !ok {
            continue;
        }
        let entry = match entry {
            Some(e) => (*e).clone(),
            None => {
                let instant = now.timestamp();
                let due = instant
                    .checked_add(SignedDuration::from_hours(
                        24 * i64::from(policy.missed_after_days),
                    ))
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                DailyReviewEntry {
                    transaction_id: t.transaction_id.clone(),
                    batch_date: instant.to_zoned(zone.clone()).date().to_string(),
                    first_seen: utc_stamp(instant),
                    due_at: utc_stamp(due),
                    resolved_at: None,
                    resolution: None,
                    missed_at: None,
                    auto_error: None,
                    auto_proposal_id: None,
                    auto_category: None,
                    audited_at: None,
                }
            }
        };
        found.push((entry, (*proposal).clone()));
    }
    Ok(found)
}

/// Save one eligible proposal as the ledger category. A category that
/// changed underneath leaves the entry with an `auto_error` for the owner.
fn apply_one(w: &Writer<'_>, entry: DailyReviewEntry, proposal: &CategoryProposal) -> Result<bool> {
    let Some((_, payload)) = get_pending(w, &proposal.proposal_id)? else {
        return Ok(false);
    };
    let (Some(category), Ok(ledger)) = (
        proposal.proposed_category.as_deref(),
        proposal.source.external_id.parse::<i64>(),
    ) else {
        return Ok(false);
    };
    let staged = DailyReviewEntry {
        auto_proposal_id: Some(proposal.proposal_id.clone()),
        auto_category: Some(category.to_owned()),
        ..entry
    };
    save_entry(w, &staged)?;
    if set_ledger_category(
        w,
        ledger,
        proposal.original_category.as_deref(),
        Some(category),
    )?
    .is_err()
    {
        save_entry(
            w,
            &DailyReviewEntry {
                auto_error: Some("FireflyCategoryConflictError".into()),
                ..staged
            },
        )?;
        return Ok(false);
    }
    approve(
        w,
        &[proposal.proposal_id.as_str()],
        &batch_token(&[&payload]),
    )?;
    save_entry(
        w,
        &DailyReviewEntry {
            resolved_at: Some(w.stamp()),
            resolution: Some("automatic".into()),
            ..staged
        },
    )?;
    Ok(true)
}

/// One daily pass: reconcile the inbox, then (when `auto_apply`) save every
/// eligible Jev suggestion. Returns how many categories were saved.
pub fn pass(w: &Writer<'_>, snap: &Snapshot, auto_apply: bool) -> Result<usize> {
    reconcile_daily_review(w, snap, &HashMap::new())?;
    let Some(policy) = snap.daily_review_policy.as_ref() else {
        return Ok(0);
    };
    if !auto_apply {
        return Ok(0);
    }
    let entries: Vec<DailyReviewEntry> = {
        let mut stmt =
            w.tx.prepare("SELECT payload_json FROM daily_review_entries")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter()
            .filter_map(|p| serde_json::from_str(p).ok())
            .collect()
    };
    let proposals = latest_proposals(&w.tx)?;
    let selected = eligible(
        policy,
        &snap.transactions,
        &proposals,
        &entries,
        &snap.deferred_sources,
        &w.now,
    )?;
    let mut applied = 0;
    for (entry, proposal) in selected {
        w.tx.execute_batch("SAVEPOINT auto_apply")?;
        match apply_one(w, entry, &proposal) {
            Ok(saved) => {
                w.tx.execute_batch("RELEASE auto_apply")?;
                applied += usize::from(saved);
            }
            Err(e) => {
                w.tx.execute_batch("ROLLBACK TO auto_apply; RELEASE auto_apply")?;
                return Err(e);
            }
        }
    }
    Ok(applied)
}

impl Service {
    /// The daily review pass; screens reload only when something changed.
    pub fn daily_review_pass(&self, auto_apply: bool) -> std::result::Result<usize, String> {
        self.write_if_changed(|w, snap| pass(w, snap, auto_apply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::fixture;

    fn jev(db: &fixture::Db, id: &str, ledger: i64, confidence: &str) {
        db.proposal(id, ledger, Some("Groceries"));
        db.store
            .connection()
            .execute(
                "UPDATE category_proposals SET payload_json = json_set(payload_json, '$.confidence', ?1) \
                 WHERE proposal_id = ?2",
                rusqlite::params![confidence, id],
            )
            .unwrap();
    }

    #[test]
    fn saves_confident_jev_suggestions_once() {
        let mut db = fixture::db();
        let id = "b".repeat(64);
        jev(&db, &id, 10, "0.95");
        let applied = db.write(|w, snap| pass(w, snap, true)).unwrap();
        assert_eq!(applied, 1);
        let category: String =
            db.one("SELECT c.name FROM transactions t JOIN categories c ON c.id = t.category_id WHERE t.id = 10");
        assert_eq!(category, "Groceries");
        let status: String = db.one(&format!(
            "SELECT status FROM category_proposals WHERE proposal_id = '{id}'"
        ));
        assert_eq!(status, "approved");
        let entry: String = db.one(
            "SELECT payload_json FROM daily_review_entries WHERE transaction_id = 'firefly:10'",
        );
        let entry: DailyReviewEntry = serde_json::from_str(&entry).unwrap();
        assert_eq!(entry.resolution.as_deref(), Some("automatic"));
        assert_eq!(entry.auto_proposal_id.as_deref(), Some(id.as_str()));
        assert_eq!(db.write(|w, snap| pass(w, snap, true)).unwrap(), 0);
    }

    #[test]
    fn leaves_weak_local_or_disabled_suggestions_for_review() {
        let mut db = fixture::db();
        jev(&db, &"c".repeat(64), 10, "0.89");
        assert_eq!(db.write(|w, snap| pass(w, snap, true)).unwrap(), 0);
        let mut db = fixture::db();
        jev(&db, &"d".repeat(64), 10, "0.99");
        db.store
            .connection()
            .execute(
                "UPDATE category_proposals SET payload_json = json_set(payload_json, '$.model_version', 'qwen3:8b')",
                [],
            )
            .unwrap();
        assert_eq!(db.write(|w, snap| pass(w, snap, true)).unwrap(), 0);
        let mut db = fixture::db();
        jev(&db, &"e".repeat(64), 10, "0.99");
        assert_eq!(db.write(|w, snap| pass(w, snap, false)).unwrap(), 0);
        let category: Option<i64> = db.one("SELECT category_id FROM transactions WHERE id = 10");
        assert_eq!(category, None);
    }

    #[test]
    fn a_conflicting_merchant_history_blocks_automation() {
        let mut db = fixture::db();
        db.store
            .connection()
            .execute(
                "INSERT INTO transactions (id, kind, date, booked_at, description, source_account_id, \
                 destination_account_id, category_id, tags) VALUES \
                 (14, 'withdrawal', '2026-10-24', '2026-10-24T12:00:00', 'Lidl Utrecht', 1, 2, 2, '[]')",
                [],
            )
            .unwrap();
        db.store
            .connection()
            .execute(
                "INSERT INTO postings (transaction_id, account_id, amount, currency) VALUES \
                 (14, 1, '-1.00', 'EUR'), (14, 2, '1.00', 'EUR')",
                [],
            )
            .unwrap();
        jev(&db, &"f".repeat(64), 10, "0.99");
        assert_eq!(db.write(|w, snap| pass(w, snap, true)).unwrap(), 0);
    }
}
