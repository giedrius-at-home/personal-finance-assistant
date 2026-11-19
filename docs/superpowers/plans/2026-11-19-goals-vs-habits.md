# Goals against Habits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tell the owner what the wishes and an emergency fund need each month, what current habits leave, and which spending to cut.

**Architecture:** The wishes ladder runs a second, read-only time on habit baselines (typical spending per group from `Dashboard.adjustments`, every non-essential group cuttable). The result and the emergency fund ride on `WishesView.goals`. A Goals card on Spending → Plan & patterns shows it; the fund is edited on Wishes.

**Tech Stack:** Rust (`finance-core`, `finance-types`), Leptos UI.

**Spec:** `docs/superpowers/specs/2026-11-19-goals-vs-habits-design.md`

## Global Constraints

- `wishes::ladder`, `baseline`, `schedule` and the Wishes verdicts and writes stay unchanged; the habit scenario only builds different `MonthBaseline`s.
- Month 0 baseline is the plan's, untouched.
- Essentials (default `Groceries`, `Transport`, `Health & personal care`, `Housing & utilities`) are never cut unless the owner ranked them in a plan.
- `emergency_months`: default 6, 0 = off, max 36.
- `cargo` via `export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"`; `cargo fmt --all` before commits; dev server only with `DEV_DB=<copy>`.

## Review Focus

1. Stored `wish_settings` rows without the new fields (and the legacy single-card shape) must load with the defaults.
2. A row with no typical month (under three earlier months) must not enter the scenario as zero and become "free" money.
3. No allocation or no wishes: `goals` is `None`/empty, no panic, card hidden.
4. A wish affordable from the savings pot alone must not produce cuts.
5. Fund off (0 months) and steady free money ≤ 0: no date, no division by zero.

---

### Task 1: Settings and report types

**Files:** `crates/types/src/gateway.rs` (WishSettings + Json + Default), `crates/types/src/report/wishes.rs` (GoalOutlook, GoalStatus, HabitCut, EmergencyFund; `WishesView.goals`), `crates/core/src/writes/wishes.rs` (validation), every `WishSettings { .. }` literal.

- [ ] Tests first (`gateway.rs` tests): `{"cards":[…]}` and the legacy single-card JSON load with `emergency_months == 6` and the four default essentials; a stored `emergency_months: 3` round-trips. `writes/wishes.rs`: `emergency_months: 37` is rejected; essentials are trimmed and empty names dropped.
- [ ] Implement; run `cargo test -p finance-types -p finance-core`; commit `feat(wishes): emergency fund settings and goal outlook types`.

### Task 2: Habit scenario and goal outlook

**Files:** create `crates/core/src/wishes/goals.rs`; modify `crates/core/src/wishes/mod.rs` (`Inputs` gains `habits: Vec<HabitRow>`; `view` fills `goals`).

**Interfaces (produces):**

```rust
pub struct HabitRow { pub id: String, pub name: String, pub typical: Decimal, pub budget: Decimal, pub typical_visits: Option<Decimal>, pub essential: bool }
pub fn habit_rows(adjustments: &[Adjustments], currency: &Currency, essentials: &[String]) -> Vec<HabitRow>;
pub fn habit_baselines(plan: &[MonthBaseline], rows: &[HabitRow], plans: &[BudgetPlan]) -> Vec<MonthBaseline>;
pub fn fixed_monthly(alloc: &CurrencyAllocation, card_bills: &[(String, Decimal)], items: &[Planned]) -> Decimal;
pub fn fund(months: u32, fixed: Decimal, rows: &[HabitRow], habit_months: &[WishMonth], steady: Decimal, month0: Date) -> Option<EmergencyFund>;
pub fn cuts(habit_months: &[WishMonth], rows: &[HabitRow]) -> Vec<HabitCut>;
```

- [ ] Tests first (values in the spec's Tests list; baselines built with `baseline::tests::{alloc,row,d}` and ladder `MonthBaseline` literals): scenario free money and rooms; essentials/floors; month 0 untouched; a row without typical is not in `habit_rows`; a wish fine on plan, short on habits; a wish paid from savings gives no cuts; cut aggregation; `fixed_monthly` with a never-ending bill change (+250 monthly), a weekly recurring wish (× 52/12) and an ending one (ignored); eta inside horizon, after it, none; fund off.
- [ ] Implement; `cargo test -p finance-core`; clippy; commit `feat(wishes): goals judged against current habits, with an emergency fund`.

### Task 3: UI

**Files:** create `app/ui/src/pages/goals.rs`; modify `app/ui/src/pages/mod.rs`, `spending.rs` (card above Adjustments), `wishes.rs` (Emergency fund block).

- [ ] Tests first (pure helpers in `goals.rs`): `fewer_visits(cut, typical, typical_visits)` → `Some(8)` for 150 / (570/30) rounded, `None` without visits or under one; `goal_line` for on-track, short and plan-short wishes; `fund_line` for full-by date, already full, no date, off; `month_label("2028-06-01") == "Jun 2028"`.
- [ ] Implement card and fund block; `cargo test -p finance-ui --bins`; wasm clippy.
- [ ] Dev server on a ledger copy: card shows needed/left/gap, the three planned wishes, fund target near 6 × bare cost; change months to 3 on Wishes → target halves; add a bill-change wish raising Rent → fund target and gap rise. Real ledger untouched.
- [ ] Commit `feat(spending): the Goals card; emergency fund on Wishes`.

### Task 4: Checks, review, PR

- [ ] Full CI set (fmt, both clippy lines, all tests); fresh whole-branch review; fix Important findings test-first; push; PR.
