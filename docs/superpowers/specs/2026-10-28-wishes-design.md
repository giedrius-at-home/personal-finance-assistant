# Wishes: can I afford it?

Date: 2026-10-28
Status: draft for owner approval

## Intent

The owner plans purchases ("wishes") and wants the app to say whether each
one is affordable, and if so, what it costs them elsewhere: lower savings,
less going out, tighter groceries, or a credit-card charge that lands on the
following month. Bigger wishes (trips, for example) are saved for over months and
are bought in several pieces, sometimes before the money is fully saved.

The point is not a yes/no gate but an honest trade-off statement the owner
can act on, e.g. "Affordable, but going out drops to €40 and €310 goes on
the card, so December starts €310 short."

## Goals

- Record wishes: name, amount, date, payment source (credit card or one of
  the current bank accounts the app knows).
- Judge each wish against everything already coming up in that month:
  salary, bills, planned payments, budget groups, savings.
- Explain the outcome through a fixed **cutting ladder** (below), naming
  every amount that had to give.
- Model the credit card: €2,500 limit, whole balance charged on the 1st/2nd
  of the following month, current usage entered by hand.
- Wishes with sub-wishes: a parent with a cap and sub-wishes, each with its
  own amount and pay-by date, with a monthly saving plan that recalculates
  when sub-wishes are paid (including early, before fully saved).

## Non-goals

- No bank connection for the credit card. Card usage is a number the owner
  types in.
- No linking of wish savings to a real pot or savings account. "Saved" is a
  number the owner records in the app. *(Claude's call: the owner said "I
  will record that it is saved".)*
- No purchasing, transfers or automatic money movement of any kind.
- No AI suggestions in this first version.

## Anti-goals (would be a failure even if it works)

- Saying "affordable" while silently pushing groceries under €300 or the
  card over its limit.
- Hiding the next-month consequence of putting something on the card.
- A saving plan that does not change after the owner records an early
  payment.

## Domain rules (decided by the owner)

### Budget group importance and floor

Each monthly budget group gains two settings:

- **importance**: 1 = protect most. Higher numbers are cut first.
- **floor**: the lowest amount the group may be cut to.

Owner's current values: going out = importance 2, floor €0; groceries =
importance 1, floor €300. Groups with no importance are never cut
*(Claude's call)*.

### The cutting ladder

When a wish (or a month's saving set-aside) does not fit in a month, the app
covers the gap in this order and stops as soon as it fits:

1. **This month's savings** (the month's expected surplus) shrink. The
   answer says "savings drop to €X". *(Claude's guess, accepted by the owner
   without objection; savings come before any cuts.)*
2. **Budget groups by importance**, highest number first, each down to its
   floor: going out to €0, then groceries to €300.
3. **Credit card**, up to the free limit (€2,500 minus current usage minus
   anything already put on the card by earlier wishes this month). Whatever
   goes on the card is added to the next month's outgoing on the 1st/2nd,
   and the next month is run through the same ladder.
4. **Not affordable**: "short by €N".

**Wishes stack** *(Claude's call)*. A month's wishes are judged together,
not one by one: saving set-asides first (they are commitments), then single
wishes by date, then creation order. Each wish sees the savings, group
amounts and card room the earlier ones left. A group is never cut below
what has already been spent in it this month.

**Step 1 detail** *(Claude's call)*: "savings" is the free surplus left
after existing savings-goal contributions. Goal contributions are not
touched, and step 1 never goes below zero (no dipping into balances carried
from earlier months).

**Month** *(Claude's call)*: the app's existing budget period (payday-aware,
as `period.rs` resolves it). The card charge lands in whichever period
contains the 1st/2nd of the calendar month after the spending.

**Payment source** *(Claude's call)*: only "card or not card" changes the
verdict in this version. The bank account is recorded as a label, with no
per-account balance check.

If the wish's own payment source is the credit card, its amount goes
straight onto the card (step 3 applies to the full amount, checked against
the free limit), and the cost appears in the following month.

### Credit card

- Limit €2,500 (editable setting).
- The owner enters "used this month so far" (October: €9.98). That amount
  is charged on the 1st/2nd of next month.
- The app flags the entered value as stale once the month it was entered
  for has ended.
- *(Claude's call)* The typed value is the card's total usage. Marking a
  wish bought on the card raises it automatically, and the owner can
  overwrite it. The app's predicted card charge for next month replaces any
  recurring card debit the outlook or bills already forecast, so it is
  never counted twice.

### Single wishes

Name, amount, date, payment source. Status: planned → bought (records
actual amount and source) or dropped. Once bought, it stops being judged
and its cost is counted as real spending/card usage.

### Wishes with sub-wishes

Any wish can have sub-wishes; a trip is just the most common case (owner:
"not like trips but wishes with sub-wishes in general").

- The parent wish has a name, a **cap** (e.g. €3,000 for a trip) and a
  date.
- Sub-wishes have a name, an amount and a pay-by date (trip example: flights
  €900 by December, hotel €1,200 by February, spending money = the rest).
- Sub-wishes may not add up to more than the cap. The app warns if they do.
  Whatever the sub-wishes leave of the cap is an implicit "rest" sub-wish,
  due on the parent's date *(Claude's call)*.
- One level of nesting only *(Claude's call)*.
- **Saving plan**: a wish whose date is beyond the current month (single or
  sub-wish) gets a monthly set-aside, spreading (amount − saved for it)
  evenly over the months left until its date. A parent shows the monthly
  total of its sub-wishes. Each month's set-aside is judged through the
  ladder like any wish.
- The owner records "saved €X" against a wish or sub-wish, by hand.
  *(Claude's call)* Saved money is earmarked: it is taken off the free
  surplus so other wishes cannot spend it, and paying uses it without
  counting it twice.
- **Paying early**: the owner records a (sub-)wish as bought (amount, date,
  source) before it is fully saved. The unsaved gap is judged as a normal
  wish in that month, so if it goes on the card the next month starts
  lighter. Money already saved for it counts as spent on it. The remaining
  sub-wishes' plans recalculate.
- A sub-wish costing less than planned returns the leftover to the parent's
  rest *(Claude's call)*. Cap and sub-wishes are editable any time; every
  edit recalculates.

## How it fits the app (Claude's calls, open to the builder)

- New top-level tab **Wishes** next to Plan.
- Calculations live in `crates/core` (new module, alongside `plans.rs` and
  `goals.rs`), types in `crates/types`, storage in `crates/storage` schema,
  Tauri commands in `app/src-tauri`, UI page in `app/ui/src/pages`.
- Importance and floor are stored on `BudgetGroup` (both optional) and
  edited where budget groups are edited today.
- "This month's savings" comes from the existing outlook / monthly budget
  figures for the month in question. The builder picks the exact source and
  states it in the plan.
- **Cuts only change the real budget once a wish is bought** *(Claude's
  call; the owner left this open)*. While planned, the ladder's cuts are only
  shown on the Wishes page. Marking a wish bought writes the cuts it needed
  into that month's `BudgetPlan`, so the budget page, alerts and outlook
  follow.
- A bought wish may become a normal planned payment so existing matching
  picks up the real transaction. Left to the builder.

## Left to the builder

- Screen layout and wording, within the existing design system.
- Exact storage shape and migrations.
- How months beyond the outlook horizon are estimated for far-off wishes
  (e.g. reuse the current month's budget as the template).
- Tests: the ladder and sub-wish recalculation need thorough unit tests in
  core.

## Addendum (2026-10-28): recurring wishes and bill changes

Owner's examples: a new subscription, a psychologist every two weeks, rent
going up when moving out, a cheaper plan going down.

A wish now has a **kind**:

1. **One-off**: everything above (sub-wishes, saving up, bought).
2. **Recurring**: amount per occurrence, every **week, two weeks, month or
   year** (owner: "both weekly, monthly, yearly", plus "biweekly"), first
   date, optional end date, paid by card or an account. No saving up, no
   sub-wishes.
3. **Bill change**: an existing bill, its new amount per occurrence, the date
   the new amount starts, optional end date. The bill's own frequency is
   used. Cancelling a bill is a change to €0.

Judging:

- Each month in the look-ahead gets one demand: (occurrences that month) ×
  amount. For a bill change it is occurrences × (new − old). The look-ahead
  is at least 12 months when any recurring wish or bill change is planned
  *(Claude's call)*.
- A positive amount runs through the same ladder as any wish. On the card,
  each month's charge comes back the next month.
- A negative amount (a cheaper bill) adds to that month's free money and is
  reported as "Frees €N a month from <month>".
- Sentences *(Claude's call)*: "Affordable every month.", "Affordable if
  going out drops to €Y each month.", "€N a month on the card, paid the
  following month.", "Fits until <month>. From <month>, short by €N.",
  "Frees €N a month from <month>."

Starting it:

- **Start** replaces Bought for these kinds. A recurring wish becomes a new
  bill: the owner picks the bill's category when starting, and two-weekly is
  stored as weekly with `skip = 1`. A bill change edits the bill's amount,
  or archives it when the new amount is €0. The wish is then Started and no
  longer judged, because the bill is in the rest of the app from then on.
- Month-0 cuts are written into the budget on Start, as for Bought.
  *(Claude's call: later months' budgets don't exist yet.)*
