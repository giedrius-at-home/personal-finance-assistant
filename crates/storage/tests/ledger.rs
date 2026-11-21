use std::str::FromStr;

use finance_storage::{
    Ledger, LedgerAccount, LedgerCategory, LedgerPosting, LedgerTransaction, Store,
};

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
