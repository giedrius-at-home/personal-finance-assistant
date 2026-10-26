use super::*;
use crate::writes::fixture::db;
use finance_types::gateway::OwnerTag;
use serde_json::json;

fn change(id: &str, expected: Option<&str>, category: &str) -> CategoryChange {
    CategoryChange {
        transaction_id: id.into(),
        expected_category: expected.map(Into::into),
        category: category.into(),
    }
}

#[test]
fn category_change_checks_what_the_owner_saw() {
    let mut db = db();
    db.entry(10, json!({}));
    let saved = db
        .write(|w, s| set_transaction_category(w, s, change("10", None, " Eating out ")))
        .expect("saved");
    assert_eq!(saved.category, "Eating out");
    assert_eq!(
        db.one::<i64>("SELECT category_id FROM transactions WHERE id = 10"),
        2
    );
    assert_eq!(
        db.one::<i64>(
            "SELECT rowid FROM transactions_search WHERE transactions_search MATCH '\"Eating out\"'"
        ),
        10
    );
    // The daily review sees the new category as the bank's.
    assert!(
        db.one::<String>(
            "SELECT payload_json FROM daily_review_entries WHERE transaction_id = 'firefly:10'"
        )
        .contains("\"resolution\":\"source\"")
    );
    // Stale: the owner saw it uncategorised.
    let err = db
        .write(|w, s| set_transaction_category(w, s, change("10", None, "Groceries")))
        .expect_err("stale");
    assert_eq!(err.to_string(), CATEGORY_CHANGED);
    // The canonical id works too; a new name creates the category.
    db.write(|w, s| {
        set_transaction_category(w, s, change("firefly:10", Some("Eating out"), "Pets"))
    })
    .expect("saved");
    assert_eq!(
        db.one::<String>("SELECT name FROM categories WHERE id = (SELECT category_id FROM transactions WHERE id = 10)"),
        "Pets"
    );
    assert!(
        db.write(|w, s| set_transaction_category(w, s, change("10", Some("Pets"), "  ")))
            .is_err()
    );
}

#[test]
fn consolidation_moves_a_category_after_a_matching_preview() {
    let mut db = db();
    let request = Consolidation {
        source: "Groceries".into(),
        target: "Eating out".into(),
        token: None,
    };
    // "Eating out" is not in use yet.
    assert!(preview_consolidation(&db.snap(), &request).is_err());
    db.write(|w, s| set_transaction_category(w, s, change("10", None, "Eating out")))
        .expect("saved");
    let preview = preview_consolidation(&db.snap(), &request).expect("preview");
    assert_eq!(preview.count, 2);
    let wrong = Consolidation {
        token: Some("0".repeat(64)),
        ..request.clone()
    };
    assert!(db.write(|w, s| apply_consolidation(w, s, wrong)).is_err());
    let applied = db
        .write(|w, s| {
            apply_consolidation(
                w,
                s,
                Consolidation {
                    token: Some(preview.token),
                    ..request
                },
            )
        })
        .expect("applied");
    assert_eq!((applied.applied, applied.complete), (2, true));
    assert_eq!(
        db.one::<i64>("SELECT COUNT(*) FROM transactions WHERE category_id = 2"),
        3
    );
}

#[test]
fn personal_list_shows_memo_cues_and_searches() {
    let db = db();
    let list = personal_transactions(&db.snap(), "").expect("list");
    let ids: Vec<_> = list
        .items
        .iter()
        .map(|i| i.transaction_id.as_str())
        .collect();
    assert_eq!(ids, ["11"]);
    assert!(list.items[0].incoming);
    assert_eq!(list.since, "2026-09-01");
    let found = personal_transactions(&db.snap(), "LIDL").expect("search");
    assert_eq!(found.items[0].transaction_id, "10");
    assert!(personal_transactions(&db.snap(), &"x".repeat(129)).is_err());
}

fn choice(role: &str, category: &str, revision: u32) -> PersonalChoice {
    PersonalChoice {
        transaction_id: "11".into(),
        revision,
        role: role.into(),
        category: category.into(),
        expected_amount: "30.00".into(),
        expected_currency: "EUR".into(),
    }
}

#[test]
fn personal_choice_validates_direction_amount_and_revision() {
    let mut db = db();
    let saved = db
        .write(|w, s| save_personal_choice(w, s, choice("return", "Groceries", 0)))
        .expect("saved");
    assert_eq!(saved.revision, 1);
    let details: OwnerDetails = serde_json::from_str(&db.one::<String>(
        "SELECT payload_json FROM transaction_details WHERE source_key = '[\"firefly_iii\", \"transaction\", \"11\"]'",
    ))
    .expect("details");
    assert_eq!(
        (
            details.personal_expense_role.as_str(),
            details.personal_expense_category.as_str()
        ),
        ("return", "Groceries")
    );
    assert_eq!(
        db.one::<String>(
            "SELECT group_name FROM spending_group_history WHERE category = 'Groceries'"
        ),
        "Groceries"
    );
    // Stale revision, wrong direction, changed amount, missing category.
    let err = db
        .write(|w, s| save_personal_choice(w, s, choice("ignore", "", 0)))
        .expect_err("stale");
    assert_eq!(err.to_string(), PERSONAL_CHANGED);
    let err = db
        .write(|w, s| save_personal_choice(w, s, choice("payment", "Groceries", 1)))
        .expect_err("direction");
    assert_eq!(err.to_string(), PERSONAL_INVALID);
    let mut moved = choice("ignore", "", 1);
    moved.expected_amount = "31.00".into();
    assert!(db.write(|w, s| save_personal_choice(w, s, moved)).is_err());
    assert!(
        db.write(|w, s| save_personal_choice(w, s, choice("return", " ", 1)))
            .is_err()
    );
    let cleared = db
        .write(|w, s| save_personal_choice(w, s, choice("clear", "Groceries", 1)))
        .expect("cleared");
    assert_eq!(cleared.revision, 2);
    assert!(
        db.one::<String>("SELECT payload_json FROM transaction_details")
            .contains("\"personal_expense_role\":\"\"")
    );
}

const P1: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn details(revision: u32, values: OwnerDetails) -> DetailsSave {
    DetailsSave {
        proposal_id: P1.into(),
        revision,
        values,
    }
}

#[test]
fn details_are_validated_and_revisioned() {
    let mut db = db();
    db.proposal(P1, 10, Some("Groceries"));
    let values = OwnerDetails {
        merchant_name: "Lidl".into(),
        to_country: "NL".into(),
        tags: vec![OwnerTag {
            label: " Nachos ".into(),
            parent: "Food".into(),
        }],
        bulk_payment_role: "component".into(),
        bulk_payment_name: " Klarna September ".into(),
        ..OwnerDetails::default()
    };
    let saved = db
        .write(|w, _| save_review_details(w, details(0, values.clone())))
        .expect("saved");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.values.tags[0].label, "Nachos");
    assert_eq!(saved.values.bulk_payment_name, "Klarna September");
    let err = db
        .write(|w, _| save_review_details(w, details(0, values.clone())))
        .expect_err("stale");
    assert!(err.to_string().starts_with("Details changed elsewhere"));
    let unknown = OwnerDetails {
        to_country: "XX".into(),
        ..values.clone()
    };
    assert!(
        db.write(|w, _| save_review_details(w, details(1, unknown)))
            .is_err()
    );
    let half_bulk = OwnerDetails {
        bulk_payment_name: String::new(),
        ..values
    };
    let err = db
        .write(|w, _| save_review_details(w, details(1, half_bulk)))
        .expect_err("bulk");
    assert!(err.to_string().starts_with("Check the details"));
}

#[test]
fn audit_marks_only_automatic_categories() {
    let mut db = db();
    db.entry(
        12,
        json!({"resolved_at": "2026-10-22T10:00:00Z", "resolution": "automatic", "auto_category": "Groceries"}),
    );
    db.entry(
        13,
        json!({"resolved_at": "2026-10-22T10:00:00Z", "resolution": "manual"}),
    );
    db.write(|w, _| audit_daily_review(w, "firefly:12"))
        .expect("audited");
    assert!(
        db.one::<String>(
            "SELECT payload_json FROM daily_review_entries WHERE transaction_id = 'firefly:12'"
        )
        .contains("\"audited_at\":\"2026-10-26T20:00:00")
    );
    assert!(
        db.write(|w, _| audit_daily_review(w, "firefly:13"))
            .is_err()
    );
    assert!(
        db.write(|w, _| audit_daily_review(w, "firefly:99"))
            .is_err()
    );
}
