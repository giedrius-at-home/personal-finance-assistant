# Adjustments Card and Visit Habits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A card on Spending → Plan & patterns that shows, per spending group, the budget, the owner's typical month and where this month is heading, with drivers, one budget action and a visit-habits table.

**Architecture:** Core extends the month's budget plan in memory with zero-amount "shadow" groups for uncovered spending groups and runs the existing budget engine on it, so every number shares the budget's months, exclusions and ownership. A second pass over the same owned purchases counts visits per row and per store. The UI derives findings from those rows with PR #105's `month_end` and writes through the existing `save_budget_plan`.

**Tech Stack:** Rust; `finance-core`, `finance-types` (rust_decimal, jiff); Leptos UI (`finance-ui`, wasm32) with Tailwind.

**Spec:** `docs/superpowers/specs/2026-11-19-adjustments-card-design.md`

## Global Constraints

- PR #105 is merged; branch is rebased on it. Do not edit `app/ui/src/pages/overview/visuals.rs`, `app/ui/src/ds/charts.rs`, `app/ui/src/pages/overview/budget.rs` or `app/ui/src/pages/category_trends.rs`; only call what they export.
- New core arithmetic uses `crate::reporting::{py_div, py_round}`, `crate::trends::{py_mul, median}`.
- No new table, setting or Tauri command. No use of `spending_group_limits`.
- Every sentence in the card is built from numbers the card shows; no AI text.
- Groups never given a shadow row: `Transfers`, `Investments`, `Other`.
- Thresholds, verbatim from the spec: above norm = heading ≥ 1.25 × typical and heading − typical ≥ 25; budget low = budget ≤ 0.8 × typical; budget high = budget ≥ 1.2 × typical and budget − typical ≥ 25; typical needs three earlier months above zero; too early = `elapsed_days < 7` or `visits < 2`; action amount = typical rounded to the nearest 10.
- `cargo` is not on PATH: `export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"` first.
- Before each commit: `cargo fmt --all`. Commit messages end with `Co-Authored-By: Claude Fable 5.2 <noreply@anthropic.com>`.
- Never run the dev server against the real ledger: always `DEV_DB=<copy>`.

## Review Focus

1. A purchase a real budget group claims by `categories` or `transaction_ids` while its reviewed group is otherwise uncovered: it must count once, in the real row (Task 3 test).
2. A month closed early (resolver moves late bookings to the next month): rows, visits "this month" and `elapsed_days` must all be the budget month (Task 3 test).
3. A row with a refund larger than its spending this month (negative `spent`): no finding, no negative "heading", no panic on averages (Task 4 test).
4. A new budget group whose slug already exists in the plan (`food-out`): the added id gets a numeric suffix so `save_budget_plan` does not reject or overwrite (Task 4 test).
5. A currency with no saved plan at all and a later month: carry-forward must fall to the default, and a plan saved for a *later* month must not flow backwards (Task 1 test).

---

### Task 1: Budgets that last

**Files:**
- Modify: `crates/core/src/budget.rs` (`monthly_budgets`, ~line 401; tests module)

**Interfaces:**
- Produces: `pub fn plan_for(saved: &[BudgetPlan], month: Date, currency: &Currency) -> BudgetPlan` — the plan saved for `month`; else the latest saved plan of that currency with an earlier month, cloned with `month` set to `month.to_string()` and `revision: 0`; else `default_budget(month, currency)`.

- [ ] **Step 1: Write the failing tests** in `budget.rs` tests:

```rust
fn saved(month: &str, amount: i64) -> BudgetPlan {
    BudgetPlan { month: month.into(), currency: Currency::eur(), revision: 3,
        groups: vec![group("food-out", "Food & drinks out", amount, &["Eating out"])] }
}

#[test]
fn a_month_without_a_plan_uses_the_latest_earlier_one() {
    let plans = [saved("2026-09-01", 250), saved("2026-11-01", 430)];
    let dec = plan_for(&plans, date(2026, 12, 1), &Currency::eur());
    assert_eq!((dec.month.as_str(), dec.revision), ("2026-12-01", 0));
    assert_eq!(dec.groups[0].amount, Decimal::from(430));
    // The month's own plan wins, untouched.
    assert_eq!(plan_for(&plans, date(2026, 11, 1), &Currency::eur()).revision, 3);
    // A later plan never flows backwards.
    assert_eq!(plan_for(&plans, date(2026, 10, 1), &Currency::eur()).groups[0].amount, Decimal::from(250));
}

#[test]
fn no_earlier_plan_falls_back_to_the_default() {
    let plans = [saved("2026-11-01", 430)];
    assert_eq!(plan_for(&plans, date(2026, 10, 1), &Currency::eur()), default_budget(date(2026, 10, 1), &Currency::eur()));
    assert!(plan_for(&plans, date(2026, 12, 1), &Currency::new("USD")).groups.is_empty());
}
```

- [ ] **Step 2: Run** `cargo test -p finance-core budget::tests::a_month_without` — Expected: FAIL, `plan_for` not found.
- [ ] **Step 3: Implement `plan_for`** and call it from `monthly_budgets` in place of the `saved.get(..).map_or_else(default_budget ..)` expression (pass `&snap.budget_plans`; drop the now-unused `saved` map).
- [ ] **Step 4: Run** `cargo test -p finance-core` — Expected: PASS (existing budget and parity tests unchanged).
- [ ] **Step 5: Commit** `feat(budget): a month without a plan keeps the last saved one`.

### Task 2: One ownership function for the budget engine

**Files:**
- Modify: `crates/core/src/budget.rs` (`monthly_budget`, lines ~146–250)

**Interfaces:**
- Produces:

```rust
/// Which budget row a purchase counts in, shared by the budget and the
/// adjustments pass.
pub struct Ownership<'a> { /* plan groups, mapping, fixed ids, reimbursement origins, paid ids */ }
impl<'a> Ownership<'a> {
    pub fn new(plan: &'a BudgetPlan, inputs: &'a BudgetInputs<'a>) -> Self;
    /// A group id, or "fixed", "extras", "unplanned".
    pub fn key(&self, t: &Tx) -> &str;
    /// The reviewed group of a category.
    pub fn source_group(&self, category: Option<&str>) -> Option<&str>;
}
/// True for the outflows and refunds of the plan's currency booked by `now`.
pub fn counts(t: &Tx, plan: &BudgetPlan, now: &Zoned) -> bool;
/// The budget month a purchase belongs to.
pub fn month_of(t: &Tx, inputs: &BudgetInputs) -> Date;
```

- [ ] **Step 1:** Pure refactor: move the `owner`/`key` computation, the `reimbursement_origins` and `paid_ids` maps, the three-condition `continue` filter and the `tx_month` match out of `monthly_budget`'s loop into the items above; `monthly_budget` calls them. No behaviour change, no new test.
- [ ] **Step 2: Run** `cargo test -p finance-core` and `cargo clippy -p finance-core --all-targets -- -D warnings` — Expected: PASS, including `examples/parity_budget.rs` building.
- [ ] **Step 3: Commit** `refactor(budget): purchase ownership as one function`.

### Task 3: Core adjustments report

**Files:**
- Create: `crates/types/src/report/adjustments.rs` (types exactly as in the spec's "One source of numbers" block: `Adjustments`, `AdjustmentRow`, `StoreHabit`, `Habit`; derive `Clone, Debug, PartialEq, Serialize, Deserialize`; `Habit` also `Default`)
- Modify: `crates/types/src/report/mod.rs` (`pub mod adjustments;`; `Dashboard` gains `#[serde(default)] pub adjustments: Vec<adjustments::Adjustments>`)
- Create: `crates/core/src/adjustments.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod adjustments;`), `crates/core/src/dashboard.rs` (compute after `budget::monthly_budgets`, before `apply_estimates`; set the field), any `Dashboard { .. }` literal in tests/examples

**Interfaces:**
- Consumes: `budget::{plan_for, Ownership, counts, month_of, monthly_budget, BudgetInputs}`.
- Produces:

```rust
pub const NO_SHADOW: [&str; 3] = ["Transfers", "Investments", "Other"];

/// The plan with one zero-amount group (id `shadow:<group>`, name and
/// `source_groups` = the group) per reviewed group that has variable
/// spending in the six budget months and that no real group lists in
/// `source_groups`, except `NO_SHADOW`. Order: first seen.
pub fn shadow_plan(plan: &BudgetPlan, inputs: &BudgetInputs) -> BudgetPlan;

/// One currency's card data for the plan's month.
pub fn adjustments(plan: &BudgetPlan, subscriptions: &SubscriptionReport,
    period_start: Option<Date>, inputs: &BudgetInputs) -> Adjustments;

/// The running month for every currency (the dashboard's `adjustments`).
pub fn all(ctx: &Ctx, spending: &[Tx], payments: &[PlannedPaymentView],
    subscriptions: &SubscriptionReport) -> Vec<Adjustments>;
```

Decisions the tests do not fix:
- `adjustments` runs `monthly_budget(&shadow_plan(..), ..)`; `rows` = its `groups` in order, `budgeted = !id.starts_with("shadow:")`, `group` = the shadow's source group.
- Habit pass: iterate `inputs.transactions` where `counts(..)`, key with `Ownership::new(&shadow, inputs)`; skip `fixed` and `extras`. Outflows are visits; `spent` nets refunds. Monthly series by `month_of` over the same six months as `BudgetRow.history`; `typical_*` = `median` of earlier months that have a visit, `None` under two. Weekly series by `t.effective_on()`: week = Monday-start; "this week" = Monday of `now.date()` through today; the 12 full weeks are the ones before it; `recent_visits` = last 4 full weeks + this week; `weekday_visits` over the 12 full weeks.
- `stores`: one per `Some(category)` among the row's purchases in the six months or 12 weeks, sorted by `recent_visits` desc then name.
- `unsorted`: net of this budget month's purchases keyed `unplanned` whose category is `None`, or whose `source_group` is `None` or `Some("Other")`.
- `all` uses `plan_for` for `ctx.current_month()` per currency, the same currency set and `BudgetInputs` as `monthly_budgets` (factor a private helper there if needed rather than copying).

- [ ] **Step 1: Write the failing tests** in `crates/core/src/adjustments.rs` (reuse `budget::tests` helpers by making `tx`, `with_category`, `now`, `month` `pub(crate)`; add a local `on(t, "2026-08-14")` setting `occurred_at`, and `inputs(&txs, &mappings, &fixed)`; `now` is Thursday 2026-09-10, so this week starts Monday 2026-09-07):

```rust
#[test] fn uncovered_groups_get_shadow_rows_and_excluded_ones_do_not()
// mappings Jumbo→Groceries, Zara→Shopping, Wise→Transfers, Misc→Other; default plan (covers Groceries).
// rows ids == ["groceries", "food-out", "shadow:Shopping"]; the Shopping row: budgeted false, group Some("Shopping"), row.planned == 0.

#[test] fn a_purchase_a_real_group_claims_counts_once()
// plan groceries.categories = ["Zara"]; one Zara 40 this month.
// groceries row.spent == 40; no shadow:Shopping row has spent > 0; sum of all rows' spent == 40.

#[test] fn habits_count_visits_per_month_and_week()
// Jumbo outflows: 2 in July, 4 in Aug, this month: Sep 1 (Tue), Sep 8 (Tue), Sep 9 (Wed), plus a refund Sep 9.
// groceries habit: visits 3; typical_visits Some(3) (median of 2,4); history_month_count 2;
// visits_this_week 2; weekday_visits sums to the outflows inside Jun 15..=Sep 6; spent nets the refund.
// stores[0].category == "Jumbo" with the same visits.

#[test] fn zero_weeks_count_in_the_usual_week()
// one visit in each of 5 of the 12 full weeks → typical_week_visits == 0.

#[test] fn fixed_spending_is_left_out()
// a Jumbo tx whose id is in fixed_ids → not in visits, not in spent.

#[test] fn unsorted_is_uncategorised_unmapped_and_other()
// this month: category None 10, "Nowhere" (no mapping) 20, Misc→Other 30, Wise→Transfers 500 → unsorted == 60.

#[test] fn an_early_closed_month_follows_the_budget_month()
// resolver with August closed on Aug 28; a Jumbo tx on Aug 30 → counts in September's row.spent and habit.visits.
```

- [ ] **Step 2: Run** `cargo test -p finance-core adjustments` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** types, `shadow_plan`, `adjustments`, `all`; wire into `dashboard.rs`.
- [ ] **Step 4: Run** `cargo test -p finance-core -p finance-types` and `cargo clippy --locked -p finance-types -p finance-core -p finance-devserver --all-targets -- -D warnings` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): adjustments report with shadow budget rows and visit habits`.

### Task 4: UI card logic (pure)

**Files:**
- Create: `app/ui/src/pages/adjustments.rs` (logic + tests at the top; component added in Task 5)
- Modify: `app/ui/src/pages/mod.rs` (`pub mod adjustments;`)

**Interfaces:**
- Consumes: `finance_types::report::adjustments::*`, `crate::pages::overview::visuals::{month_end, Pace}`.
- Produces:

```rust
pub fn typical(row: &BudgetRow) -> Option<Decimal>;            // median of history[..len-1] entries > 0, needs 3
pub fn too_early(row: &AdjustmentRow) -> bool;                 // elapsed_days < 7 || habit.visits < 2
pub fn heading(row: &AdjustmentRow, days: usize) -> Option<Decimal>; // None when too early; month_end(.., Pace::Daily, days)
pub fn above_norm(row: &AdjustmentRow, days: usize) -> Option<Decimal>; // the excess D, when the rule holds
#[derive(Debug, PartialEq)] pub enum BudgetFit { Low, High }
pub fn budget_fit(row: &AdjustmentRow) -> Option<BudgetFit>;   // budgeted rows only
pub fn round_to_ten(amount: Decimal) -> Decimal;               // half up: 425 → 430, 424.99 → 420
#[derive(Debug, PartialEq)] pub enum Action { Set(Decimal), Lower(Decimal), Add(Decimal) }
pub fn action(row: &AdjustmentRow, days: usize) -> Option<Action>; // Low→Set, High→Lower, shadow+above_norm→Add
pub fn apply(plan: &BudgetPlan, row: &AdjustmentRow, action: &Action) -> BudgetPlan;
pub struct Driver { pub category: String, pub visits: usize, pub spent: Decimal, pub usual: Option<(Decimal, Decimal)> } // (typical_visits, typical_spent)
pub fn drivers(row: &AdjustmentRow, days: usize) -> Vec<Driver>; // top 3, positive excess only
pub fn mostly(weekday_visits: &[usize; 7]) -> String;          // "Fri, Sat" | "any day"
pub fn busy_week(habit: &Habit) -> bool;                       // visits_this_week ≥ typical_week_visits + 2
pub fn shown(habit: &Habit) -> bool;                           // recent_visits ≥ 4
```

`apply`: `Set`/`Lower` replace the amount of the group whose id is `row.row.id`; `Add` pushes `BudgetGroup { id: slug, name: group, amount, source_groups: vec![group], ..empty }` where slug is the lowercased group with non-alphanumerics as `-`, suffixed `-2`, `-3`… until unused. `plan` passed in is the real plan (`MonthlyBudget.plan`), never the shadow one. `mostly`: weekdays with ≥ 25% of the total, the two largest, in week order, three-letter names; "any day" when none or total is zero.

- [ ] **Step 1: Write the failing tests** (helper `row(planned, spent, history, elapsed, visits, budgeted)`; `days = 30`):

```rust
#[test] fn typical_needs_three_earlier_months_with_spending()
// history [0,400,430,0,460,90] → Some(430); [0,0,400,430,90] → None.
#[test] fn above_norm_at_the_boundary()
// typical 400, spent/elapsed giving heading 500 → Some(100); heading 499 → None; typical 60 heading 80 (≥1.25× but D=20) → None.
#[test] fn too_early_gives_no_heading_or_norm_finding_but_keeps_budget_fit()
// elapsed 6 → heading None, above_norm None, budget_fit still Some(Low) for planned 100, typical 430.
// visits 1 at elapsed 20 → same.
#[test] fn budget_fit_boundaries()
// typical 430: planned 344 → Low; 345 → None; planned 516 → High; 515 → None; typical 100 planned 120 (diff 20) → None; shadow row → None.
#[test] fn a_row_can_be_above_norm_and_under_budgeted()
// planned 100, typical 430, heading 580 → above_norm Some(150) and action Some(Set(430)).
#[test] fn a_shadow_row_above_norm_offers_a_budget()   // typical 150, heading 400 → Some(Add(150)); near norm → None.
#[test] fn a_refund_heavy_row_has_no_finding()          // spent -20 → heading Some(0) or None, above_norm None, no panic in drivers.
#[test] fn round_to_ten_rounds_half_up()               // 425→430, 424.99→420, 4→0, 5→10.
#[test] fn apply_sets_and_adds_without_touching_other_groups()
// Set(430) on food-out: only that amount changes. Add(150) for group "Shopping": id "shopping", source_groups ["Shopping"].
#[test] fn an_added_group_avoids_a_taken_id()           // plan already has id "shopping" → "shopping-2".
#[test] fn drivers_rank_by_pace_adjusted_excess_and_include_new_stores()
// elapsed 15 of 30: A spent 100 typical 60 (excess 70), B spent 40 typical 100 (−10, dropped), C spent 30 no history (30). → [A, C].
#[test] fn mostly_names_up_to_two_weekdays()            // [0,0,0,0,5,5,1] → "Fri, Sat"; [2,2,2,2,2,2,2] → "any day"; zeros → "any day".
#[test] fn busy_week_and_shown_thresholds()             // week 4 vs usual 2 → true; 3 vs 2 → false; recent 4 → shown, 3 → not.
```

- [ ] **Step 2: Run** `cargo test -p finance-ui --bins adjustments` — Expected: FAIL.
- [ ] **Step 3: Implement** the functions above.
- [ ] **Step 4: Run** `cargo test -p finance-ui --bins` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): adjustments findings, drivers and plan edits`.

### Task 5: The card on Plan & patterns

**Files:**
- Modify: `app/ui/src/pages/adjustments.rs` (add `#[component] pub fn AdjustmentsCard() -> impl IntoView`)
- Modify: `app/ui/src/pages/spending.rs` (`PlanPatterns`: render `<AdjustmentsCard />` after the "Review shared spending" button, before the variable switch)

**Interfaces:**
- Consumes: Task 4's functions; `report.adjustments`; `report.monthly_budgets` (the current-month `MonthlyBudget` of the same currency, for `plan` and `visuals::row_color`); `crate::format::days_in_month(&month)` as `visuals.rs:237` does; `invoke::<BudgetPlan>("save_budget_plan", &PlanArgs { plan })` then `state().refresh()`, as `overview/budget.rs:261`.

Layout and copy, from the spec:
- One `Card` per currency; hidden when no row has a `typical`.
- Header "Adjustments"; right side "€X above your norm this month" (sum of `above_norm`), hidden at zero.
- Flagged rows (any finding) first, ordered by `above_norm` desc then name: dot (`row_color` for budgeted, muted otherwise), name, "budget €B" or "no budget", "typical €T", "heading €H" or "too early to tell".
  - Above norm: "Heading to €H, about €D above a typical month." Budgeted rows add a link "See this month →" (`state().go(Tab::Overview)`).
  - `BudgetFit::Low`: "The budget is €B; a typical month is €T." `High`: "You usually spend €T; €(B−T) of this budget is spare."
  - Action button labels: "Set budget to €T", "Lower budget to €T", "Add a €T budget". First click swaps the button for "{name}: €B → €T · Confirm · Cancel" ("no budget → €T" for Add); Confirm saves. A failed save shows the command's error under the row ("Could not save this budget." when empty).
  - Above-norm rows hold a `<details>` "What drives it" with `drivers`: "{store}: {visits} visits × {avg}, usually {typical_visits} × {typical_avg}" or "{store}: new this month, {visits} visits, {spent}"; `avg = spent / visits`, usual visits to one decimal.
- "Unsorted" row when `unsorted > 0`: "€X not sorted yet", button "Review" → `Tab::Transactions`.
- "Near your norm (n)": a muted line of the remaining row names.
- "Habits" sub-table: rows where `shown(&habit)`, each a `<details>` over its shown stores; columns this week (bold accent when `busy_week`), usual/wk (one decimal, "<1" under 1, "0" at 0), this month, usual/mo ("—" when `None`), avg ticket ("—" at zero visits), mostly. Footer "Less often (n)" counting unshown rows.
- Amounts through the existing `money`/`Money` helpers; whole euros for budget/typical/heading.

- [ ] **Step 1: Implement** the component and wire it into `PlanPatterns`.
- [ ] **Step 2: Run** `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings` and `cargo test --locked -p finance-ui --bins` — Expected: PASS.
- [ ] **Step 3: Check in the dev server on a copy.**

```bash
cp "$HOME/Library/Application Support/Finance/finance.sqlite3" "$SCRATCH/ledger-copy.sqlite3"
(cd app/ui && trunk build) && cargo build -p finance-devserver
DEV_DB="$SCRATCH/ledger-copy.sqlite3" ./target/debug/finance-devserver app/ui/dist 1431
```

Open `http://127.0.0.1:1431`, Spending → Plan & patterns. Expected: Food & drinks out shows budget 100 with a "Set budget" action; Shopping appears as an unbudgeted row; no row named Transfers, Investments or Other; Habits lists Albert Heijn To Go under its group. Click "Set budget", Confirm: the row's budget changes and Overview's budget table shows the same amount; `POST /invoke/dashboard` on the copy shows next month's budget with the new amount (carry-forward). Compare one row's "heading" with Overview's bar for the same group: equal.

- [ ] **Step 4: Commit** `feat(spending): the Adjustments card with visit habits`.

### Task 6: Full check and PR

- [ ] **Step 1: Run** the CI set: `cargo fmt --all --check`; both clippy lines from `.github/workflows/ci.yml`; `cargo test --locked -p finance-storage -p finance-types -p finance-core -p finance-bank -p finance-notify -p finance-timesfm -p finance-ai`; `cargo test --locked -p finance-ui --bins`. Expected: all PASS.
- [ ] **Step 2:** Push the branch and open the PR (title `feat(spending): adjustments against your own norm, with visit habits`); body covers why, what changed, the carry-forward side effect on Overview, and testing.
