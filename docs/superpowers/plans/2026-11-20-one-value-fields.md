# One Value Per Field Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overview's "What could be better" fields stop pairing two values with " · ", and the Wishes rows show their verdict as labelled fields instead of a sentence.

**Architecture:** Both are UI-only changes over reports that already carry the numbers. `nudges.rs` pushes two facts where it pushed one joined string; `wishes.rs` replaces `verdict_line -> String` with `verdict_facts -> Vec<(String, String)>` rendered by the existing `FactsRow`.

**Tech Stack:** Rust, Leptos (`app/ui`, crate `finance-ui`), `rust_decimal`.

**Spec:** `docs/superpowers/specs/2026-11-20-events-design.md`, parts 1 and 2.

## Global Constraints

- cargo is not on PATH: `export PATH="$HOME/.rustup/toolchains/$(ls ~/.rustup/toolchains | head -1)/bin:$PATH"` before any cargo command.
- Tests cover calculations, not wording: assert on which facts exist and their values; do not add tests for label text beyond what a fact's identity needs.
- No " · " and no sentence in anything this plan adds.
- Only the nudge fields and the planned-wish verdict change. Other middle dots (eyebrows, Plan, Transactions, the past-wishes list `done_row`) stay.
- CI gates: `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings` and `cargo test --locked -p finance-ui --bins`.

## Review Focus

1. A group with no usual month (`typical_visits == None`): only "Usual visits a week" shows, no empty "a month" field.
2. A cap with no visit price (`ticket == 0`): money fields show, no visits fields with "0" or blank.
3. A wish that is plainly affordable: `verdict_facts` is empty and the row renders no empty `<dl>` (FactsRow already returns nothing for an empty list).
4. A wish cut in two groups: one fact per group, not one joined value.
5. A not-judged wish: exactly one fact carrying the reason, and none of the money facts.

---

### Task 1: Nudge facts, one value each

**Files:**
- Modify: `app/ui/src/pages/overview/nudges.rs` (the `facts` built in `nudges()`, lines ~142–271; `visits_text`)

**Interfaces:**
- Produces: `fn visit_facts(h: &Habit) -> Vec<(String, String)>` and `fn allowance_facts(label: &str, visits_label: &str, money: String, visits: Option<Decimal>) -> Vec<(String, String)>`, both private to the module, used by `nudges()`.

- [ ] **Step 1: Write the failing tests** in the module's `tests`:

```rust
#[test]
fn visits_and_usual_visits_are_separate_facts() {
    let h = Habit {
        visits: 6,
        visits_this_week: 0,
        typical_visits: Some(d("5")),
        typical_week_visits: d("1.5"),
        ..Habit::default()
    };
    assert_eq!(
        visit_facts(&h),
        vec![
            ("Visits this month".to_owned(), "6".to_owned()),
            ("Visits this week".to_owned(), "0".to_owned()),
            ("Usual visits a month".to_owned(), "5".to_owned()),
            ("Usual visits a week".to_owned(), "1.5".to_owned()),
        ]
    );
    // No usual month yet: the weekly norm alone.
    let new = Habit { typical_visits: None, ..h };
    assert_eq!(visit_facts(&new).len(), 3);
}

#[test]
fn an_allowance_is_money_and_then_visits_when_a_visit_has_a_price() {
    assert_eq!(
        allowance_facts("A week, to stay inside", "Visits a week, to stay inside", "€40".into(), Some(d("2"))),
        vec![
            ("A week, to stay inside".to_owned(), "€40".to_owned()),
            ("Visits a week, to stay inside".to_owned(), "2".to_owned()),
        ]
    );
    assert_eq!(allowance_facts("x", "y", "€40".into(), None).len(), 1);
}
```

- [ ] **Step 2: Run** `cargo test -p finance-ui --bins nudges` — expected: FAIL, functions not found.

- [ ] **Step 3: Implement both helpers and use them in `nudges()`.** Replace the joined facts with:

| Was | Now (label → value) |
|---|---|
| `Visits` | `Visits this month`, `Visits this week` |
| `Usually` | `Usual visits a month` (only when known), `Usual visits a week` |
| `A week, to stay inside` | same label → money; `Visits a week, to stay inside` → visits |
| `{next_month}, a week` | same label → money; `{next_month}, visits a week` → visits |
| `{next_month}` (under one visit a week) | same label → money; `{next_month} visits` → whole visits |
| `Mostly {store}` | `{store} spent` → money; `{store} visits` → count |

Visit counts are bare numbers (`Decimal::normalize().to_string()`); delete `visits_text` and the `allowance_text` closure.

- [ ] **Step 4: Run** `cargo test -p finance-ui --bins nudges` — expected: PASS. Then `grep -n '·' app/ui/src/pages/overview/nudges.rs` — expected: no match outside comments.

- [ ] **Step 5: Commit** `feat(ui): one value per field in "What could be better"`.

### Task 2: Wish verdicts as facts

**Files:**
- Modify: `app/ui/src/pages/wishes.rs` — `verdict_line`, `earlier_saves_note`, `one_off_line`, `skipped_note`, `one_off_verdict`, `repeating_line`, `cuts_phrase` (lines ~100–420), the `line` use in `wish_body` (~1675), and the tests that call `verdict_line` (~2440–3090).

**Interfaces:**
- Produces: `pub fn verdict_facts(row: &WishRow, months: &[WishMonth]) -> Vec<(String, String)>`. `verdict_line` is removed.

- [ ] **Step 1: Rewrite the existing `verdict_line` tests as `verdict_facts` tests.** Keep each test's fixture; change the assertion to the facts below. Collapse tests that only differed in wording. The mapping (values formatted with the module's `amount` and `month_name`):

| Case (existing test fixture) | Facts, in this order |
|---|---|
| not judged | `("Not judged yet", why)` |
| affordable, nothing notable | empty |
| free money drops | `("Free money after", "€350")` |
| short one-off | `("Short by", "€50")` |
| cuts | one `("{Cut name} down to", "€40")` per cut |
| from savings | `("From savings", "€120")`, `("Savings left", "€30")` |
| on the cards | `("On the cards", "€100")`, `("February starts lighter by", "€100")` |
| cards and next month short | `("On the cards", n)`, `("February starts short by", n)` |
| cards with next-month cuts | the card facts, then one `("{Cut name} down to, February", to)` per next cut |
| set-aside skipped | `("Saving starts", "December")`, `("A month from then", "€300")`; skipped with no later month: `("Not set aside in", "November")` |
| earlier wishes saved first | one `("Saved first for Bike", "€300")` per earlier wish, last |
| repeating, short from the start | `("Short from", "March")`, `("Short by", "€50")` |
| repeating, fits then short | `("Fits until", "February")`, `("Short from", "March")`, `("Short by", "€50")` |
| repeating, frees money | `("Frees a month", "€20")`, `("From", "March")` |
| repeating, on the cards | `("On the cards a month", "€30")`, then cut facts |

Example of the rewritten shape:

```rust
assert_eq!(
    verdict_facts(&row("w"), &months),
    vec![
        ("From savings".to_owned(), "€120".to_owned()),
        ("Savings left".to_owned(), "€30".to_owned()),
    ]
);
```

- [ ] **Step 2: Run** `cargo test -p finance-ui --bins wishes` — expected: FAIL, `verdict_facts` not found.

- [ ] **Step 3: Implement `verdict_facts`** by turning each sentence builder into a builder of facts with the same branching (the branch order in `one_off_verdict` and `repeating_line` is the precedence; keep it). The "saved first for" facts are appended unless the list is otherwise empty for a plainly affordable wish, as `verdict_line` did. Remove `cuts_phrase`.

- [ ] **Step 4: Render.** In `wish_body`, replace the `<p>` that showed `line` with `<FactsRow rows=facts />`; a not-judged row keeps its muted tone. The chip beside the name is unchanged.

- [ ] **Step 5: Run** `cargo test -p finance-ui --bins` and `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings` — expected: both clean.

- [ ] **Step 6: Commit** `feat(ui): wish verdicts as fields instead of a sentence`.

### Task 3: See it in the app

- [ ] **Step 1:** Start the devserver UI (see `README.md`, "devserver"; it reads the real ledger and this task writes nothing) and open Overview and Wishes.
- [ ] **Step 2:** Check Shopping and Travel on Overview show eight separate facts and no " · "; check Moving, Japan and Giedre's wedding on Wishes show fields and no sentence, at desktop and phone width.
- [ ] **Step 3:** Open the pull request `feat(ui): one value per field, wish verdicts as fields`; merge when CI is green (the owner lets Claude squash-merge its own green PRs).
