use super::*;
use crate::writes::fixture::db;
use serde_json::json;

const P1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const P2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn decision(id: &str, action: ReviewAction, category: Option<&str>) -> ReviewDecision {
    ReviewDecision {
        proposal_id: id.into(),
        decision: action,
        category: category.map(Into::into),
        expected_proposed_category: Some("Groceries".into()),
        expected_original_category: None,
    }
}

fn status(db: &crate::writes::fixture::Db, id: &str) -> String {
    db.one(&format!(
        "SELECT status FROM category_proposals WHERE proposal_id = '{id}'"
    ))
}

#[test]
fn approve_writes_the_ledger_and_resolves_the_daily_entry() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    db.entry(10, json!({}));
    let saved = db
        .write(|w, s| {
            review_decisions(
                w,
                s,
                vec![decision(P1, ReviewAction::Approve, Some("Groceries"))],
            )
        })
        .expect("saved");
    assert_eq!(saved.saved, 1);
    assert_eq!(
        db.one::<i64>("SELECT category_id FROM transactions WHERE id = 10"),
        1
    );
    assert_eq!(status(&db, P1), "approved");
    assert_eq!(db.one::<i64>("SELECT COUNT(*) FROM category_batches"), 1);
    let entry: DailyReviewEntry = serde_json::from_str(&db.one::<String>(
        "SELECT payload_json FROM daily_review_entries WHERE transaction_id = 'firefly:10'",
    ))
    .expect("entry");
    assert_eq!(entry.resolution.as_deref(), Some("manual"));
    // The search index knows the new category.
    assert_eq!(
        db.one::<i64>(
            "SELECT rowid FROM transactions_search WHERE transactions_search MATCH '\"Groceries\"'"
        ),
        10
    );
    // Uncategorised transaction 11 is now tracked, with an abstaining proposal.
    assert_eq!(
        db.one::<i64>(
            "SELECT COUNT(*) FROM daily_review_entries WHERE transaction_id = 'firefly:11'"
        ),
        1
    );
    assert!(
        db.one::<String>("SELECT payload_json FROM category_proposals WHERE external_id = '11'")
            .contains("awaiting_model")
    );
}

#[test]
fn a_changed_proposal_is_dropped_without_writes() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    let mut stale = decision(P1, ReviewAction::Approve, Some("Groceries"));
    stale.expected_proposed_category = Some("Rent".into());
    let saved = db
        .write(|w, s| review_decisions(w, s, vec![stale]))
        .expect("result");
    assert_eq!(saved.saved, 0);
    assert_eq!(saved.results[0].status, DecisionStatus::Changed);
    assert_eq!(
        db.one::<Option<i64>>("SELECT category_id FROM transactions WHERE id = 10"),
        None
    );
    // A bank category other than the one the proposal saw is a conflict too.
    db.store
        .connection()
        .execute("UPDATE transactions SET category_id = 2 WHERE id = 10", [])
        .expect("update");
    let saved = db
        .write(|w, s| {
            review_decisions(
                w,
                s,
                vec![decision(P1, ReviewAction::Approve, Some("Groceries"))],
            )
        })
        .expect("result");
    assert_eq!(saved.results[0].status, DecisionStatus::Changed);
    assert_eq!(status(&db, P1), "pending");
}

#[test]
fn reject_defer_and_correct_follow_python() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    db.proposal(P2, 11, Some("Groceries"));
    db.entry(10, json!({}));
    db.write(|w, s| {
        review_decisions(
            w,
            s,
            vec![
                decision(P1, ReviewAction::Reject, None),
                decision(P2, ReviewAction::Defer, None),
            ],
        )
    })
    .expect("saved");
    assert_eq!(status(&db, P1), "rejected");
    // A rejected suggestion gets a fresh abstaining proposal.
    assert!(
        db.one::<String>(
            "SELECT payload_json FROM category_proposals WHERE external_id = '10' AND status = 'pending'"
        )
        .contains("suggestion_rejected")
    );
    assert_eq!(
        db.one::<String>("SELECT source_key FROM review_deferrals"),
        "[\"firefly_iii\", \"transaction\", \"11\"]"
    );
    // Correcting resumes it; the spelling snaps to the one in use.
    let saved = db
        .write(|w, s| {
            review_decisions(
                w,
                s,
                vec![decision(P2, ReviewAction::Correct, Some("  groceries "))],
            )
        })
        .expect("saved");
    assert_eq!(saved.saved, 1);
    assert_eq!(
        db.one::<i64>("SELECT category_id FROM transactions WHERE id = 11"),
        1
    );
    assert_eq!(db.one::<i64>("SELECT COUNT(*) FROM review_deferrals"), 0);
    assert_eq!(
        db.one::<String>("SELECT approved_category FROM category_corrections"),
        "Groceries"
    );
    assert_eq!(status(&db, P2), "corrected");
}

#[test]
fn correcting_to_a_new_name_creates_the_category() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    db.write(|w, s| {
        review_decisions(
            w,
            s,
            vec![decision(P1, ReviewAction::Correct, Some("Pet food"))],
        )
    })
    .expect("saved");
    assert_eq!(
        db.one::<String>(
            "SELECT c.name FROM transactions t JOIN categories c ON c.id = t.category_id WHERE t.id = 10"
        ),
        "Pet food"
    );
}

#[test]
fn invalid_streams_are_rejected() {
    let mut db = db();
    for decisions in [
        vec![],
        vec![decision("nope", ReviewAction::Reject, None)],
        vec![decision(P1, ReviewAction::Approve, None)],
        vec![decision(P1, ReviewAction::Reject, Some("Groceries"))],
        vec![
            decision(P1, ReviewAction::Reject, None),
            decision(P1, ReviewAction::Defer, None),
        ],
    ] {
        assert!(db.write(|w, s| review_decisions(w, s, decisions)).is_err());
    }
}

fn batch(id: &str, category: &str) -> BatchDecision {
    BatchDecision {
        proposal_id: id.into(),
        category: category.into(),
        expected_proposed_category: "Groceries".into(),
        expected_original_category: None,
    }
}

#[test]
fn batch_saves_all_or_nothing() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    db.proposal(P2, 11, Some("Groceries"));
    let mut stale = batch(P2, "Eating out");
    stale.expected_proposed_category = "Rent".into();
    let err = db
        .write(|w, s| review_batch(w, s, vec![batch(P1, "Groceries"), stale]))
        .expect_err("changed");
    assert_eq!(err.to_string(), BATCH_CHANGED);
    assert_eq!(
        db.one::<i64>("SELECT COUNT(*) FROM transactions WHERE category_id IS NOT NULL"),
        2
    );
    assert!(
        db.write(|w, s| review_batch(w, s, vec![batch(P1, "Groceries"), batch(P1, "x")]))
            .is_err()
    );
    let saved = db
        .write(|w, s| review_batch(w, s, vec![batch(P1, "Groceries"), batch(P2, "Eating out")]))
        .expect("saved");
    assert_eq!(saved.saved, 2);
    assert_eq!(status(&db, P1), "approved");
    assert_eq!(status(&db, P2), "corrected");
    assert_eq!(
        db.one::<i64>("SELECT category_id FROM transactions WHERE id = 11"),
        2
    );
}

#[test]
fn merchants_are_normalized_like_python() {
    assert_eq!(
        normalize_merchant(
            "BEA, Betaalpas  https://www.Shop.nl/x?y=1 NL91ABNA0417164300 nr: 123456 a@b.nl"
        ),
        "bea, betaalpas shop.nl account nr: number email"
    );
    assert!(!has_merchant_description(Some(" (No description) ")));
    assert!(has_merchant_description(Some("Lidl")));
}
