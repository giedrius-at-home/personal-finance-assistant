//! Category review decisions (Python `web.py` `/review/stream`,
//! `/review/{id}/{decision}`, `/review/batch`, `/review/{id}/category-options`)
//! and the daily review reconciliation they trigger
//! (`daily_review.reconcile_daily_review`).
//!
//! The Firefly category write of each decision is a ledger write here
//! ([`super::transactions::set_ledger_category`]); the proposal lifecycle
//! (`category_proposals`, `category_batches`, `category_corrections`,
//! `review_deferrals`, `daily_review_entries`) is stored exactly as Python did.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use finance_types::gateway::DailyReviewEntry;
use finance_types::report::review::{
    BatchDecision, BatchSaved, CategorisationMethod, CategoryAlternative, CategoryOptions,
    CategoryProposal, DecisionResult, DecisionStatus, DecisionsSaved, ProposalSource,
    ProposalStatus, ReviewAction, ReviewDecision,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use regex::{Captures, Regex};
use rusqlite::{OptionalExtension, params};

use super::transactions::{iso_now, set_ledger_category, sha256_hex};
use super::{Writer, invalid};
use crate::catalog::category_key;
use crate::load::parse_instant;
use crate::model::Kind;
use crate::review::latest_proposals;
use crate::text::casefold;
use crate::{Error, Result, Snapshot};

/// Python `domain/categorisation.MAX_CATEGORIES`.
const MAX_CATEGORIES: usize = 256;

fn chars(text: &str) -> usize {
    text.chars().count()
}

fn is_proposal_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A pending proposal and its stored payload.
fn get_pending(w: &Writer<'_>, proposal_id: &str) -> Result<Option<(CategoryProposal, String)>> {
    let payload: Option<String> = w
        .tx
        .query_row(
            "SELECT payload_json FROM category_proposals WHERE proposal_id = ?1 AND status = 'pending'",
            [proposal_id],
            |r| r.get(0),
        )
        .optional()?;
    payload
        .map(|p| {
            let proposal: CategoryProposal = serde_json::from_str(&p)
                .map_err(|e| Error::Invalid(format!("category proposal: {e}")))?;
            Ok((proposal, p))
        })
        .transpose()
}

/// The ledger id of a Firefly transaction proposal.
fn ledger_id(source: &ProposalSource) -> Option<i64> {
    (source.system == "firefly_iii" && source.entity == "transaction")
        .then(|| source.external_id.parse().ok())
        .flatten()
}

/// `json.dumps(identity)`: the `review_deferrals` / `transaction_details` key.
fn identity_key(source: &ProposalSource) -> String {
    format!(
        "[\"{}\", \"{}\", \"{}\"]",
        source.system, source.entity, source.external_id
    )
}

/// `categorisation_service.source_identity`.
fn source_identity(source: &ProposalSource) -> String {
    sha256_hex(&format!(
        "{}:{}:{}",
        source.system, source.entity, source.external_id
    ))
}

/// `_batch_token`: binds an approval to the exact proposals shown.
fn batch_token(payloads: &[&str]) -> String {
    let mut canonical: Vec<String> = payloads
        .iter()
        .map(|p| {
            let mut value: serde_json::Value = serde_json::from_str(p).unwrap_or_default();
            if let Some(map) = value.as_object_mut() {
                map.remove("status");
                map.remove("created_at");
            }
            // serde_json's map is sorted, like `sort_keys=True`.
            value.to_string()
        })
        .collect();
    canonical.sort();
    sha256_hex(&canonical.join("\n"))
}

fn approve(w: &Writer<'_>, ids: &[&str], token: &str) -> Result<()> {
    let now = iso_now(w);
    for id in ids {
        w.tx.execute(
            "UPDATE category_proposals SET status = 'approved', batch_id = ?1, updated_at = ?2 \
             WHERE proposal_id = ?3 AND status = 'pending'",
            params![token, now, id],
        )?;
    }
    w.tx.execute(
        "INSERT OR IGNORE INTO category_batches (batch_id, status, created_at) VALUES (?1, 'approved', ?2)",
        params![token, now],
    )?;
    Ok(())
}

fn resolve(w: &Writer<'_>, id: &str, status: ProposalStatus) -> Result<bool> {
    let status = match status {
        ProposalStatus::Corrected => "corrected",
        ProposalStatus::Rejected => "rejected",
        _ => return Err(Error::Invalid("unsupported proposal status".into())),
    };
    Ok(w.tx.execute(
        "UPDATE category_proposals SET status = ?1, updated_at = ?2 \
         WHERE proposal_id = ?3 AND status = 'pending'",
        params![status, iso_now(w), id],
    )? == 1)
}

fn record_correction(w: &Writer<'_>, proposal: &CategoryProposal, category: &str) -> Result<()> {
    w.tx.execute(
        "INSERT OR IGNORE INTO category_corrections (source_identity, feature_hash, \
         normalized_merchant, approved_category, approved_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            source_identity(&proposal.source),
            proposal.feature_hash,
            proposal.normalized_merchant,
            category,
            iso_now(w)
        ],
    )?;
    Ok(())
}

/// Correct to the most-used existing spelling of the same name
/// (`canonicalize_case_spacing`).
fn canonical_category(snap: &Snapshot, category: &str) -> String {
    let key = category_key(category);
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for c in snap
        .raw_transactions
        .iter()
        .filter_map(|t| t.category.as_deref())
    {
        if category_key(c) == key {
            *counts.entry(c).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .min_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)))
        .map_or_else(|| category.to_owned(), |(name, _)| name.to_owned())
}

/// Why one decision was not saved.
enum Skipped {
    Changed(String),
    Retry(String),
}

/// Python `_apply_category_review` for one decision (without the
/// reconciliation, which runs once per request).
fn apply_decision(
    w: &Writer<'_>,
    snap: &Snapshot,
    decision: &ReviewDecision,
) -> Result<std::result::Result<(), Skipped>> {
    let changed = |m: &str| Ok(Err(Skipped::Changed(m.to_owned())));
    let Some((proposal, payload)) = get_pending(w, &decision.proposal_id)? else {
        return changed("This transaction was already updated.");
    };
    if proposal.proposed_category != decision.expected_proposed_category
        || proposal.original_category != decision.expected_original_category
    {
        return changed("proposal changed after it was shown for review");
    }
    let id = proposal.proposal_id.as_str();
    match decision.decision {
        ReviewAction::Approve => {
            let Some(category) = proposal.proposed_category.as_deref() else {
                return changed("proposal is no longer approvable");
            };
            let Some(ledger) = ledger_id(&proposal.source) else {
                return changed("only Firefly transaction categories can be updated");
            };
            let token = batch_token(&[&payload]);
            if set_ledger_category(
                w,
                ledger,
                proposal.original_category.as_deref(),
                Some(category),
            )?
            .is_err()
            {
                return changed("Firefly category changed after this suggestion was created");
            }
            approve(w, &[id], &token)?;
        }
        ReviewAction::Defer => {
            w.tx.execute(
                "INSERT OR REPLACE INTO review_deferrals (source_key, updated_at) VALUES (?1, ?2)",
                params![identity_key(&proposal.source), iso_now(w)],
            )?;
        }
        ReviewAction::Reject => {
            if !resolve(w, id, ProposalStatus::Rejected)? {
                return changed("proposal is no longer pending");
            }
        }
        ReviewAction::Correct => {
            let category =
                canonical_category(snap, decision.category.as_deref().unwrap_or("").trim());
            if category.is_empty() {
                return Ok(Err(Skipped::Retry("category cannot be empty".into())));
            }
            let Some(ledger) = ledger_id(&proposal.source) else {
                return changed("only Firefly transaction categories can be updated");
            };
            if set_ledger_category(
                w,
                ledger,
                proposal.original_category.as_deref(),
                Some(&category),
            )?
            .is_err()
            {
                return changed("Firefly category changed after this suggestion was created");
            }
            record_correction(w, &proposal, &category)?;
            if !resolve(w, id, ProposalStatus::Corrected)? {
                return changed("proposal is no longer pending");
            }
        }
    }
    if decision.decision != ReviewAction::Defer {
        w.tx.execute(
            "DELETE FROM review_deferrals WHERE source_key = ?1",
            [identity_key(&proposal.source)],
        )?;
    }
    Ok(Ok(()))
}

fn decision_shape_valid(d: &ReviewDecision) -> bool {
    let expected_ok =
        |e: &Option<String>| e.as_deref().is_none_or(|e| (1..=128).contains(&chars(e)));
    let category_ok = match d.decision {
        ReviewAction::Approve | ReviewAction::Correct => d
            .category
            .as_deref()
            .is_some_and(|c| (1..=128).contains(&chars(c.trim()))),
        ReviewAction::Reject | ReviewAction::Defer => d.category.is_none(),
    };
    is_proposal_id(&d.proposal_id)
        && expected_ok(&d.expected_proposed_category)
        && expected_ok(&d.expected_original_category)
        && category_ok
}

/// `/review/stream`: apply up to 20 decisions, each on its own; the result
/// says per decision whether it was saved, dropped (changed) or can be retried.
pub fn review_decisions(
    w: &Writer<'_>,
    snap: &Snapshot,
    decisions: Vec<ReviewDecision>,
) -> Result<DecisionsSaved> {
    if !(1..=20).contains(&decisions.len()) || !decisions.iter().all(decision_shape_valid) {
        return Err(invalid("Invalid transaction decisions."));
    }
    let ids: HashSet<&str> = decisions.iter().map(|d| d.proposal_id.as_str()).collect();
    if ids.len() != decisions.len() {
        return Err(invalid("The review stream contains duplicates."));
    }
    let mut results = Vec::with_capacity(decisions.len());
    for decision in &decisions {
        w.tx.execute_batch("SAVEPOINT decision")?;
        let outcome = apply_decision(w, snap, decision)?;
        let (status, error) = match outcome {
            Ok(()) => {
                w.tx.execute_batch("RELEASE decision")?;
                (DecisionStatus::Saved, None)
            }
            Err(skipped) => {
                w.tx.execute_batch("ROLLBACK TO decision; RELEASE decision")?;
                match skipped {
                    Skipped::Changed(m) => (DecisionStatus::Changed, Some(m)),
                    Skipped::Retry(m) => (DecisionStatus::Retry, Some(m)),
                }
            }
        };
        results.push(DecisionResult {
            proposal_id: decision.proposal_id.clone(),
            status,
            error,
        });
    }
    let saved = results
        .iter()
        .filter(|r| r.status == DecisionStatus::Saved)
        .count();
    if saved > 0 {
        reconcile_daily_review(w, snap, &HashMap::new())?;
    }
    Ok(DecisionsSaved { saved, results })
}

const BATCH_CHANGED: &str = "This batch changed. Refresh and check it again.";

/// `/review/batch`: save the current daily batch at once. Unlike Python
/// (one Firefly write each), the whole batch is one transaction: all or nothing.
pub fn review_batch(
    w: &Writer<'_>,
    snap: &Snapshot,
    items: Vec<BatchDecision>,
) -> Result<BatchSaved> {
    let valid = (1..=100).contains(&items.len())
        && items.iter().all(|d| {
            is_proposal_id(&d.proposal_id)
                && (1..=128).contains(&chars(d.category.trim()))
                && (1..=128).contains(&chars(&d.expected_proposed_category))
                && d.expected_original_category
                    .as_deref()
                    .is_none_or(|e| (1..=128).contains(&chars(e)))
        });
    if !valid {
        return Err(invalid("Choose a category for every transaction."));
    }
    let ids: HashSet<&str> = items.iter().map(|d| d.proposal_id.as_str()).collect();
    if ids.len() != items.len() {
        return Err(invalid("The category batch contains duplicates."));
    }
    let mut selected = Vec::with_capacity(items.len());
    for item in &items {
        let Some((proposal, payload)) = get_pending(w, &item.proposal_id)? else {
            return Err(invalid(BATCH_CHANGED));
        };
        if proposal.proposed_category.as_deref() != Some(item.expected_proposed_category.as_str())
            || proposal.original_category != item.expected_original_category
        {
            return Err(invalid(BATCH_CHANGED));
        }
        selected.push((proposal, payload, item.category.trim().to_owned()));
    }
    for (proposal, _, category) in &selected {
        let Some(ledger) = ledger_id(&proposal.source) else {
            return Err(invalid(BATCH_CHANGED));
        };
        if set_ledger_category(
            w,
            ledger,
            proposal.original_category.as_deref(),
            Some(category),
        )?
        .is_err()
        {
            return Err(invalid(BATCH_CHANGED));
        }
    }
    let approvals: Vec<(&str, &str)> = selected
        .iter()
        .filter(|(p, _, c)| p.proposed_category.as_deref() == Some(c.as_str()))
        .map(|(p, payload, _)| (p.proposal_id.as_str(), payload.as_str()))
        .collect();
    if !approvals.is_empty() {
        let payloads: Vec<&str> = approvals.iter().map(|a| a.1).collect();
        let ids: Vec<&str> = approvals.iter().map(|a| a.0).collect();
        approve(w, &ids, &batch_token(&payloads))?;
    }
    for (proposal, _, category) in &selected {
        if proposal.proposed_category.as_deref() == Some(category.as_str()) {
            continue;
        }
        record_correction(w, proposal, category)?;
        if !resolve(w, &proposal.proposal_id, ProposalStatus::Corrected)? {
            return Err(invalid(BATCH_CHANGED));
        }
    }
    reconcile_daily_review(w, snap, &HashMap::new())?;
    Ok(BatchSaved {
        saved: selected.len(),
    })
}

/// `/review/{id}/category-options`: the ranked categories the suggestion
/// pass stored, limited to categories in use, plus every category in use.
pub fn category_options(
    conn: &rusqlite::Connection,
    snap: &Snapshot,
    proposal_id: &str,
) -> Result<CategoryOptions> {
    if !is_proposal_id(proposal_id) {
        return Err(invalid("Invalid transaction."));
    }
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM category_proposals WHERE proposal_id = ?1 AND status = 'pending'",
            [proposal_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(proposal) = payload.and_then(|p| serde_json::from_str::<CategoryProposal>(&p).ok())
    else {
        return Err(invalid("This review changed. Refresh and try again."));
    };
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for t in &snap.raw_transactions {
        if let Some(c) = t.category.as_deref().filter(|c| !c.is_empty())
            && t.kind != Kind::Transfer
        {
            *counts.entry(c).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, u64)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| casefold(a.0).cmp(&casefold(b.0)))
    });
    let existing: Vec<String> = ranked
        .into_iter()
        .take(MAX_CATEGORIES)
        .map(|(c, _)| c.to_owned())
        .collect();
    let known = |c: &str| existing.iter().any(|e| e == c);
    let mut options: Vec<CategoryAlternative> = proposal
        .category_options
        .iter()
        .filter(|o| known(&o.category))
        .cloned()
        .collect();
    if options.is_empty() {
        options = [
            (&proposal.proposed_category, proposal.confidence),
            (&proposal.runner_up, proposal.runner_up_confidence),
        ]
        .into_iter()
        .filter_map(|(c, p)| match (c, p) {
            (Some(c), Some(p)) if known(c) => Some(CategoryAlternative {
                category: c.clone(),
                probability: p,
            }),
            _ => None,
        })
        .collect();
    }
    options.truncate(4);
    Ok(CategoryOptions {
        options,
        existing_categories: existing,
    })
}

// ---------------------------------------------------------------------------
// Daily review reconciliation

pub(crate) fn save_entry(w: &Writer<'_>, entry: &DailyReviewEntry) -> Result<()> {
    w.put_json(
        "INSERT INTO daily_review_entries VALUES (?2, ?1) ON CONFLICT(transaction_id) \
         DO UPDATE SET payload_json = excluded.payload_json",
        entry,
        &[&entry.transaction_id],
    )
}

/// `categorisation_runtime.has_merchant_description`.
fn has_merchant_description(description: Option<&str>) -> bool {
    description.map(str::trim).is_some_and(|d| {
        !d.is_empty()
            && ![
                "(no description)",
                "no description",
                "no description provided",
                "unknown",
                "n/a",
            ]
            .contains(&casefold(d).as_str())
    })
}

/// `domain/categorisation.normalize_merchant`: one line, lower case, with
/// URLs reduced to their host and e-mail addresses, IBANs and long numbers
/// replaced by placeholders. (Python's NFKC step is left out.)
pub fn normalize_merchant(value: &str) -> String {
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+").expect("valid regex"));
    static EMAIL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[^\s@]+@[^\s@]+\.[^\s@]+\b").expect("valid regex"));
    static IBAN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").expect("valid regex"));
    static LONG_NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d{4,}\b").expect("valid regex"));
    static HOST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9.-]+$").expect("valid regex"));
    let text: String = value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{e000}'..='\u{f8ff}')
        })
        .collect();
    let text = URL.replace_all(&text, |c: &Captures<'_>| {
        let value = c[0].trim_end_matches(['.', ',', ';', ')']);
        let rest = value.split_once("://").map_or(value, |(_, rest)| rest);
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = authority.rsplit('@').next().unwrap_or("");
        let host = host.split(':').next().unwrap_or("").to_lowercase();
        if host.is_empty() || !HOST.is_match(&host) {
            " url ".to_owned()
        } else {
            format!(" {} ", host.strip_prefix("www.").unwrap_or(&host))
        }
    });
    let text = EMAIL.replace_all(&text, " email ");
    let text = IBAN.replace_all(&text, " account ");
    let text = LONG_NUMBER.replace_all(&text, " number ");
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .take(200)
        .collect()
}

/// Python `reconcile_daily_review`, run after review decisions: track new
/// uncategorised transactions, resolve entries whose transaction got a
/// category or a decided proposal, mark missed deadlines, and give every
/// open entry without a live proposal an abstaining one.
///
/// Categories are those of `snap` (before this write), as Python's local
/// copy lagged Firefly until the next sync; `categories` overrides them for
/// transactions whose new category counts as synced already.
pub fn reconcile_daily_review(
    w: &Writer<'_>,
    snap: &Snapshot,
    categories: &HashMap<i64, Option<String>>,
) -> Result<()> {
    let Some(policy) = snap.daily_review_policy.as_ref() else {
        return Ok(());
    };
    let zone = TimeZone::get(&policy.timezone).unwrap_or_else(|_| snap.zone.clone());
    let Some(activated) = parse_instant(&policy.activated_at) else {
        return Ok(());
    };
    let activated_on = activated.to_zoned(zone.clone()).date();
    let proposals: HashMap<String, CategoryProposal> = latest_proposals(&w.tx)?
        .into_iter()
        .map(|p| (identity_key(&p.source), p))
        .collect();
    let mut entries: HashMap<String, DailyReviewEntry> = {
        let mut stmt =
            w.tx.prepare("SELECT payload_json FROM daily_review_entries")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .filter_map(|p| serde_json::from_str::<DailyReviewEntry>(&p).ok())
            .map(|e| (e.transaction_id.clone(), e))
            .collect()
    };
    let now: Timestamp = w.now.timestamp();
    let stamp = w.stamp();
    for t in &snap.raw_transactions {
        let category = categories
            .get(&t.ledger_id)
            .cloned()
            .unwrap_or_else(|| t.category.clone());
        let tracked = entries.remove(&t.transaction_id);
        if t.kind == Kind::Transfer
            || (tracked.is_none()
                && (snap.daily_review_baseline.contains(&t.transaction_id)
                    || t.occurred_at.with_time_zone(zone.clone()).date() < activated_on))
        {
            continue;
        }
        let mut save = false;
        let mut entry = match tracked {
            Some(entry) => entry,
            None => {
                if category.is_some() {
                    continue;
                }
                save = true;
                let due = now
                    .checked_add(SignedDuration::from_hours(
                        24 * i64::from(policy.missed_after_days),
                    ))
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                DailyReviewEntry {
                    transaction_id: t.transaction_id.clone(),
                    batch_date: now.to_zoned(zone.clone()).date().to_string(),
                    first_seen: stamp.clone(),
                    due_at: super::utc_stamp(due),
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
        if entry.resolved_at.is_some() {
            continue;
        }
        if let Some(category) = &category {
            entry.resolved_at = Some(stamp.clone());
            entry.resolution = Some(
                if entry.auto_category.as_deref() == Some(category.as_str()) {
                    "automatic"
                } else {
                    "source"
                }
                .into(),
            );
            save = true;
        } else if entry.missed_at.is_none()
            && parse_instant(&entry.due_at).is_some_and(|due| now >= due)
        {
            entry.missed_at = Some(entry.due_at.clone());
            save = true;
        }
        if save {
            save_entry(w, &entry)?;
        }
        if entry.resolved_at.is_some() {
            continue;
        }
        let previous = proposals.get(&t.source_key());
        if let Some(previous) = previous
            && matches!(
                previous.status,
                ProposalStatus::Approved | ProposalStatus::Corrected
            )
        {
            entry.resolved_at = Some(stamp.clone());
            entry.resolution = Some(
                if entry.auto_proposal_id.as_deref() == Some(previous.proposal_id.as_str()) {
                    "automatic"
                } else {
                    "manual"
                }
                .into(),
            );
            save_entry(w, &entry)?;
            continue;
        }
        if previous.is_some_and(|p| p.status != ProposalStatus::Rejected) {
            continue;
        }
        let described = has_merchant_description(t.description.as_deref());
        let reason = if previous.is_some() {
            "suggestion_rejected"
        } else if described {
            "awaiting_model"
        } else {
            "missing_bank_description"
        };
        let previous_id = previous.map_or("", |p| p.proposal_id.as_str());
        let fingerprint = sha256_hex(&format!("daily:{}:{previous_id}", t.transaction_id));
        let merchant = if described {
            normalize_merchant(t.description.as_deref().unwrap_or(""))
        } else {
            "Missing bank description".to_owned()
        };
        record_proposal(
            w,
            &CategoryProposal {
                proposal_id: fingerprint.clone(),
                source: ProposalSource {
                    system: "firefly_iii".into(),
                    entity: "transaction".into(),
                    external_id: t.ledger_id.to_string(),
                    source_revision: None,
                    source_updated_at: None,
                },
                original_category: None,
                proposed_category: None,
                method: CategorisationMethod::Abstain,
                confidence: None,
                runner_up: None,
                runner_up_confidence: None,
                category_options: Vec::new(),
                reasons: vec![reason.into()],
                model_version: None,
                prompt_version: Some("daily-review-v1".into()),
                policy_version: "daily-review-v1".into(),
                vocabulary_version: "daily-review-v1".into(),
                feature_hash: fingerprint,
                normalized_merchant: merchant,
                status: ProposalStatus::Pending,
                created_at: stamp.clone(),
                sanity_check: Default::default(),
                extras: Default::default(),
                country_suggestions: Default::default(),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;

/// Idempotently store a proposal; only a superseded row is replaced.
fn record_proposal(w: &Writer<'_>, proposal: &CategoryProposal) -> Result<()> {
    let created = iso_now(w);
    w.put_json(
        "INSERT OR IGNORE INTO category_proposals (proposal_id, source_system, source_entity, \
         external_id, feature_hash, status, batch_id, payload_json, created_at, updated_at) \
         VALUES (?2, ?3, ?4, ?5, ?6, 'pending', NULL, ?1, ?7, ?7) \
         ON CONFLICT(proposal_id) DO UPDATE SET status = excluded.status, \
         payload_json = excluded.payload_json, updated_at = excluded.updated_at \
         WHERE category_proposals.status = 'superseded'",
        proposal,
        &[
            &proposal.proposal_id,
            &proposal.source.system,
            &proposal.source.entity,
            &proposal.source.external_id,
            &proposal.feature_hash,
            &created,
        ],
    )
}
