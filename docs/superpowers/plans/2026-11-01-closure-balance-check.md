# Closure Balance Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Before a month's split can be saved, the owner confirms every spending/savings account's balance on the closure screen; a typed difference is booked as a reconciliation entry that never counts as income or spending.

**Architecture:** One new closure action (`ConfirmBalance`) stores a per-account `BalanceCheck` in the month's `MonthPeriod` payload and books the difference through the reconciliation helper bank sync already uses. The closure view computes a `BalanceStatus` per account (ledger figure, last bank figure, check state); `settle` refuses while any status is not `Confirmed`. The Leptos closure page gets a Balances panel on the Sync step and locks the Split step until every row is confirmed.

**Tech Stack:** Rust 2024, rusqlite, serde, rust_decimal, jiff; Leptos (CSR, Tauri `invoke`) for the UI. Tests with `cargo test -p finance-core` and `cargo check -p finance-ui`.

**Spec:** `docs/superpowers/specs/2026-11-01-closure-balance-check-design.md`

## Global Constraints

- `MonthPeriod` keeps `#[serde(deny_unknown_fields)]`; the new field has `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]` so the existing exact-JSON round-trip test stays byte-identical.
- Money is `rust_decimal::Decimal`; a typed balance is refused when it has more decimals than the account currency's `minor_unit`.
- Reconciliation entries use transaction kind `"reconciliation"` and the counterparty account named `"{account} reconciliation ({currency})"` of kind `"reconciliation"`, exactly as `reconcile_bank_account` does today.
- Error texts, verbatim: `"month closure changed; refresh"` (stale revision), `"a later month is already closed"`, `"confirm every account balance first: {names}"` (names joined by `", "`, in review order), `"invalid balance"`, `"account cannot be confirmed"`.
- Audit: a booked entry records `w.record("reconciliation", {"account_id": <ledger id>, "difference": <string>})`, same shape as bank sync.
- Only accounts that are active, have a role of `spending` or `savings`, and appear in `review.accounts` are checked. Ignored, unclassified and inactive accounts are neither listed nor gated.
- Commit after every task; commit messages in the repo's style (`feat(closure): …`, `test(closure): …`), each ending with the attribution line the session gives.

## Review Focus

1. A balance typed with more decimals than the currency allows (e.g. `12.345` EUR) must be refused with `"invalid balance"`, not rounded silently. → Task 3 test `confirm_refuses_more_decimals_than_the_currency`.
2. A negative balance (an overdrawn account) must be accepted and booked, since liabilities and overdrafts are real. → Task 3 test `confirm_books_a_negative_balance`.
3. A confirm sent with a revision the screen read before another save must be refused with `"month closure changed; refresh"` and book nothing. → Task 3 test `confirm_with_a_stale_revision_books_nothing`.
4. A reconciliation made on the Banks screen after a confirm must turn the check `Stale`; confirming again at the ledger figure must book nothing and leave the earlier entry gone. → Task 4 test `banks_screen_reconciliation_makes_the_check_stale`.
5. "Confirm all matching" that fails on the second row must stop, show the error, and leave the first row confirmed (the server saved it). → Task 5, manual check listed in its verification step; the server side is covered by Task 3's per-row semantics.

---

### Task 1: Types

**Files:**
- Modify: `crates/types/src/report/closure.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct BalanceCheck {
      pub balance: Decimal,
      pub ledger_balance: Decimal,
      pub confirmed_at: String,
      #[serde(default)] pub transaction_id: Option<String>,
  }
  // on MonthPeriod:
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub balance_checks: BTreeMap<String, BalanceCheck>,   // account_id → check
  // new ClosureAction variant (serde tag "confirm_balance"):
  ConfirmBalance { month: String, revision: u32, account_id: String, balance: Decimal },
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum BalanceState { Unconfirmed, Confirmed, Stale }
  pub struct BalanceStatus {
      pub account_id: String, pub name: String, pub currency: Currency,
      pub ledger: Decimal,
      pub bank: Option<Decimal>, pub bank_reported_on: Option<String>,
      pub check: Option<BalanceCheck>,
      pub state: BalanceState,
      pub adjusted_by: Option<Decimal>,
  }
  // on ClosureView:
  #[serde(default)] pub balances: Vec<BalanceStatus>,
  ```
  `ClosureAction::month()` covers the new variant. `BalanceCheck` derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize` and `#[serde(deny_unknown_fields)]`.

- [ ] **Step 1: Write the failing tests** in the `tests` module of `crates/types/src/report/closure.rs`

```rust
#[test]
fn balance_checks_round_trip_and_stay_out_of_an_empty_payload() {
    let json = r#"{"month":"2026-10-01","starts_at":"2026-10-01T00:00:00+02:00","cutoff_at":null,"status":"settling","settled_at":null,"settled_snapshot":null,"missing_banks":[],"allocations":[],"revision":2,"reviewed_through":null,"reopened_at":null,"balance_checks":{"firefly:1":{"balance":"1234.56","ledger_balance":"1234.56","confirmed_at":"2026-11-01T10:00:00Z","transaction_id":"firefly:77"}}}"#;
    let period: MonthPeriod = serde_json::from_str(json).unwrap();
    assert_eq!(period.balance_checks["firefly:1"].transaction_id.as_deref(), Some("firefly:77"));
    assert_eq!(serde_json::to_string(&period).unwrap(), json);
    let empty = MonthPeriod { balance_checks: BTreeMap::new(), ..period };
    assert!(!serde_json::to_string(&empty).unwrap().contains("balance_checks"));
}

#[test]
fn confirm_balance_action_parses() {
    let action: ClosureAction = serde_json::from_str(
        r#"{"action":"confirm_balance","month":"2026-10-01","revision":2,"account_id":"firefly:1","balance":"270.00"}"#,
    ).unwrap();
    assert!(matches!(action, ClosureAction::ConfirmBalance { ref account_id, .. } if account_id == "firefly:1"));
    assert_eq!(action.month(), "2026-10-01");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-types balance_checks confirm_balance_action`
Expected: compile error (no `balance_checks`, no `ConfirmBalance`).

- [ ] **Step 3: Add the types listed under Interfaces**

Keep `stored_period_round_trips` unchanged; it must still pass.

- [ ] **Step 4: Run the crate's tests**

Run: `cargo test -p finance-types`
Expected: all pass, including `stored_period_round_trips`.

- [ ] **Step 5: Commit**

```bash
git add crates/types/src/report/closure.rs
git commit -m "feat(closure): balance check types and the confirm_balance action"
```

---

### Task 2: Shared reconciliation booking and single-transaction delete

**Files:**
- Modify: `crates/storage/src/ledger.rs` (add `delete_transaction`)
- Modify: `crates/core/src/bank_sync/mod.rs` (extract `book_reconciliation`, make `ledger_balance` `pub(crate)`)

**Interfaces:**
- Produces:
  ```rust
  // finance_storage::ledger
  /// Remove one transaction with its postings and search row; Ok(false) when absent.
  pub fn delete_transaction(conn: &Connection, id: i64) -> Result<bool>;

  // finance_core::bank_sync
  pub(crate) fn ledger_balance(conn: &Sql, account: i64, currency: &str) -> crate::Result<Decimal>;
  /// Book a reconciliation entry that brings `account_id`'s ledger balance in
  /// `currency` to `target`. Returns the new transaction id, or None when the
  /// ledger already matches. Records the audit row.
  pub(crate) fn book_reconciliation(
      w: &Writer<'_>, account_id: i64, account_name: &str, currency: &str,
      target: Decimal, description: &str,
  ) -> crate::Result<Option<i64>>;
  ```
  `reconcile_bank_account` calls `book_reconciliation` and keeps its current error `"The ledger already matches the bank."` when `None` comes back.

- [ ] **Step 1: Write the failing tests** in `crates/core/src/bank_sync/mod.rs` `tests` (they already build a `Store` from `writes/test_fixture.sql`; account 1 "Checking" is an asset with EUR postings)

```rust
#[test]
fn book_reconciliation_moves_the_ledger_to_the_target_and_delete_undoes_it() {
    let mut db = crate::writes::fixture::db();   // in-memory Store on test_fixture.sql
    let conn = db.store.connection();
    let before = ledger_balance(conn, 1, "EUR").unwrap();
    let id = db.write(|w, _| book_reconciliation(w, 1, "Checking", "EUR", before + d("40"), "test"))
        .unwrap().expect("booked");
    let conn = db.store.connection();
    assert_eq!(ledger_balance(&conn, 1, "EUR").unwrap(), before + d("40"));
    let kind: String = conn.query_row("SELECT kind FROM transactions WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(kind, "reconciliation");
    assert!(finance_storage::delete_transaction(&conn, id).unwrap());
    assert_eq!(ledger_balance(&conn, 1, "EUR").unwrap(), before);
    let left: i64 = conn.query_row("SELECT COUNT(*) FROM postings WHERE transaction_id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
    assert!(!finance_storage::delete_transaction(&conn, id).unwrap());
}

#[test]
fn book_reconciliation_books_nothing_when_the_ledger_matches() {
    // same setup; target == current ledger → Ok(None) and no new transactions row
}
```
`crate::writes::fixture::Db::write` runs a closure in one committed `Writer`; `d` is `crate::closure::tests::d`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core bank_sync::tests::book_reconciliation`
Expected: compile error (`book_reconciliation`, `delete_transaction` missing).

- [ ] **Step 3: Implement `delete_transaction` in `crates/storage/src/ledger.rs`**

Three statements in order: `DELETE FROM transactions_search WHERE rowid = ?1`, `DELETE FROM postings WHERE transaction_id = ?1`, `DELETE FROM transactions WHERE id = ?1`; return whether the last changed a row. Export it from `crates/storage/src/lib.rs` next to `insert_transaction`.

- [ ] **Step 4: Extract `book_reconciliation` from `reconcile_bank_account`**

Move the body from `let ledger = ledger_balance(...)` through `w.record(...)` into the new function; `reconcile_bank_account` passes its description string and maps `None` to the existing "already matches" error.

- [ ] **Step 5: Run the module's tests**

Run: `cargo test -p finance-core bank_sync`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/storage/src/ledger.rs crates/storage/src/lib.rs crates/core/src/bank_sync/mod.rs
git commit -m "refactor(bank-sync): share the reconciliation booking; delete one transaction"
```

---

### Task 3: The `confirm_balance` write

**Files:**
- Modify: `crates/core/src/writes/closure.rs` (new `confirm_balance`, `act` arm)
- Modify: `crates/core/src/writes/closure/tests.rs`

**Interfaces:**
- Consumes: `book_reconciliation`, `ledger_balance`, `delete_transaction` (Task 2); `BalanceCheck`, `ClosureAction::ConfirmBalance` (Task 1); existing `period`, `require`, `require_no_later_closure`, `save`, `stamp`, `load_roles`.
- Produces:
  ```rust
  pub fn confirm_balance(
      w: &Writer<'_>, snap: &Snapshot, month: Date, revision: u32,
      account_id: &str, balance: Decimal,
  ) -> Result<()>;
  /// Accounts a month's balance check covers, in review order: active, role spending or savings.
  pub(crate) fn checked_accounts<'a>(snap: &'a Snapshot, roles: &[AccountRole]) -> Vec<&'a Account>;
  ```
  Semantics (from the spec): allowed statuses `Open`, `Settling`, `Settled`; stale revision → `CHANGED`; later closed month refused; account not in `checked_accounts` → `"account cannot be confirmed"`; `balance.scale() > currency.minor_unit` → `"invalid balance"`. Delete the earlier check's transaction (if any), then book at `balance`, store `BalanceCheck { balance, ledger_balance: balance, confirmed_at: stamp(w), transaction_id: id.map(|i| format!("firefly:{i}")) }`, save (revision bumps). Description of the entry: `format!("Balance confirmed at {balance} {code} for {YYYY-MM} closure")`.

- [ ] **Step 1: Write the failing tests** in `crates/core/src/writes/closure/tests.rs`. These need a real ledger, so use `crate::writes::fixture::db()` (in-memory `Store` with `test_fixture.sql`: account `firefly:1` "Checking", EUR) and its `write` helper; put a `spending` role for `firefly:1` with `put_role`, and a `period("2026-10-01", Settling)` with `put`.

```rust
#[test]
fn confirm_books_the_difference_and_a_retype_replaces_it() {
    // ledger of firefly:1 = L. confirm L+40 → one reconciliation tx, check.transaction_id = Some
    // confirm L-10 → the first tx is gone, exactly one reconciliation tx exists, ledger = L-10
    // confirm L → no reconciliation tx left, check.transaction_id = None, ledger = L
    // revision went 1 → 2 → 3 → 4
}

#[test]
fn confirm_refuses_more_decimals_than_the_currency() {
    // balance "12.345" → error "invalid balance"; no transaction row added
}

#[test]
fn confirm_books_a_negative_balance() {
    // balance "-25.00" → ledger_balance(firefly:1) == -25.00
}

#[test]
fn confirm_with_a_stale_revision_books_nothing() {
    // period revision 1, send revision 0 → error "month closure changed; refresh"; transactions count unchanged
}

#[test]
fn confirm_refuses_closed_months_and_unchecked_accounts() {
    // status Closed → error "month is closed"
    // role ignored for firefly:1 → error "account cannot be confirmed"
}
```
Assertions use `stored(conn, "2026-10-01").balance_checks["firefly:1"]`, `ledger_balance`, and `SELECT COUNT(*) FROM transactions WHERE kind = 'reconciliation'`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core writes::closure::tests::confirm_`
Expected: compile error (`confirm_balance` missing).

- [ ] **Step 3: Implement `checked_accounts` and `confirm_balance`; add the `act` arm**

Parse the ledger id from `account_id` via the `Account.ledger_id` of the matched snapshot account. Currency is `account.currency()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p finance-core writes::closure`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/writes/closure.rs crates/core/src/writes/closure/tests.rs
git commit -m "feat(closure): confirm an account balance with a reconciliation entry"
```

---

### Task 4: Balance statuses in the view and the settle gate

**Files:**
- Modify: `crates/core/src/closure.rs` (`balance_statuses`, `closure_view`)
- Modify: `crates/core/src/writes/closure.rs` (`settle` gate)
- Modify: `crates/core/src/writes/closure/tests.rs`, `crates/core/src/closure/tests.rs`

**Interfaces:**
- Consumes: `checked_accounts` (Task 3); `bank_sync::runs(conn)`, `bank_sync::ledger_balance`; `BalanceStatus`, `BalanceState` (Task 1).
- Produces:
  ```rust
  /// One status per checked account, in review order.
  pub fn balance_statuses(
      conn: &Connection, snap: &Snapshot, period: &MonthPeriod, roles: &[AccountRole],
  ) -> Result<Vec<BalanceStatus>>;
  /// Names of checked accounts whose status is not Confirmed (empty = may settle).
  pub(crate) fn unconfirmed_names(statuses: &[BalanceStatus]) -> Vec<String>;
  ```
  `state`: no check → `Unconfirmed`; check and `ledger == check.ledger_balance` → `Confirmed`; otherwise `Stale`. `bank`/`bank_reported_on`: from the `AccountBalance` across all `runs` whose `account_id == account.ledger_id` and `currency == account.currency().code`, else `None`. `adjusted_by`: the booked entry's posting on the account (`SELECT amount FROM postings WHERE transaction_id = ?1 AND account_id = ?2`), `None` when the check has no transaction. `closure_view` fills `ClosureView.balances`. `settle` calls `balance_statuses` after `require(...)` and refuses with `"confirm every account balance first: {names}"` when `unconfirmed_names` is non-empty.

- [ ] **Step 1: Write the failing tests**

In `crates/core/src/writes/closure/tests.rs` (fixture db, as Task 3):
```rust
#[test]
fn settle_needs_every_account_confirmed() {
    // Settling period, role spending on firefly:1, no check → settle error
    //   "confirm every account balance first: Checking"
    // confirm at the ledger figure → settle succeeds
}

#[test]
fn banks_screen_reconciliation_makes_the_check_stale() {
    // confirm L+40 → statuses()[0].state == Confirmed, adjusted_by == Some(40)
    // book_reconciliation(w, 1, "Checking", "EUR", L+50, "banks") → state == Stale
    // settle → refused naming Checking
    // confirm at L+50 → Confirmed, transaction_id None (ledger already matched), only the banks entry remains
}
```
In `crates/core/src/closure/tests.rs`, a pure test on `unconfirmed_names` with three hand-built statuses (Confirmed, Stale, Unconfirmed) expecting the two names in order.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core settle_needs_every_account_confirmed banks_screen_reconciliation unconfirmed_names`
Expected: compile errors.

- [ ] **Step 3: Implement `balance_statuses`, `unconfirmed_names`, fill `ClosureView.balances`, add the gate in `settle`**

- [ ] **Step 4: Run the whole core suite**

Run: `cargo test -p finance-core`
Expected: all pass. The existing settle tests that use the in-memory `TABLES` db (no ledger) must still pass: `balance_statuses` returns an empty list when `checked_accounts` is empty, and those tests' snapshots give accounts no `spending`/`savings` role unless they already do; where one does, add a confirmed check to the stored period in that test (`balance_checks` with `ledger_balance` equal to the snapshot account's balance) rather than weakening the gate.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/closure.rs crates/core/src/closure/tests.rs crates/core/src/writes/closure.rs crates/core/src/writes/closure/tests.rs
git commit -m "feat(closure): balance statuses on the view; settle waits for every confirmation"
```

---

### Task 5: Closure screen — Balances panel, locked Split, review note

**Files:**
- Create: `app/ui/src/pages/closure/balances.rs`
- Modify: `app/ui/src/pages/closure.rs` (`mod balances;`, `Wizard::confirm_balance`, step 0 body, step 2 lock)
- Modify: `app/ui/src/pages/closure/steps.rs` (review "Adjusted by" row)

**Interfaces:**
- Consumes: `ClosureView.balances: Vec<BalanceStatus>`, `ClosureAction::ConfirmBalance`, `post_closure`, `Wizard::write`/`base`/`latest`, `locked(...)`, ds `Input`, `Button`, `TABLE*`, `money`.
- Produces:
  ```rust
  impl Wizard {
      /// Confirm one account at `balance`; reloads the view on success.
      pub fn confirm_balance(self, account_id: String, balance: String);
      /// Confirm, in order, every row whose typed value equals its ledger figure; stop at the first error.
      pub fn confirm_matching(self, rows: Vec<(String, String)>);
  }
  #[component] pub fn BalancesPanel(balances: Vec<BalanceStatus>, disabled: Signal<bool>) -> impl IntoView;
  ```

- [ ] **Step 1: `Wizard::confirm_balance` and `confirm_matching`** in `app/ui/src/pages/closure.rs`

`confirm_balance` parses `balance` as `Decimal` (error `"Enter a number"` into `wizard.error` when it fails) and posts `ConfirmBalance` through `self.write`, done text `"Balance confirmed."`. `confirm_matching` loops: for each row call `self.latest().await` for the fresh revision, post, stop on `Err` and set `wizard.error`; done text `"Balances confirmed."`.

- [ ] **Step 2: `BalancesPanel`** in `app/ui/src/pages/closure/balances.rs`

Table columns `Account | Ledger | Bank | Confirm as | Status`, `data-density="sm"`, mirroring `BalanceCheck` in `app/ui/src/pages/manage/banks.rs`. Per row an `RwSignal<String>` prefilled with `bank` (if `Some`) else `ledger`, formatted with the currency's minor units; `Input` with `kind="number"`, `step` = `10^-minor_unit`. Status cell text by state: `Confirmed` → `"Confirmed"` (muted), `Stale` → `"Changed since confirmed"` (warning), `Unconfirmed` with bank off → `"Off by {money(bank - ledger)}"` (warning), else empty. A `Confirm` button on every row that is not `Confirmed`, plus a row for a `Confirmed` one whose input differs from its check balance. Above the table one `Confirm all matching` button (secondary), disabled when no row's input equals its ledger figure or `disabled` is true. Empty `balances` → render nothing.

- [ ] **Step 3: Place the panel**

In `step_view` case `0`, statuses `Open | Settling`: render `BalancesPanel` under `BankSyncPanel` (before `NewAccounts`). In case `2` (not `ChangedAfterClose`): when `view.balances.iter().any(|b| b.state != BalanceState::Confirmed)` render `locked(wizard, "Confirm every account balance in Sync first.".into(), Some(0), Some("Go to Sync"))` instead of the `SplitEditor`.

- [ ] **Step 4: Review note** in `ReviewStep` (`app/ui/src/pages/closure/steps.rs`)

`ReviewStep` gains a prop `balances: Vec<BalanceStatus>`; for an account whose status has `adjusted_by: Some(x)`, add a row `("Balance adjustment (not counted)", None, x)` styled with `FOOTNOTE`, placed after "Carry-in". Pass `view.balances.clone()` from `step_view`.

- [ ] **Step 5: Build the UI crate**

Run: `cargo check -p finance-ui --target wasm32-unknown-unknown` (or the workspace's usual UI check; see `app/ui/README` or `justfile` if present)
Expected: no errors or warnings.

- [ ] **Step 6: Manual check in the app** (`/run` skill or the project's dev server)

Open a Settling month: the Balances panel lists every spending/savings account; Split shows the lock until all rows read Confirmed; typing a different figure and confirming shows "Confirmed" and the review shows "Balance adjustment (not counted)"; the Overview balance moved by the same amount; "Confirm all matching" with one row failing (e.g. make the revision stale from a second window) stops and shows the error with the earlier row confirmed.

- [ ] **Step 7: Commit**

```bash
git add app/ui/src/pages/closure.rs app/ui/src/pages/closure/balances.rs app/ui/src/pages/closure/steps.rs
git commit -m "feat(closure): confirm every account balance before the split"
```

---

### Task 6: Whole-branch check

- [ ] **Step 1:** `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings` → clean.
- [ ] **Step 2:** `cargo test --workspace` → all pass.
- [ ] **Step 3:** `cargo run -p finance-core --example parity_closure` if it still builds against a sample period → unchanged output for periods without checks.
- [ ] **Step 4:** Open the PR with the spec and plan linked; the release workflow ships it on merge.
