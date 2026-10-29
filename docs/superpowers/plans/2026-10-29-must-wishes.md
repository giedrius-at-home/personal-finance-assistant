# Must Wishes, Honest Free Money, Flexible Saving, Several Cards — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wishes can be flagged "must" and are judged first; free money for wishes stops holding back savings-goal contributions; set-asides skip tight months and catch up later; the card becomes a list of cards with pooled room and per-card payback.

**Architecture:** The pure engine in `crates/core/src/wishes/` changes in three places. `baseline` adds goal contributions back to later months' free money and seeds a per-card usage list. `schedule` stops pre-computing set-asides; a new `Saver` hands the ladder each month's set-asides and purchases from what is still unsaved, and records skips. `ladder::judge` drives the months, runs must demands before optional ones, tries a set-aside without the cards and rolls back to skip it, and fills cards in list order. Types gain `must` on wishes, a `cards` list on settings, per-card charges on a purchase, and per-card usage on each judged month. The UI gets a Must toggle, a cards list, per-card amounts when buying on card, a payback line per month, and the new sentences.

**Tech Stack:** Rust 2024 workspace: `finance-types` (serde types), `finance-core` (engine, writes, service), `finance-storage` (SQLite JSON payload rows), `finance-ui` (Leptos, wasm), Tauri commands in `app/src-tauri/src/main.rs`. `rust_decimal::Decimal`, `jiff::civil::Date`.

**Spec:** `docs/superpowers/specs/2026-10-28-wishes-design.md`, section "Addendum (2026-10-29)" and "Settled after the builder check".

## Global Constraints

- All money is `Decimal`; round shares with `.round_dp(2)` as the existing code does.
- Stored JSON must keep parsing: every new field on `Wish`, `WishSettings`, `WishPurchase`, `Verdict`, `WishMonth` is `#[serde(default)]` or converted from the old shape. Existing tests `old_wish_json_without_kind_is_one_off` and `stored_rows_round_trip` must keep passing.
- Default settings: one card named "Credit card", limit 2500, used 0.
- Sentences (exact copy from the spec): "Nothing set aside in {Month}; {X}/month from {Month}.", "{X} on the cards, so {Month} starts {X} short.", "Pay back {Y} on 1 {Month} ({Card} {a}, {Card} {b}).", free-money line: "Free money each month: salary minus bills, planned payments and budget groups. Savings goals are not held back."
- CI gate before every commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings`, `cargo test -p finance-types -p finance-storage -p finance-core`, `cargo test -p finance-ui --bins`.
- Commit messages: conventional, end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. A stored `wish_settings` row in the old single-card shape must load as one card and never as an empty list (Task 1 test `old_settings_json_becomes_one_card`).
2. A must set-aside in the final month before its date is the purchase, never a skip; a skip must never push a wish past its date (Task 4 test `must_set_aside_never_skips_past_due`).
3. A card wish larger than all cards' pooled room falls back to free money and cuts, then short, with usage never above any card's limit (Task 4 test `card_fill_never_exceeds_limit`).
4. Flagging a sub-wish must when its parent is optional is refused; un-flagging a parent cascades (Task 6 test `must_cascades_and_is_bounded_by_parent`).
5. Marking a wish bought on cards with amounts that do not sum to the purchase is refused; an empty list fills cards in order (Task 7 test `bought_card_amounts_must_sum`).

---

### Task 1: Types — `must`, cards list, per-card purchase, per-card month usage

**Files:**
- Modify: `crates/types/src/gateway.rs` (`Wish`, `WishDraft`, `WishSettings`, `WishPurchase`, tests module)
- Modify: `crates/types/src/report/wishes.rs` (`Verdict`, `WishMonth`, `WishesView`)

**Interfaces:**
- Produces:
  ```rust
  // gateway.rs
  pub struct CardSettings { pub card_id: String, pub name: String, pub limit: Decimal, pub used: Decimal, /// `YYYY-MM-01` the usage was entered for
      pub used_month: String, pub bill_id: Option<String> }
  #[serde(from = "WishSettingsJson")]
  pub struct WishSettings { pub cards: Vec<CardSettings> }   // Default: one card id "card-1", name "Credit card", limit 2500
  pub struct CardCharge { pub card_id: String, pub amount: Decimal }
  // on WishPurchase: #[serde(default)] pub cards: Vec<CardCharge>
  // on Wish and WishDraft: #[serde(default)] pub must: bool
  // report/wishes.rs
  pub struct CardUsed { pub card_id: String, pub name: String, pub used: Decimal }
  // on Verdict: #[serde(default)] pub must: bool, #[serde(default)] pub skipped: bool
  // on WishMonth: #[serde(default)] pub cards: Vec<CardUsed>   // end-of-month usage per card = next month's payback
  // on WishesView: #[serde(default)] pub stale_cards: Vec<String>  // card ids whose usage is from an earlier month
  ```
  `WishSettingsJson` is a private serde struct with `#[serde(default)] cards: Vec<CardSettings>` plus the four legacy fields as `Option` (`card_limit`, `card_used`, `card_used_month`, `card_bill_id`). `From<WishSettingsJson>`: non-empty `cards` wins; otherwise a legacy `card_limit` makes one card ("card-1", "Credit card", those values); otherwise the default.
- `WishesView.card_stale` is kept and means `!stale_cards.is_empty()`.

- [ ] **Step 1: Write the failing tests** in `crates/types/src/gateway.rs` tests module

```rust
#[test]
fn old_settings_json_becomes_one_card() {
    let json = r#"{"card_limit":"3000","card_used":"120","card_used_month":"2026-10-01","card_bill_id":"b1"}"#;
    let s: WishSettings = serde_json::from_str(json).unwrap();
    assert_eq!(s.cards.len(), 1);
    assert_eq!(s.cards[0].name, "Credit card");
    assert_eq!(s.cards[0].limit, Decimal::from(3000));
    assert_eq!(s.cards[0].used, Decimal::from(120));
    assert_eq!(s.cards[0].bill_id.as_deref(), Some("b1"));
    let back = serde_json::to_string(&s).unwrap();
    assert!(back.contains("\"cards\"") && !back.contains("card_limit"));
}

#[test]
fn old_wish_json_is_not_must_and_purchase_has_no_cards() {
    let json = r#"{"wish_id":"w","name":"Bike","amount":"900","currency":{"code":"EUR","minor_unit":2},"due_on":"2026-12-01","source":{"kind":"card"},"created_at":"x"}"#;
    let wish: Wish = serde_json::from_str(json).unwrap();
    assert!(!wish.must);
    let p: WishPurchase = serde_json::from_str(r#"{"amount":"10","on":"2026-10-01","source":{"kind":"card"}}"#).unwrap();
    assert!(p.cards.is_empty());
    assert_eq!(WishSettings::default().cards[0].limit, Decimal::from(2500));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-types old_settings_json_becomes_one_card old_wish_json_is_not_must`
Expected: compile error (no `cards`, no `must`).

- [ ] **Step 3: Add the types** as in Interfaces. Fix every construction site that now fails to compile across the workspace (`crates/core/src/wishes/store.rs` tests, `crates/core/src/writes/wishes.rs`, `app/ui/src/pages/wishes.rs`) with the minimal edit: `must: false`, `cards: Vec::new()`, and replace `WishSettings { card_limit, .. }` literals with `WishSettings { cards: vec![CardSettings { .. }] }`. Leave behaviour changes for later tasks; where code read `settings.card_limit`/`card_used`/`card_used_month`/`card_bill_id`, add a temporary helper on `WishSettings`:
  ```rust
  impl WishSettings { pub fn first(&self) -> &CardSettings  // panics never: Default guarantees one card; use cards.first().unwrap_or(&DEFAULT_CARD) via a const/lazy
  ```
  Tasks 3, 4, 8 and 9 remove the callers of `first()`; Task 9 deletes it.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test -p finance-types && cargo test -p finance-core -p finance-storage && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): types for must wishes, a cards list and per-card charges"
```

---

### Task 2: Baseline — free money ignores goal contributions, per-card seeding

**Files:**
- Modify: `crates/core/src/wishes/baseline.rs`
- Modify: `crates/core/src/wishes/ladder.rs` (`MonthBaseline` only)
- Modify: `crates/core/src/wishes/mod.rs` (`Inputs.card_bill`, `dashboard_parts`)

**Interfaces:**
- Produces:
  ```rust
  // ladder.rs
  #[derive(Clone, Debug)]
  pub struct CardRoom { pub card_id: String, pub name: String, pub limit: Decimal, pub used: Decimal }
  pub struct MonthBaseline { pub month: Date, pub free: Decimal, pub groups: Vec<GroupRoom>, pub cards: Vec<CardRoom> }  // `card_used` field removed
  // baseline.rs
  pub fn baselines(alloc: &CurrencyAllocation, plans: &[BudgetPlan], card_bills: &[(String, Decimal)], settings: &WishSettings, earmarked: Decimal, months: usize) -> (Vec<MonthBaseline>, Vec<String>)
  // returns (months, stale card ids). `card_bills` is (card_id, that card's linked bill amount), one entry per card that has a linked bill.
  // mod.rs
  pub struct Inputs<'a> { ..., pub card_bills: Vec<(String, Decimal)> }  // replaces card_bill
  ```
- Rules: month 0 `free = current_wiggle_room − earmarked` (unchanged). Later months `free = next_wiggle_room − next_shortfall + next_goals + Σ card_bills`. Month 0 `cards[k].used` = the typed usage when `used_month == month0`, else 0 (and the id goes in stale unless the card's `used_month` is empty and `used` is zero). Later months `cards[k].used` = that card's linked bill amount or 0.

- [ ] **Step 1: Write the failing tests** in `baseline.rs` tests (reuse the module's `alloc`, `plan`, `row` helpers; set `next_goals` on the alloc)

```rust
#[test]
fn later_months_add_goal_contributions_back() {
    let mut a = alloc(/* as in existing tests */);
    a.next_wiggle_room = d("100"); a.next_shortfall = Decimal::ZERO; a.next_goals = d("250");
    let (months, _) = baselines(&a, &[], &[], &WishSettings::default(), Decimal::ZERO, 3);
    assert_eq!(months[1].free, d("350"));
    assert_eq!(months[2].free, d("350"));
}

#[test]
fn cards_seed_from_their_own_bills_and_stale_is_per_card() {
    let settings = WishSettings { cards: vec![
        card("visa", "2026-10-01", "40", Some("b1")),     // helper: CardSettings with limit 2500
        card("mc",   "2026-09-01", "70", None),
    ]};
    let (months, stale) = baselines(&alloc_oct(), &[], &[("visa".into(), d("500"))], &settings, Decimal::ZERO, 3);
    assert_eq!(months[0].cards[0].used, d("40"));
    assert_eq!(months[0].cards[1].used, Decimal::ZERO);
    assert_eq!(stale, vec!["mc".to_string()]);
    assert_eq!(months[1].cards[0].used, d("500"));
    assert_eq!(months[1].cards[1].used, Decimal::ZERO);
    assert_eq!(months[1].free, alloc_oct().next_wiggle_room + alloc_oct().next_goals + d("500"));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core wishes::baseline`
Expected: compile error (`cards`, `next_goals` in free, signature).

- [ ] **Step 3: Implement** the new `baselines` signature and `MonthBaseline.cards`; in `mod.rs` change `dashboard_parts` to return `Vec<(String, Decimal)>` by looking up each card's `bill_id` in `next_month_subscriptions`/`subscriptions` rows (`amount_max`), and thread `card_bills` through `Inputs`. Update existing baseline/mod tests that asserted the old `card_used` or `next_month_shortfall_makes_free_negative` numbers to the new rule (that test's alloc has `next_goals` 0, so its numbers stand; `later_months_start_from_their_card_usage` in ladder moves to Task 4).

- [ ] **Step 4: Run tests**

Run: `cargo test -p finance-core wishes`
Expected: baseline and mod tests PASS; ladder tests may fail on `MonthBaseline` construction — fix their literals to `cards: vec![]` only, no behaviour yet.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): free money keeps goal contributions; cards seed from their own bills"
```

---

### Task 3: Schedule — `Saver` replaces pre-computed set-asides; `must` on demands

**Files:**
- Modify: `crates/core/src/wishes/schedule.rs`
- Modify: `crates/core/src/wishes/ladder.rs` (`Demand` only)

**Interfaces:**
- Produces:
  ```rust
  // ladder.rs: on Demand: pub must: bool
  // schedule.rs
  pub struct Planned { ..., pub must: bool }                  // from Wish.must; a sub-wish or rest is must iff its parent is
  pub fn repeat_demands(items: &[Planned], current_month: Date, months: usize) -> Vec<Vec<Demand>>
  //   only items with per_month; negative amounts (freed) first, then costs; replaces `demands` for repeats.
  pub struct Saver { /* one entry per one-off Planned with remaining > 0 and due within horizon */ }
  impl Saver {
      pub fn new(items: &[Planned], current_month: Date, months: usize) -> Self;
      /// Set-asides this month: one per item whose due month is after `month`, amount = round2(remaining / months_left) where
      /// months_left = due_idx − month + 1 − (1 if month == 0 && saved_this_month); none when the amount rounds to 0.
      pub fn set_asides(&self, month: usize) -> Vec<Demand>;
      /// Purchases due in `month`: each item's whole remaining, sorted by (due, created_at); on_card from the item.
      pub fn purchases(&self, month: usize) -> Vec<Demand>;
      /// `later_free[j]` is month (month+1+j)'s baseline free, up to the item's due month. True when
      /// remaining / (due_idx − month) ≤ min(later_free), i.e. a must set-aside may skip `month`.
      pub fn may_skip_must(&self, wish_id: &str, month: usize, later_free: &[Decimal]) -> bool;
      /// Record that `amount` of `wish_id` was paid (set-aside or purchase) in `month`.
      pub fn paid(&mut self, wish_id: &str, amount: Decimal);
      pub fn remaining(&self, wish_id: &str) -> Decimal;
  }
  ```
- `demands()` is deleted; `plan_items` and `earmarked` unchanged except `must` propagation.

- [ ] **Step 1: Write the failing tests** (replace the old `spreads_over_months_left`, `saved_reduces_set_aside`, `due_month_takes_rounding_remainder`, `ordering_set_asides_then_due_by_date_then_created`, `saving_recorded_this_month_skips_month_zero`, `parent_rest_saved_this_month_skips_month_zero` with `Saver` equivalents; keep their numbers)

```rust
#[test]
fn saver_spreads_then_recomputes_after_a_skip() {
    // 900 due in 2 months (idx 2), nothing saved: 300 / 300 / 300.
    let items = vec![planned("w", "900", "2026-12-15")];
    let mut s = Saver::new(&items, oct(), 3);
    assert_eq!(s.set_asides(0)[0].amount, d("300"));
    // Month 0 skipped: nothing paid. Month 1 now spreads 900 over 2.
    assert_eq!(s.set_asides(1)[0].amount, d("450"));
    s.paid("w", d("450"));
    assert_eq!(s.set_asides(2), vec![]);               // due month: purchase, not set-aside
    assert_eq!(s.purchases(2)[0].amount, d("450"));
}

#[test]
fn must_set_aside_may_skip_only_when_later_months_can_carry_it() {
    let items = vec![planned_must("w", "900", "2026-12-15")];
    let s = Saver::new(&items, oct(), 3);
    assert!(s.may_skip_must("w", 0, &[d("450"), d("500")]));
    assert!(!s.may_skip_must("w", 0, &[d("449"), d("500")]));
    assert!(!s.may_skip_must("w", 2, &[]));             // nothing after the due month
}

#[test]
fn must_flag_flows_to_demands_and_parts() {
    let parent = wish_must("p", None, "1000", "2027-01-10");
    let sub = wish("s", Some("p"), "400", "2026-12-10");   // not flagged itself
    let (items, _) = plan_items(&[parent, sub], oct(), &HashMap::new());
    assert!(items.iter().all(|i| i.must));
    let s = Saver::new(&items, oct(), 4);
    assert!(s.set_asides(0).iter().all(|d| d.must));
}

#[test]
fn repeat_demands_keep_freed_first_and_carry_must() {
    // existing recurring/bill-change fixtures: assert the negative delta comes first and `must` copies the item's flag
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core wishes::schedule`
Expected: compile error (no `Saver`).

- [ ] **Step 3: Implement `Saver`, `repeat_demands`, `must` on `Planned`/`Demand`;** delete `demands`. `plan_items`: a leaf's `must = w.must || parent.must`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p finance-core wishes::schedule`
Expected: PASS. (`mod.rs` no longer compiles: it calls `demands`. Stub by calling `repeat_demands` and leaving set-asides out until Task 4; mark with `// Task 4` and keep `cargo build` green.)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): saver hands the ladder each month's set-asides"
```

---

### Task 4: Ladder — pooled cards, must tiers, skip-and-catch-up

**Files:**
- Modify: `crates/core/src/wishes/ladder.rs`
- Modify: `crates/core/src/wishes/mod.rs` (`Inputs::judge`, `RowContext::set_aside`)

**Interfaces:**
- Consumes: `Saver`, `repeat_demands` (Task 3); `MonthBaseline.cards` (Task 2).
- Produces:
  ```rust
  pub fn judge(months: &[MonthBaseline], repeats: &[Vec<Demand>], saver: &mut Saver) -> Vec<WishMonth>
  ```
  `card_limit` parameter removed. Per month, in order:
  1. "Already short" and last month's card charge, as today. Card charge amount = Σ previous month's `cards[].used`. `WishMonth.card_used` = Σ cards used; `WishMonth.cards` = per-card end usage.
  2. Freed (negative) repeats.
  3. Tier must, then tier optional. Within a tier: set-asides from `saver.set_asides(i)` filtered by `must`, then `saver.purchases(i)`, then repeat costs. Every paid set-aside/purchase calls `saver.paid`.
  4. A set-aside is first tried with `card_allowed = false` on a cloned purse. If `short == 0`, keep. Else roll back and: optional → skip; must → skip iff `saver.may_skip_must(id, i, later_free)` where `later_free` = `months[i+1..=due].free`; otherwise pay for real with `card_allowed = true`. A skip pushes `Verdict { amount: 0, set_aside: true, skipped: true, .. }` and calls nothing on the saver.
  5. Card fill: `Purse.cards: Vec<(CardRoom, Decimal used)>`; `card_room()` = Σ max(limit − used, 0); taking fills cards in list order up to each limit.
- `RowContext::set_aside(wish_id)` now reads month 0's set-aside verdicts (amount, skipped excluded) instead of demands; `Inputs::judge` returns `(Vec<WishMonth>, Vec<String> stale)`.

- [ ] **Step 1: Write the failing tests** in `ladder.rs` tests (helpers: `base(free, cards)`, `saver_for(...)`, two cards "visa" limit 100 / "mc" limit 200)

```rust
#[test]
fn must_wish_is_paid_before_an_optional_one() {
    // free 100; optional purchase 80 created first, must purchase 80 due same month, no cards, no groups.
    let out = judge(&[base("100", vec![])], &[vec![]], &mut saver(vec![purchase("opt", "80"), purchase_must("must", "80")]));
    let v = &out[0].verdicts;
    assert_eq!(v[0].wish_id.as_deref(), Some("must")); assert_eq!(v[0].short, Decimal::ZERO);
    assert_eq!(v[1].wish_id.as_deref(), Some("opt"));  assert_eq!(v[1].short, d("60"));
}

#[test]
fn optional_set_aside_skips_a_tight_month_and_catches_up() {
    // free 100 / 400 / 400; wish 600 due month 2 → 200 each; month 0 cannot pay 200 without cards.
    let out = judge(&bases(["100", "400", "400"]), &[vec![], vec![], vec![]], &mut saver(vec![due_in("w", "600", 2)]));
    assert!(out[0].verdicts[0].skipped && out[0].verdicts[0].amount.is_zero());
    assert_eq!(out[1].verdicts[0].amount, d("300"));   // 600 over the 2 months left
    assert_eq!(out[2].verdicts[0].amount, d("300"));
    assert!(out.iter().all(|m| m.verdicts.iter().all(|v| v.short.is_zero() && v.on_card.is_zero())));
}

#[test]
fn must_set_aside_pays_on_the_cards_when_later_months_cannot_carry_it() {
    // free 100 / 250 / 250; must 600 due month 2 → 200 each; skipping would need 300 > 250, so month 0 pays 200: 100 free + 100 card.
    let out = judge(&bases_with_cards(["100", "250", "250"]), &[vec![]; 3], &mut saver(vec![due_in_must("w", "600", 2)]));
    assert_eq!(out[0].verdicts[0].on_card, d("100"));
    assert!(!out[0].verdicts[0].skipped);
}

#[test]
fn must_set_aside_never_skips_past_due() {
    // free 0 then 0; must 100 due month 1. Month 0 cannot carry (later free 0): pays on card. Month 1 purchase: remaining 0 → nothing due.
    let out = judge(&bases_with_cards(["0", "0"]), &[vec![]; 2], &mut saver(vec![due_in_must("w", "100", 1)]));
    assert_eq!(out[0].verdicts.iter().map(|v| v.on_card).sum::<Decimal>(), d("100"));
}

#[test]
fn card_fill_never_exceeds_limit() {
    // visa limit 100 used 30, mc limit 200 used 0; card purchase 250 → visa 70, mc 180; then 400 more → mc 20 then free/cuts/short.
    let out = judge(&[base_cards("0", vec![("visa", "100", "30"), ("mc", "200", "0")])], &[vec![]], &mut saver(vec![card_purchase("a", "250"), card_purchase("b", "400")]));
    let m = &out[0];
    assert_eq!(m.cards[0].used, d("100")); assert_eq!(m.cards[1].used, d("200"));
    assert_eq!(m.verdicts[1].short, d("380"));
    assert_eq!(m.card_used, d("300"));
}

#[test]
fn next_month_charge_is_the_sum_of_every_card() {
    // month 0 ends visa 60 / mc 90 → month 1 "Credit card" verdict amount 150, paid from free.
}
```
Also port `later_months_start_from_their_card_usage` and `card_charge_shortfall_split_by_on_card` to the per-card baseline; keep their numbers.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core wishes::ladder`
Expected: compile errors (signature, `skipped`, `cards`).

- [ ] **Step 3: Implement** per the Interfaces block. `Purse` derives `Clone` (owns `Vec<(usize, Decimal)>` indices into `base.groups` rather than references, so the clone is cheap and borrow-free). Wire `Inputs::judge` in `mod.rs`: `let mut saver = Saver::new(&items, month0, months); let repeats = repeat_demands(&items, month0, months); ladder::judge(&bases, &repeats, &mut saver)`. `purchase_cuts` keeps working through the same path.

- [ ] **Step 4: Run tests, including `mod.rs` tests and the UI verdict tests**

Run: `cargo test -p finance-core wishes && cargo test -p finance-ui --bins`
Expected: PASS. Any `mod.rs` test whose expected numbers depended on goal contributions being held back is updated to the spec rule, with a comment naming the addendum.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): must first, set-asides skip tight months, cards pooled in list order"
```

---

### Task 5: View — stale cards, per-card payback, free-money note

**Files:**
- Modify: `crates/core/src/wishes/mod.rs` (`Inputs::view`)
- Modify: `crates/core/src/wishes/store.rs` tests (settings round trip with two cards)

**Interfaces:**
- Produces: `WishesView { stale_cards, card_stale: !stale_cards.is_empty(), months[i].cards, .. }` filled from Task 2/4 outputs.

- [ ] **Step 1: Write the failing test** in `mod.rs` tests

```rust
#[test]
fn view_reports_stale_cards_and_per_card_payback() {
    // settings: visa (used_month = this month, used 50), mc (used_month last month, used 10). One card wish 120 due now.
    let view = Inputs { .. }.view(&[card_wish("w", "120", today)], &settings);
    assert_eq!(view.stale_cards, vec!["mc".to_string()]);
    assert!(view.card_stale);
    assert_eq!(view.months[0].cards.iter().map(|c| c.used).sum::<Decimal>(), d("170"));
    assert_eq!(view.months[1].card_charge_in, d("170"));
}
```
And in `store.rs` `stored_rows_round_trip`: settings with two `CardSettings` round-trip equal.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p finance-core wishes::tests::view_reports_stale_cards`
Expected: FAIL (fields empty).

- [ ] **Step 3: Fill the fields in `Inputs::view`.**

- [ ] **Step 4: Run tests**

Run: `cargo test -p finance-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): view carries stale cards and per-card payback"
```

---

### Task 6: Writes — must flag with cascade, card list settings

**Files:**
- Modify: `crates/core/src/writes/wishes.rs` (`save_wish`, new `set_must`, `save_wish_settings`)
- Modify: `crates/core/src/service.rs` (new `set_wish_must`)
- Modify: `app/src-tauri/src/main.rs` (new command `set_wish_must`, registered in the handler list)

**Interfaces:**
- Produces:
  ```rust
  // writes/wishes.rs
  pub fn set_must(w: &Writer<'_>, wish_id: &str, must: bool) -> Result<Wish>
  //   planned wish only. must=true on a sub-wish whose parent is not must → Err(invalid("only parts of a must wish can be must")).
  //   must=false on a parent → every planned sub-wish is set must=false too. Returns the wish.
  // save_wish: honours draft.must with the same parent rule.
  // save_wish_settings: each card needs a non-empty name ≤ 64 chars, limit > 0, used ≥ 0, used_month `YYYY-MM-01` parseable; ids unique and non-empty
  //   (a card with an empty card_id gets a fresh id `card-<ulid or uuid, whichever the crate already uses for wish ids>`); at least one card.
  // service.rs
  pub fn set_wish_must(&self, wish_id: &str, must: bool) -> Result<Wish, String>
  // main.rs
  #[tauri::command] fn set_wish_must(service: State<'_, Service>, wish_id: String, must: bool) -> Result<Wish, String>
  ```

- [ ] **Step 1: Write the failing tests** in `writes/wishes.rs` tests (use the module's `db()` fixture and `draft()` helper)

```rust
#[test]
fn must_cascades_and_is_bounded_by_parent() {
    let db = db();
    let parent = save_wish(&db.writer(), &db.snap, draft("Trip")).unwrap();
    let sub = save_wish(&db.writer(), &db.snap, WishDraft { parent_id: Some(parent.wish_id.clone()), ..draft("Flights") }).unwrap();
    assert!(set_must(&db.writer(), &sub.wish_id, true).is_err());
    set_must(&db.writer(), &parent.wish_id, true).unwrap();
    set_must(&db.writer(), &sub.wish_id, true).unwrap();
    set_must(&db.writer(), &parent.wish_id, false).unwrap();
    let wishes = load_wishes(db.store.connection()).unwrap();
    assert!(wishes.iter().all(|w| !w.must));
}

#[test]
fn settings_need_valid_cards_and_mint_ids() {
    let db = db();
    let bad = WishSettings { cards: vec![] };
    assert!(save_wish_settings(&db.writer(), bad).is_err());
    let saved = save_wish_settings(&db.writer(), WishSettings { cards: vec![card("", "Visa", "1000", "0", "2026-10-01"), card("mc", "MC", "500", "20", "2026-10-01")] }).unwrap();
    assert!(!saved.cards[0].card_id.is_empty());
    assert_eq!(saved.cards[1].card_id, "mc");
    assert!(save_wish_settings(&db.writer(), WishSettings { cards: vec![card("x", "", "1000", "0", "2026-10-01")] }).is_err());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-core writes::wishes`
Expected: compile error (`set_must`).

- [ ] **Step 3: Implement** `set_must`, the `must` handling in `save_wish`, the new `save_wish_settings` validation, the service method and Tauri command.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test -p finance-core && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): must flag with cascade; card list settings"
```

---

### Task 7: Writes — bought on several cards

**Files:**
- Modify: `crates/core/src/writes/wishes.rs` (`mark_bought`)

**Interfaces:**
- Consumes: `WishPurchase.cards`, `CardSettings` (Task 1).
- Produces: `mark_bought` behaviour: when `purchase.source == Card`: if `cards` is empty, fill in settings list order (each up to `limit − used` for the purchase month; the last card takes any overflow) and store the filled list on the wish's `bought`. If non-empty: every `card_id` must exist and `Σ amount == purchase.amount` else `Err(invalid("card amounts must add up to the purchase"))`. Each card's `used`/`used_month` is raised by the same month rule the code applies today, per card.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn bought_card_amounts_must_sum() {
    let db = db_with_cards(&[("visa", "100", "30"), ("mc", "200", "0")]);   // helper: saves settings for this month
    let w = save_wish(&db.writer(), &db.snap, draft("Bike")).unwrap();
    let bad = WishPurchase { amount: d("150"), on: today(), source: WishSource::Card, cards: vec![CardCharge { card_id: "visa".into(), amount: d("100") }] };
    assert!(mark_bought(&db.writer(), &db.snap, &w.wish_id, bad, &[]).is_err());
    let auto = WishPurchase { amount: d("150"), on: today(), source: WishSource::Card, cards: vec![] };
    let bought = mark_bought(&db.writer(), &db.snap, &w.wish_id, auto, &[]).unwrap();
    let charges = bought.bought.unwrap().cards;
    assert_eq!(charges, vec![CardCharge { card_id: "visa".into(), amount: d("70") }, CardCharge { card_id: "mc".into(), amount: d("80") }]);
    let s = load_wish_settings(db.store.connection()).unwrap();
    assert_eq!(s.cards[0].used, d("100")); assert_eq!(s.cards[1].used, d("80"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p finance-core bought_card_amounts_must_sum`
Expected: FAIL.

- [ ] **Step 3: Implement** in `mark_bought`; existing single-card tests (`card_used` assertions) move to `cards[0].used`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p finance-core writes::wishes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(wishes): buying on card records which cards took it"
```

---

### Task 8: UI — must toggle and tag, free-money note, new sentences

**Files:**
- Modify: `app/ui/src/pages/wishes.rs` (`add_form`, `wish_body`, `page`, `one_off_line`, `verdict_line`, tests)

**Interfaces:**
- Consumes: `set_wish_must` command (Task 6), `Verdict.skipped`, `WishMonth.cards` (Tasks 1/4).
- Produces:
  - Add form: a "Must happen" checkbox bound to a new `ui.must: RwSignal<bool>`, sent as `draft.must`; reset on success.
  - Wish row: a small "Must" tag beside the name when `wish.must`; a text button "Mark must" / "Not a must" in the row's action strip that calls `set_wish_must` through `ui.write("must-<id>", "set_wish_must", MustArgs { wish_id, must }, "Saved.", || {})`, where `#[derive(Serialize)] struct MustArgs { wish_id: String, must: bool }`.
  - `page`: one `<p class="text-sm text-muted-fg">` under the "Planned purchases" heading with the free-money sentence from Global Constraints.
  - `one_off_line`: when a month's verdict for the row is `skipped`, prefix "Nothing set aside in {Month}; {X}/month from {Next}." where X is the next month's set-aside amount for the row; the on-card sentence uses "on the cards" wording: "{n} on the cards, so {next} starts {short} short." and "Affordable with {n} on the cards — {next} starts {n} lighter…".

- [ ] **Step 1: Write the failing tests** in the page's tests module (reuse its `row`, `d` helpers; `month(...)` helper building a `WishMonth`)

```rust
#[test]
fn skipped_month_is_said_then_catch_up() {
    let mut r = row("w"); r.wish.kind = WishKind::OneOff;
    let months = vec![
        month("2026-11-01", vec![verdict_skipped("w")]),
        month("2026-12-01", vec![verdict_set_aside("w", d(30000, 2))]),
        month("2027-01-01", vec![verdict_set_aside("w", d(30000, 2))]),
    ];
    assert_eq!(verdict_line(&r, &months), "Nothing set aside in November; €300.00/month from December.");
}

#[test]
fn card_sentence_says_cards() {
    // existing `on_card` fixture → "Not affordable — €566.00 on the cards, so November starts €30.48 short."
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p finance-ui --bins skipped_month_is_said`
Expected: FAIL.

- [ ] **Step 3: Implement** the four UI pieces above.

- [ ] **Step 4: Run tests and wasm clippy**

Run: `cargo test -p finance-ui --bins && cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(ui): must wishes, free-money note, skipped-month wording"
```

---

### Task 9: UI — cards list, per-card buying, payback line; delete `WishSettings::first`

**Files:**
- Modify: `app/ui/src/pages/wishes.rs` (`card_strip`, bought form inside `wish_body`, `month_card`)
- Modify: `crates/types/src/gateway.rs` (remove `WishSettings::first`)

**Interfaces:**
- Consumes: `CardSettings`, `CardCharge`, `WishMonth.cards`, `WishesView.stale_cards`.
- Produces:
  - `card_strip` becomes "CREDIT CARDS": one row per card with Name, Limit, Used this month, Card bill (select), a "Remove" text button (disabled when it is the only card), an "Add card" button appending a blank row, and one "Save cards" submit sending the whole list as `WishSettings { cards }` with `used_month = current_month` on every row. Rows keep list order; order is the fill order (hint text under the heading: "Charges fill the cards in this order."). The stale notice names the stale cards: "Usage for {names} was entered for an earlier month; until re-entered it counts as €0."
  - Bought form: when the chosen source is the card and there is more than one card, show one amount field per card prefilled by list order from the typed amount (each up to `limit − used`, last takes the rest); on submit send `cards: Vec<CardCharge>` and refuse locally with "Card amounts must add up to the purchase." when they do not sum. With one card, send `cards: vec![]`.
  - `month_card`: below the stats, "Pay back {total} on 1 {NextMonth} ({Name} {a}, {Name} {b})." built from `month.cards` (cards with zero usage omitted; with one card no parenthesis).

- [ ] **Step 1: Write the failing test** for the payback sentence as a pure helper `fn payback_line(month: &WishMonth, code: &str) -> Option<String>`

```rust
#[test]
fn payback_line_names_each_card() {
    let mut m = month("2026-11-01", vec![]);
    m.cards = vec![card_used("visa", "Visa", d(6000, 2)), card_used("mc", "Mastercard", d(9000, 2)), card_used("x", "Amex", Decimal::ZERO)];
    m.card_used = d(15000, 2);
    assert_eq!(payback_line(&m, "EUR").unwrap(), "Pay back €150.00 on 1 December (Visa €60.00, Mastercard €90.00).");
    m.cards.truncate(1); m.card_used = d(6000, 2);
    assert_eq!(payback_line(&m, "EUR").unwrap(), "Pay back €60.00 on 1 December.");
    m.card_used = Decimal::ZERO; m.cards.clear();
    assert!(payback_line(&m, "EUR").is_none());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p finance-ui --bins payback_line_names_each_card`
Expected: FAIL.

- [ ] **Step 3: Implement** `payback_line`, the cards list form, the per-card bought fields; delete `WishSettings::first` and fix any remaining caller.

- [ ] **Step 4: Full CI gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings && cargo test -p finance-types -p finance-storage -p finance-core && cargo test -p finance-ui --bins`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(ui): several credit cards, per-card buying and payback"
```

---

### Task 10: Run the app and check the owner's screenshot case

**Files:** none.

- [ ] **Step 1: Build and open the Wishes tab** (the `run` skill; `cargo tauri dev` or the devserver per README).
- [ ] **Step 2: Check** that the four wishes from the owner's screenshot no longer all read "Not affordable" when November's free money includes the goal contribution, that flagging one "Must" moves it above the others in the month's verdict order, that the cards list saves two cards and buying on card offers two amount fields, and that each month card shows the payback line.
- [ ] **Step 3: Report** what was seen, with a screenshot, in the final message. No commit.
