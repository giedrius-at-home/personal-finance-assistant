# Closure: Wishes Instead of Pots — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove pots from the month closure and let a split line save toward a planned wish instead, while every closed month with old pot lines keeps rendering and matching.

**Architecture:** `wish:<id>` becomes a validated split destination; confirming a split rewrites each touched wish's `saved` relative to the period's previous lines (the pot-entry side effect goes). The suggestion drops the General pot and the take offer. The UI loses the Pots panel, pot destinations and takes, gains wish destinations and legacy labels. Pot reads stay only for old lines.

**Tech Stack:** Rust 2024 (rusqlite, serde, rust_decimal, jiff), Leptos CSR UI, Tauri commands. `cargo test --workspace`, clippy native and wasm, fmt.

**Spec:** `docs/superpowers/specs/2026-11-01-closure-wishes-instead-of-pots-design.md`

## Global Constraints

- Serde shapes of `MonthPeriod`, `AllocationLine`, `SavingsSuggestion` payloads stored in `month_periods` do not change; `SavingsSuggestion` (a view type, never stored) may drop fields.
- Error texts verbatim: `"pots are no longer available"` (any `pot:` destination or any negative non-delta line on a new split), `"unknown wish"`, `"a wish line must be positive"`, `"wish currency does not match"`, `"saves more than the wish still needs"`.
- Wish lines: `LineKind::Split` (or `Delta` on resettle), `TransferStatus::NotNeeded`, `to_account_id: None`, `transfer_hops` empty.
- Only `WishKind::OneOff` and `WishStatus::Planned` wishes accept lines; a parent's cap is `rest − parent.saved + this month's earlier contribution` where `rest = amount − Σ planned/bought sub-wish amounts`; a leaf's cap is `amount − saved + earlier contribution`.
- Side effect: `saved' = max(0, saved − Σ previous lines for the wish + Σ new lines for the wish)`, over all lines in the period's allocations; `saved_month' = max(saved_month, closure month)`; non-planned wishes skipped; written with the wishes store's upsert.
- Legacy `pot:` lines: still deserialise, still render (`Pot (retired): <name>`), still route and match transfers via `load_pots`/`with_transfer_targets`; `general_pot` is no longer created on read or write.
- Every commit compiles (`cargo check --workspace --all-targets`); `cargo fmt --all`; commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. A re-confirm that drops a wish line must lower `saved` by exactly that line, never below zero even after a manual lower on the Wishes page. → Task 2 tests `resplit_without_the_wish_line_gives_the_money_back` and `manual_lower_then_resplit_clamps_at_zero`.
2. Closing October in November must not switch off a November set-aside already recorded (`saved_month` only moves forward). → Task 2 test `closure_never_moves_saved_month_backwards`.
3. September (closed, with `pot:general` lines and matched transfers) must still load, render and keep its transfer status after the change. → Task 1 test `legacy_pot_lines_still_route_and_match`, Task 4 manual check.
4. A split that saves to a wish from an account in another currency than the wish must be refused, not silently converted. → Task 1 test `wish_line_currency_must_match`.
5. The suggestion must count wish lines as assigned so "Already assigned in this split" and "Suggested" move when a wish line is typed. → Task 3 test `wish_lines_count_as_assigned`.

---

### Task 1: Destinations and validation (core)

**Files:**
- Modify: `crates/types/src/report/closure.rs` (`AllocationLine::wish_id`)
- Modify: `crates/core/src/closure.rs` (`validate_allocations`, `WishCaps`)
- Modify: `crates/core/src/closure/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  impl AllocationLine { pub fn wish_id(&self) -> Option<&str> }   // after "wish:"
  /// What each one-off planned wish can still take this month, keyed by wish id.
  pub struct WishCap { pub currency: Currency, pub room: Decimal }
  pub type WishCaps = HashMap<String, WishCap>;
  /// Caps from the wishes and the period's previous lines (earlier contribution added back).
  pub fn wish_caps(wishes: &[Wish], previous: &[AllocationLine]) -> WishCaps;
  pub fn validate_allocations(
      review: &ClosureReview, lines: &[AllocationLine], goal_ids: &HashSet<&str>,
      baseline: Option<&ClosureReview>, wishes: &WishCaps,
  ) -> Result<(), String>;   // `pots: &PotBalances` parameter removed
  ```
  Rules in order per line: `pot:` destination or (negative amount and `baseline.is_none()`) → `"pots are no longer available"`; `wish:` → id in caps else `"unknown wish"`; amount ≤ 0 → `"a wish line must be positive"` (also in delta mode); currency ≠ cap currency → `"wish currency does not match"`; Σ lines for the wish > room → `"saves more than the wish still needs"`. `wish_caps` includes only `Planned` + `OneOff` wishes; a parent with planned/bought sub-wishes gets `rest − saved`, others `amount − saved`; plus Σ previous lines for that wish; room floored at 0.

- [ ] **Step 1: Failing tests** in `crates/core/src/closure/tests.rs` (helpers `review`, `line`, `d` exist; add a `wish(id, amount, saved, parent: Option<&str>)` helper building a `Wish` with `kind: OneOff`, `status: Planned`, EUR)

```rust
#[test] fn pot_lines_and_takes_are_refused_on_a_new_split()
  // line("p","pot:general","10") → "pots are no longer available"; line("s","stay","-10") → same
#[test] fn wish_lines_need_a_planned_one_off_wish_with_room()
  // unknown id → "unknown wish"; bought wish → "unknown wish"; amount -5 → "a wish line must be positive";
  // wish amount 100 saved 90, lines 20 → "saves more than the wish still needs"; lines 10 → Ok
#[test] fn wish_line_currency_must_match()
  // USD line to EUR wish → "wish currency does not match"
#[test] fn parent_cap_is_its_rest_and_earlier_lines_count_back()
  // parent 500 with sub 200 (planned), parent saved 250 → room 50; previous line of 30 to parent → room 80
#[test] fn legacy_pot_lines_still_route_and_match()
  // with_transfer_targets on a stored pot:general line (pot located on another account) still sets to_account_id and Pending; match_transfers still matches it
```

- [ ] **Step 2: Run** `cargo test -p finance-core closure::tests` → compile failures on the new names.
- [ ] **Step 3: Implement** `wish_id`, `WishCap`, `wish_caps`, the new `validate_allocations`; update the two existing callers in `crates/core/src/writes/closure.rs` (settle, resettle) to pass `wish_caps(&load_wishes(&w.tx)?, &period.allocations)` and remove `ensure_general_pot`, `open_pot_balances` from them (leave `load_pots` where transfers need it). Delete `open_pot_balances` if unused; keep `pot_balances`, `load_pots`, `load_pot_entries` for legacy reads.
- [ ] **Step 4: Run** `cargo test -p finance-core`; `cargo check --workspace --all-targets` clean (the existing settle tests that used pot lines or takes change to goal/buffer/stay lines or to the new error, with the test name saying which).
- [ ] **Step 5: Commit** `feat(closure): a split can save toward a wish; pots are refused`.

---

### Task 2: The side effect on wishes (core)

**Files:**
- Modify: `crates/core/src/writes/closure.rs` (`record_side_effects`)
- Modify: `crates/core/src/writes/wishes.rs` (make `store` `pub(crate)`)
- Modify: `crates/core/src/writes/closure/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  /// Bring each touched wish's `saved` from the period's previous lines to its new lines.
  pub(crate) fn apply_wish_lines(w: &Writer<'_>, period: &MonthPeriod, previous: &[AllocationLine]) -> Result<()>;
  ```
  `record_side_effects` calls it instead of the pot-entry rewrite (the `DELETE FROM pot_entries` and insert loop go; `pot_entries()` in closure.rs is deleted).

- [ ] **Step 1: Failing tests** on the fixture db (`crate::writes::fixture::db()`; store a wish with the wishes store; settle with `line("w","wish:<id>","40")`)

```rust
#[test] fn settle_raises_the_wish_saved_and_sets_its_month()        // saved 0 → 40, saved_month = "2026-10-01"
#[test] fn resplit_without_the_wish_line_gives_the_money_back()     // 40 → re-settle with no wish line → 0
#[test] fn manual_lower_then_resplit_clamps_at_zero()               // settle 40; record_saved 10; re-settle 40 → max(0, 10-40+40)=10; re-settle none → 0
#[test] fn closure_never_moves_saved_month_backwards()              // wish saved_month "2026-11-01"; close Oct → stays "2026-11-01"
#[test] fn a_bought_wish_is_skipped_but_its_line_stays()            // mark bought, resettle → saved unchanged, line present in period
#[test] fn a_resettle_delta_line_adds_to_the_wish()                 // ChangedAfterClose + delta wish line 15 → saved 55
```

- [ ] **Step 2: Run** → failures.
- [ ] **Step 3: Implement** `apply_wish_lines`; remove `pot_entries`, `ensure_general_pot`, `save_pot`, `pots_view`, `general_pot` and `PotsView`/`PotDraft`/`PotEntryView` usages in core and `Service` (`pots`, `save_pot`), and the Tauri commands `closure_pots`/`save_pot` in `app/src-tauri/src/main.rs` and `build.rs` (regenerate the autogenerated permission files by building). `crates/types` keeps `PotView` (used by `ClosureView.pots` for legacy labels) and drops `PotsView`, `PotEntryView`, `PotDraft`.
- [ ] **Step 4: Run** `cargo test --workspace`; `cargo check --workspace --all-targets`; `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(closure): confirming a split records what each wish saved`.

---

### Task 3: The suggestion without the General pot (core + UI figures)

**Files:**
- Modify: `crates/types/src/report/closure.rs` (`SavingsSuggestion`: drop `general_pot_id`, `general_balance`)
- Modify: `crates/core/src/closure.rs` (`suggestion`, `closure_view`)
- Modify: `app/ui/src/pages/closure/suggestion.rs` (`Figures`, `figures`, card)
- Test: `crates/core/src/closure/tests.rs`, `app/ui/src/pages/closure/suggestion.rs` tests

**Interfaces:**
- Produces: `Figures { projected, everyday, known_future, safety_buffer, assigned, suggested }` with `suggested = projected − everyday − known_future − safety_buffer − assigned`; `assigned` = Σ positive EUR lines whose destination is `wish:`, `goal:`, `plan` or `buffer`. Card copy: `"{Month} is short by €X"` / `"{Month} has €X to spare"`, no Take button, breakdown without the General savings line.

- [ ] **Step 1: Failing tests** — `wish_lines_count_as_assigned` (UI `figures`: a `wish:` line of 50 lowers `suggested` by 50 and raises `assigned`); core `suggestion_starts_from_overview_and_reserves_what_is_still_to_come` updated to the new struct.
- [ ] **Step 2: Run** → failures. **Step 3: Implement.** **Step 4:** `cargo test -p finance-core -p finance-ui`, wasm clippy clean. **Step 5: Commit** `feat(closure): the shortfall card informs; no take from a pot`.

---

### Task 4: Closure screen without pots

**Files:**
- Modify: `app/ui/src/pages/closure.rs` (remove `PotsPanel`, `save_pot*`, `create_pot_for_line`, `pots` props; pass `wishes` to the editor)
- Modify: `app/ui/src/pages/closure/split.rs` (destination list, remove takes / `NEW_POT` / `can_create_pot` / `pots` / `locations` props, add `wishes: Vec<WishOption>`)
- Modify: `app/ui/src/pages/closure/panels.rs` (delete `PotsPanel`, `PotRow`, pot overflow notes)
- Modify: `app/ui/src/pages/closure/steps.rs` (legacy label `Pot (retired): <name>`; wish label `Wish: <name>`)
- Modify: `app/ui/src/pages/closure/ui.rs` (remove `load_pots`, `post_pot`)
- Modify: `crates/types/src/report/closure.rs` + `crates/core/src/closure.rs`: `ClosureView.wishes: Vec<WishOption { wish_id, label, currency, room }>` built from `wish_caps` (labels `Wish: <name> (€saved of €amount)`, `Wish: <name> rest` for a parent).

**Interfaces:**
- Consumes: `ClosureView.wishes`, `ClosureView.pots` (legacy names only).
- Produces: destination options in order: Stays on this account, Next month's buffer, each `wish:` option, each `goal:` option, Plan a purchase…. Lines whose destination is `pot:` or a `wish:` not in `wishes` render read-only with the legacy/bought label and a Remove button.

- [ ] **Step 1:** Unit tests for the pure option builder (`destination_options(wishes, goals, current)` keeps a legacy current value in the list with its label) in split.rs tests.
- [ ] **Step 2:** Implement; delete dead code; `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings`, `cargo test -p finance-ui`, fmt.
- [ ] **Step 3: Manual check** (controller, dev server on a DB copy): October split lists wishes and no pots; September (closed) Transfers still shows its pot transfers as matched with `Pot (retired): General savings`; the shortfall card has no Take button.
- [ ] **Step 4: Commit** `feat(ui): split lines save toward wishes; the pots panel is gone`.

---

### Task 5: Whole-branch check

- [ ] `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings`; `cargo test --workspace`.
- [ ] `grep -rn "pot" app/ui/src crates/core/src/closure.rs crates/core/src/writes/closure.rs` lists only legacy-read code and comments.
- [ ] Open the PR linking spec and plan.
