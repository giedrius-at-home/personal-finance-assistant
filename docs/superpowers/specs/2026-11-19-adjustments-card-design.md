# Adjustments: where spending runs above the owner's own norm

Date: 2026-11-19
Status: approved by the owner in chat (2026-11-19) and built. Approach chosen by the owner
in chat ("A + drivers"), then widened: "it should track how often I go
somewhere like to groceries, eat out, ah to go and at what times, it should
nudge me also". The owner agreed to three steps, one PR each: **(1) this
document: the card and visit habits; (2) nudges; (3) time of day.**

## Intent

The owner asked for "better insights into my spending behavior and what kind
of adjustments I should make". The app already shows what was spent, how the
month is pacing against the budget (Overview, PR #105) and six months of
trends. Nothing says, per group, *"this is above what you normally spend,
here is what drives it, and here is the budget that would be realistic"*.

The ledger shows why that matters. The November plan budgets Groceries at
€300 and Food & drinks out at €100, while going out has run at several times
that for months: the budget is either unrealistic or ignored, and the app
cannot tell the owner which. Shopping has no budget and has climbed five pay
periods in a row. The saved "group limits" (`spending_group_limits`) have no
screen and no rows; the budget plan is the tool the owner really uses.

## Goals

1. An **Adjustments** card on Spending → Plan & patterns, one row per
   spending group, showing the budget (if any), the owner's typical month,
   and where this month is heading.
2. Each row gets at most one finding, in plain words: above your norm,
   budget below your habit, or budget above your habit. Rows with no finding
   fold into a "near your norm" list.
3. A flagged row opens to its **drivers**: the up to three stores
   (categories) behind the excess, as visits × average ticket against the
   usual ("Albert Heijn To Go: 21 visits × €3.50, usually 8 × €3.40").
4. One action per finding that writes the month's budget plan: set the
   budget to the typical amount, or add a budget for an unbudgeted group.
5. A **Habits** section in the same card: how often the owner goes to each
   place this week and this month against the usual, the average ticket and
   the weekdays it mostly happens on.
6. Money the owner has not sorted (the "Other" group and uncategorised
   spending) appears as a single "Unsorted" row linking to the review
   inbox, never as something to cut.

## Non-goals

- No second answer to "am I on track this month?". Overview owns that since
  PR #105; the card links there.
- No nudges or notifications (step 2) and no time of day (step 3). Clock
  times exist only in ABN AMRO and Swedbank memos; the Revolut pockets,
  where everyday purchases are paid, store dates only. Weekdays are known
  for everything and are used here.
- No savings target ("free €300 a month") and no use of the Wishes cutting
  ladder. Approach B, later, once budgets are realistic.
- No fixed spending: bills, subscriptions, rent. Subscription price creep is
  a different feature (it needs history-based recurring detection).
- No use of `spending_group_limits` and no new table or setting.
- No edits to PR #105's files (`visuals.rs`, `charts.rs`, `overview/budget.rs`,
  `category_trends.rs`) beyond calling what they export. The one change
  Overview sees is "Budgets that last", below.
- No AI-written text. Every sentence is built from numbers the card shows.

## Anti-goals

- A month-end figure that disagrees with Overview for the same budget row.
- Telling the owner to cut money that is not spending: transfers to own
  pockets and savings, card settlements, unsorted money.
- A finding from too little history (a group seen in one earlier month) or
  too little of the month (two purchases on day 2).

## Constraints

- PR #105 must be on `main` first; build on a branch rebased onto it. The
  card reuses its `month_end(row, pace, days)` and `row_color`
  (`app/ui/src/pages/overview/visuals.rs`).
- Months are the budget's months (the app's existing definition), not pay
  periods.
- EUR and any other currency are handled per currency, as trends are.
- Core keeps the Python-parity decimal helpers (`py_div`, `py_round`,
  `py_mul`) for new arithmetic.

## Approaches

- **A. Norm-vs-budget card.** Budget, typical, heading and one action per
  group. Small; reuses trends and the budget plan write. Says where, not why.
- **B. Savings-target card.** The owner names an amount to free and the card
  spreads cuts by group importance and floor. No group has an importance
  today, so it needs a setup step, and cutting from unrealistic budgets
  gives unrealistic cuts.
- **C. A plus drivers.** A's rows, and flagged rows open to the stores
  behind the excess.

**Chosen: C** (owner: "A + drivers"). It answers both where and why, and
leaves B possible once the budgets mean something.

## Design

### One source of numbers: budget rows, real and shadow

Everything in the card is computed in core by the budget's own machinery
(`crates/core/src/budget.rs`), so months (including an early closure),
fixed-spending exclusions and ownership of a purchase are the same as on
Overview. New module `crates/core/src/adjustments.rs`; new report type
`Adjustments` on `Dashboard` (`#[serde(default)] adjustments: Vec<Adjustments>`,
one per currency):

```rust
pub struct Adjustments {
    pub currency: Currency,
    pub month: String,                // the budget month, YYYY-MM-01
    pub rows: Vec<AdjustmentRow>,
    pub unsorted: Decimal,            // this month, see below
}
pub struct AdjustmentRow {
    pub row: BudgetRow,               // planned, spent, history, daily_spending, elapsed_days, categories
    pub budgeted: bool,               // false: a shadow row, `planned` is zero
    pub group: Option<String>,        // shadow rows: the reviewed group
    pub habit: Habit,                 // the row as a whole
    pub stores: Vec<StoreHabit>,      // per category in the row
}
pub struct StoreHabit { pub category: String, pub habit: Habit }
pub struct Habit {
    pub visits: usize,                // outflows this budget month
    pub spent: Decimal,               // net this budget month
    pub typical_visits: Option<Decimal>, // median over earlier months with spending, needs 2
    pub typical_spent: Option<Decimal>,
    pub history_month_count: usize,
    pub visits_this_week: usize,      // Monday through today
    pub typical_week_visits: Decimal, // median of the 12 full weeks before, zero weeks count
    pub recent_visits: usize,         // the last 4 full weeks plus this one
    pub weekday_visits: [usize; 7],   // Monday first, the 12 full weeks
}
```

- **Budgeted rows** are the current month's `MonthlyBudget.groups`, as is.
- **Shadow rows**: the month's plan is extended in memory (never saved)
  with one zero-amount `BudgetGroup` per reviewed group that has variable
  spending in the six months and that no real budget group names in
  `source_groups`, and `monthly_budget` is run on that extended plan. Real
  groups keep precedence (`budget_owner`), so a purchase a real group
  already claims by category or id is not counted again. The owner's groups
  `Transfers`, `Investments` and `Other` get no shadow row.
- **Habits** for a row and its stores are counted over exactly the
  purchases that row owns (same `budget_owner` call), so sums and medians
  are taken over the row's own weekly and monthly series, never added up
  from stores. A visit is one outflow; refunds are not visits. Weeks start
  on Monday in the owner's time zone.
- **Unsorted** is this budget month's net variable spending that has no
  category, no reviewed group, or the group `Other`.

The UI computes, for every row alike:

- typical = median of the entries of `row.history` before the last one
  (the last is this month) that are above zero; needs three.
- heading = PR #105's `month_end(&row, Pace::Daily, days)`.
- **too early**: `row.elapsed_days < 7` or `habit.visits < 2`. Then heading
  shows "too early to tell" and no above-norm finding is made. Budget
  findings (below) do not depend on this month and still show.

### Findings

Two independent findings; a row can carry both.

- **Above your norm** (any row, not too early): heading ≥ 1.25 × typical
  and heading − typical ≥ 25. Text: "Heading to €H, about €D above a
  typical month." The card header sums D over these rows: "€X above your
  norm this month" (hidden when zero).
- **Budget fit** (budgeted rows):
  - budget ≤ 0.8 × typical → "The budget is €B; a typical month is €T."
    Action **Set budget to €T**.
  - budget ≥ 1.2 × typical and budget − typical ≥ 25 → "You usually spend
    €T; €(B−T) of this budget is spare." Action **Lower budget to €T**.
- A shadow row that is above its norm gets the action **Add a €T budget**.

€T in actions is typical rounded to the nearest 10. Budgeted rows also link
to Overview ("See this month"). Rows with no finding fold into "Near your
norm (n)". Thresholds are the author's call.

### Drivers

A row with an above-norm finding opens to its top three stores by
`spent − typical_spent × (elapsed_days / days in the budget month)`, a store
without history counting its whole spend, positive values only:
"{store}: {visits} visits × {avg}, usually {typical_visits} × {typical_avg}"
or "{store}: new this month, {visits} visits, €{spent}". Averages are
spent / visits, computed in the UI.

### Habits

Below the adjustment rows, the same rows as a table, each opening to its
stores.

- Shown: rows and stores with `recent_visits >= 4`; the rest fold into
  "Less often (n)".
- Columns: this week, usual per week (one decimal, "<1" below one), this
  month, usual per month, average ticket this month, mostly (the weekdays
  holding at least a quarter of `weekday_visits`, at most two; "any day"
  when none does).
- "This week" is highlighted when it is at least two visits above the
  usual week. That is the only judgement here; money findings stay above.

```
Habits                 this week  usual/wk  this month  usual/mo  avg ticket  mostly
▾ Food & drinks out        6         4         19         14       €9.40     Fri, Sat
    Albert Heijn To Go     5         2         14          8       €3.50     Mon, Thu
    Coffee                 1         2          5          6       €2.80     any day
▸ Groceries                4         4         13         14      €11.00     Sat
  Less often (6)
```

### Budgets that last (the author's call, flagged for the owner)

Today a month without a saved plan falls back to the built-in default
(Groceries €300, Food & drinks out €100) in `budget::monthly_budgets`, so a
budget set in November would be gone on 1 December and the same finding
would return. Change: a month without a saved plan uses the latest earlier
saved plan of that currency (its groups, with the new month), and only
then the default. The daily forecast, the savings outlook and wish cuts
use the same rule (`budget::plan_for`), so no screen plans with a different
budget. Effect outside the card: Overview's budget for future months
follows the last plan the owner saved instead of the default.

### Actions

All actions call the existing `save_budget_plan` command with the current
budget month's real plan (shadow groups stripped) edited in memory:

- *Set / lower budget*: the group's `amount` becomes the rounded typical.
- *Add a budget*: a new `BudgetGroup` with `id` = slug of the group name,
  `name` = the group name, `amount` = rounded typical,
  `source_groups` = [group], the rest empty.

The button shows the change first ("Food & drinks out: €100 → €430") and
saves on a second click; after saving the report refreshes, as other writes
do. Errors from the command are shown inline.

### UI (`app/ui/src/pages/adjustments.rs`, new)

Rendered in `PlanPatterns` (`pages/spending.rs`) directly under the budget
panel, above the weekday card. Pure functions (rows, findings, driver
ranking, plan edit) live at the top of the module with unit tests
(`cargo test -p finance-ui --bins`); the component only renders them.

```
Adjustments                               €400 above your norm this month
● Food & drinks out   budget 100 · typical 430 · heading 580
    Heading to €580, about €150 above a typical month.     See this month →
    The budget is €100; a typical month is €430.       [Set budget to €430]
    ▸ Soju Bar: 4 visits × €40, usually 1 × €44
    ▸ Coffee: 22 visits × €2.80, usually 12 × €2.90
● Shopping            no budget · typical 150 · heading 400
    Heading to €400, about €250 above a typical month.   [Add a €150 budget]
    ▸ Amazon: 5 visits × €52, usually 1 × €60
  Unsorted            €76 not sorted yet                        [Review]
  Near your norm (3)  Groceries · Transport · Health & personal care
```

The Unsorted row shows when `unsorted` is above zero: "€X not sorted yet",
button "Review" → Transactions tab; it has no typical and no finding.
Row dots use `row_color` for budgeted rows and the neutral tone otherwise.
The card is hidden when there is no row with a typical value.

### Tests

- Core: shadow rows (uncovered groups only, real groups keep precedence,
  no double count, excluded groups), habit medians and visits, refunds
  netting, fixed spending left out, week boundaries (Monday, time zone),
  zero weeks in the weekly median, the unsorted sum, an early-closed month,
  and the plan carry-forward (saved plan beats default, later months).
- UI: each finding and its boundary, both findings on one row, the "too
  early" guard on budgeted and shadow rows, the unsorted row,
  driver ranking including a new store, the plan edit for both actions,
  the habits table's show/fold rule, "mostly" weekdays and the highlight.
- Checked in the dev server against a **copy** of the ledger (`DEV_DB`).

## Left to the builder

- Exact wording, spacing and badges, following the design system in
  `app/ui/src/ds`.
- Whether the confirm step is an inline swap or a small popover.
- Slug collisions for a new budget group id (append a number).
