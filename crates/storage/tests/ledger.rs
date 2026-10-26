use std::str::FromStr;

use finance_storage::{
    Ledger, LedgerAccount, LedgerCategory, LedgerPosting, LedgerTransaction, Store,
};
use finance_types::{TxKind, TxQuery};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn account(id: i64, name: &str, kind: &str) -> LedgerAccount {
    LedgerAccount {
        id,
        name: name.into(),
        kind: kind.into(),
        iban: (kind == "asset").then(|| "LT000000000000001234".into()),
        active: true,
        role: None,
        include_net_worth: true,
    }
}

fn tx(
    id: i64,
    kind: &str,
    date: &str,
    description: &str,
    category: Option<i64>,
) -> LedgerTransaction {
    LedgerTransaction {
        id,
        group_id: Some(id),
        kind: kind.into(),
        booked_at: format!("{date}T12:00:00"),
        description: description.into(),
        category_id: category,
        bill_id: None,
        external_id: None,
        internal_reference: None,
        notes: None,
        tags: vec![],
    }
}

fn posting(id: i64, tx: i64, account: i64, amount: &str) -> LedgerPosting {
    LedgerPosting {
        id,
        transaction_id: tx,
        account_id: account,
        amount: dec(amount),
        currency: "EUR".into(),
        foreign_amount: None,
        foreign_currency: None,
        reconciled: false,
    }
}

/// Checking (1) and savings (2); a shop (3) and an employer (4).
fn store() -> (Store, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "finance-test-{}-{:?}.sqlite3",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut store = Store::open(&path).unwrap();
    let ledger = Ledger {
        accounts: vec![
            account(1, "Checking", "asset"),
            account(2, "Savings", "asset"),
            account(3, "Rimi", "expense"),
            account(4, "Employer", "revenue"),
        ],
        categories: vec![LedgerCategory {
            id: 10,
            name: "Groceries".into(),
        }],
        bills: vec![],
        transactions: vec![
            tx(100, "deposit", "2026-09-25", "Salary", None),
            tx(101, "withdrawal", "2026-10-02", "Rimi Vilnius", Some(10)),
            tx(102, "withdrawal", "2026-10-03", "Coffee", None),
            tx(103, "transfer", "2026-10-05", "To savings", None),
        ],
        postings: vec![
            posting(1, 100, 4, "-2500.00"),
            posting(2, 100, 1, "2500.00"),
            posting(3, 101, 1, "-42.10"),
            posting(4, 101, 3, "42.10"),
            posting(5, 102, 1, "-3.50"),
            posting(6, 102, 3, "3.50"),
            posting(7, 103, 1, "-500"),
            posting(8, 103, 2, "500"),
        ],
    };
    store.replace_ledger(&ledger, "test").unwrap();
    (store, path)
}

#[test]
fn balances_are_exact_sums_of_postings() {
    let (store, path) = store();
    let accounts = store.accounts().unwrap();
    let balance = |name: &str| accounts.iter().find(|a| a.name == name).unwrap().balances[0].amount;
    assert_eq!(balance("Checking"), dec("1954.4"));
    assert_eq!(balance("Savings"), dec("500"));
    assert!(
        accounts.iter().all(|a| a.kind.is_own()),
        "shops are not own accounts"
    );
    assert_eq!(store.overview().unwrap().net_worth[0].amount, dec("2454.4"));
    std::fs::remove_file(path).ok();
}

#[test]
fn months_split_income_and_spending_and_skip_transfers() {
    let (store, path) = store();
    let months = store.overview().unwrap().months;
    let oct = months.iter().find(|m| m.month == "2026-10").unwrap();
    assert_eq!((oct.income, oct.spending), (Decimal::ZERO, dec("45.6")));
    let sep = months.iter().find(|m| m.month == "2026-09").unwrap();
    assert_eq!(sep.income, dec("2500"));
    std::fs::remove_file(path).ok();
}

#[test]
fn filters_sign_amounts_from_the_owner_side() {
    let (store, path) = store();
    let all = store
        .transactions(&TxQuery {
            limit: 50,
            ..TxQuery::default()
        })
        .unwrap();
    assert_eq!(all.total, 4);
    assert_eq!(all.rows[0].id, 103, "newest first");
    assert_eq!(
        all.sums[0].amount,
        dec("2454.4"),
        "transfers between own accounts cancel out"
    );
    let coffee = all.rows.iter().find(|r| r.id == 102).unwrap();
    assert_eq!(coffee.amount.amount, dec("-3.5"));

    // From the savings account's side the transfer is money in.
    let savings = store
        .transactions(&TxQuery {
            account_id: Some(2),
            limit: 50,
            ..TxQuery::default()
        })
        .unwrap();
    assert_eq!(savings.total, 1);
    assert_eq!(savings.sums[0].amount, dec("500"));

    let spending = store
        .transactions(&TxQuery {
            kind: Some(TxKind::Withdrawal),
            from: Some("2026-10-01".into()),
            limit: 50,
            ..TxQuery::default()
        })
        .unwrap();
    assert_eq!(spending.total, 2);
    assert_eq!(spending.sums[0].amount, dec("-45.6"));

    let uncategorised = store
        .transactions(&TxQuery {
            uncategorised: true,
            limit: 50,
            ..TxQuery::default()
        })
        .unwrap();
    assert_eq!(
        uncategorised.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        [102]
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn search_matches_substrings_of_description_counterparty_and_category() {
    let (store, path) = store();
    let search = |text: &str| {
        store
            .transactions(&TxQuery {
                text: Some(text.into()),
                limit: 50,
                ..TxQuery::default()
            })
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(search("vilni"), [101]);
    assert_eq!(search("grocer"), [101], "category name");
    assert_eq!(search("employ"), [100], "counterparty");
    assert_eq!(search("Co"), [102], "short text falls back to LIKE");
    assert!(search("nothing like this").is_empty());
    std::fs::remove_file(path).ok();
}

#[test]
fn category_totals_include_uncategorised_spending() {
    let (store, path) = store();
    let totals = store.category_totals("2026-10").unwrap();
    assert_eq!(totals.len(), 2);
    assert_eq!(totals[0].category.as_ref().unwrap().name, "Groceries");
    assert_eq!(totals[0].amount, dec("42.1"));
    assert!(totals[1].category.is_none());
    std::fs::remove_file(path).ok();
}

#[test]
fn reimport_replaces_instead_of_duplicating() {
    let (mut store, path) = store();
    let before = store.account_checks().unwrap();
    store.replace_ledger(&Ledger::default(), "empty").unwrap();
    assert!(store.account_checks().unwrap().is_empty());
    assert_eq!(before.iter().map(|c| c.postings).sum::<u64>(), 8);
    std::fs::remove_file(path).ok();
}
