# Closure: Move to Another Account — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A split line can move part of an account's leftover to another own account, with a bank transfer to tick off; on a spending pocket the money becomes next month's buffer.

**Architecture:** New destination `account:<id>` validated against the review's accounts; `with_transfer_targets` routes it like a goal line; `carry_in_for` credits spending-role targets; the next-month guards treat moves like buffer lines; the editor lists `Move to <name>` options and labels moves everywhere.

**Tech Stack:** Rust 2024 (rusqlite, serde, rust_decimal, jiff), Leptos CSR UI. `cargo test --workspace`, clippy native + wasm, fmt.

**Spec:** `docs/superpowers/specs/2026-11-01-closure-move-to-account-design.md`

## Global Constraints

- Error texts verbatim: `"move to an unknown account"`, `"cannot move money to the same account"`, `"a move must be positive"`, `"move currency does not match"`; next-month guard reuses `"next month already uses this buffer"`.
- Validation reads targets from `review.accounts` only: present, role `Spending` or `Savings` (not `Ignored`/`Unclassified`), same currency as the line.
- Carry-in: previous month's `account:` lines credit the **target** when the target's review role is `Spending`; a `Savings` target gets nothing. The source is never credited.
- Transfer: server sets `to_account_id = target`, `Pending`, then routes via `routed(...)`; client sends `to_account_id: None`. `checked_lines` regex gains `account:[^\s]{1,64}`.
- Suggestion counts `account:` lines as assigned.
- Payload shape unchanged; every commit compiles; both clippy targets and fmt clean; commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. A move to a savings account must not reappear as that account's leftover next month. → Task 1 test `a_move_to_savings_is_not_carried_in`.
2. A move between two accounts in different currencies must be refused, not converted. → Task 1 test `a_move_needs_the_same_currency`.
3. A move whose target sits behind a via-account must get the two-hop route, like a goal line. → Task 2 test `a_move_routes_through_the_via_account`.
4. Settling October with a move while November is already settled must be refused like a buffer line. → Task 3 test `a_move_is_guarded_like_a_buffer_line`.
5. The editor must not offer the line's own account, an ignored account, or an account in another currency. → Task 4 test `move_options_skip_self_ignored_and_other_currencies`.

---

### Task 1: Destination, validation, carry-in (core)

**Files:**
- Modify: `crates/types/src/report/closure.rs` (`AllocationLine::target_account_id`, doc on `destination`)
- Modify: `crates/core/src/closure.rs` (`validate_allocations`, `carry_in_for`)
- Modify: `crates/core/src/closure/tests.rs`

**Interfaces:**
- Produces: `impl AllocationLine { pub fn target_account_id(&self) -> Option<&str> }` (after `account:`); `carry_in_for(month, periods, roles: &[AccountRole])` — new `roles` parameter so a target's role is known (`AccountRoleKind::parse(&r.role)`); callers in `closure.rs` (`review_month` inputs ~line 469) pass the loaded roles.
- Rules in `validate_allocations`, checked per `account:` line before the per-account totals: target not in `review.accounts` or its role `Ignored`/`Unclassified` → `"move to an unknown account"`; target == `from_account_id` → `"cannot move money to the same account"`; amount ≤ 0 (both modes) → `"a move must be positive"`; target currency ≠ line currency → `"move currency does not match"`.

- [ ] **Step 1: Failing tests** in `crates/core/src/closure/tests.rs` (helpers `review`, `line`, `role`, `period`, `d`; build a review with two accounts by extending `review()` or a local helper with `accounts` of given ids/currencies/roles):
  - `a_move_needs_a_known_target_in_the_same_currency`: unknown → unknown text; ignored target → unknown text; same account → same-account text; USD target from EUR line → currency text; −10 → positive text; valid move → Ok.
  - `a_move_needs_the_same_currency` (the Review Focus case, separate test name for the record).
  - `a_move_is_the_targets_carry_in_only_when_it_spends`: previous period with `account:pocket` 300 from `abn` and `account:vault` 200; roles pocket=spending, vault=savings → carry_in == {pocket: 300}; `a_move_to_savings_is_not_carried_in` asserts vault absent and abn absent.
- [ ] **Step 2: Run** `cargo test -p finance-core closure::tests` → compile failures.
- [ ] **Step 3: Implement**; update `carry_in_for` callers.
- [ ] **Step 4:** `cargo test -p finance-core`; `cargo check --workspace --all-targets`.
- [ ] **Step 5: Commit** `feat(closure): a split line can move money to another account`.

---

### Task 2: Transfer routing and the line regex (core)

**Files:**
- Modify: `crates/core/src/closure/transfers.rs` (`with_transfer_targets`)
- Modify: `crates/core/src/writes/closure.rs` (`checked_lines` regex)
- Modify: `crates/core/src/closure/tests.rs`

**Interfaces:**
- `with_transfer_targets`: for an `account:` line with target ≠ from and amount > 0 → `to_account_id = Some(target)`, `transfer_status = Pending`, then the existing `routed` pass (via routes).
- Regex: `^(goal:[^\s]{1,128}|pot:[^\s]{1,64}|wish:[^\s]{1,64}|account:[^\s]{1,64}|plan|buffer|stay)$`.

- [ ] **Step 1: Failing tests**: `a_move_gets_a_pending_transfer_to_its_target` (direct); `a_move_routes_through_the_via_account` (target with `via` → two hops, like `transfers_into_a_pocket_route_via_the_main_account`); `match_transfers` matches a bank transfer of the amount to the target (reuse `transfer_matching_uses_each_bank_transfer_once`'s setup).
- [ ] **Step 2: Run** → failures. **Step 3: Implement.** **Step 4:** core tests green. **Step 5: Commit** `feat(closure): a move expects a bank transfer to its target`.

---

### Task 3: Settle/resettle guards and the fixture-db round trip (core)

**Files:**
- Modify: `crates/core/src/writes/closure.rs` (next-month guards ~line 803 and ~942: treat `account:` like `buffer`)
- Modify: `crates/core/src/writes/closure/tests.rs`

- [ ] **Step 1: Failing tests** (fixture db, as the `confirm_*`/`settle_*` tests): `a_move_is_guarded_like_a_buffer_line` (November stored as Settled → settling October with a move → `"next month already uses this buffer"`); `settling_a_move_stores_a_pending_transfer` (settle October with `account:firefly:5` 100 from `firefly:1` → stored line has `to_account_id == Some("firefly:5")`, `Pending`; then `mark` with a transaction id marks it done); `a_resettle_delta_move_gets_its_own_transfer`.
- [ ] **Step 2–5:** run, implement, `cargo test --workspace`, commit `feat(closure): moves follow the buffer guards and settle with a transfer`.

---

### Task 4: Screen

**Files:**
- Modify: `app/ui/src/pages/closure/split.rs` (`destination_options` gains `moves: &[(String, String)]` — (account_id, name) of eligible targets; `Setup`/`SplitTarget` carry the per-account target list; `parse` treats `account:` as positive-only)
- Modify: `app/ui/src/pages/closure.rs` (build the move list per account from `view.accounts` roles + `view.review.accounts` currencies: active spending/savings, same currency, not self)
- Modify: `app/ui/src/pages/closure/steps.rs` (`Move to <name>` label; carry-in row label `Carry-in (buffer, moved in, planned purchases)`)
- Modify: `app/ui/src/pages/closure/suggestion.rs` (`account:` counts as assigned; test)

- [ ] **Step 1: Failing tests**: `move_options_skip_self_ignored_and_other_currencies`; `move_lines_count_as_assigned`; extend `destination_options` order test (stay, buffer, moves by name, wishes, goals, plan).
- [ ] **Step 2: Implement**; `cargo test -p finance-ui`; wasm clippy; fmt.
- [ ] **Step 3: Manual check** (controller, dev server on a DB copy): October split on ABN AMRO offers "Move to Revolut Groceries / Revolut Fun out / Revolut Savings / Swedbank / Revolut"; a move line shows in the Transfers step with a pending transfer.
- [ ] **Step 4: Commit** `feat(ui): split lines can move money to another account`.

---

### Task 5: Whole-branch check

- [ ] fmt, both clippy targets, `cargo test --workspace`; open the PR linking spec and plan.
