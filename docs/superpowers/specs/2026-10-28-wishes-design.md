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

## Addendum (2026-10-29): must wishes, honest free money, flexible saving, several cards

Owner's problem: with four wishes planned, every one shows "Not affordable"
and November starts about €40 short before any wish. Two reasons: the app
subtracts savings-goal contributions before wishes get a look, and it
insists on an equal set-aside every month. The owner's rule: cover what is
most immediate first, plan against salary minus fixed expenses minus
groceries minus going out (the budget as set, not a forecast), and treat
whatever is left as savings that can be taken back when needed.

### Must wishes

- A wish gains a **must** flag (one-off, recurring or bill change; a
  parent's flag covers its sub-wishes, and a sub-wish cannot be more
  "must" than its parent *(Claude's call)*).
- **Order within a month**: must demands first, then optional ones. Inside
  each tier the existing order holds: set-asides, then purchases by date,
  then creation order. A must wish therefore sees the free money, group
  room and card room before any optional wish does.
- A must wish is never the one that gives way. When the month cannot fit
  everything, optional wishes are what end up short or postponed.
- The page marks must wishes visibly (a "Must" tag beside the name) and
  lets the owner toggle the flag on an existing wish without re-entering
  it.

### Free money for wishes

- Each month's free money = salary + expected income − fixed bills −
  planned payments − budget group limits. **Savings-goal contributions are
  no longer subtracted** for the purpose of judging wishes. Savings is what
  remains; the owner takes from it when something comes up.
- The outlook, budget page and goals keep their own arithmetic. Only the
  wishes view changes.
- The Wishes page states this in one line near the top, e.g. "Free money
  each month: salary minus bills, planned payments and budget groups.
  Savings goals are not held back." *(Claude's call: the owner should see
  why the number differs from the outlook.)*

### Flexible saving

- A set-aside is no longer a fixed equal share. The plan is still
  "(amount − saved) spread over the months left", but a month that cannot
  fit a set-aside without going on the card **skips it** (optional wishes
  first, must wishes last), and the gap spreads over the months that
  remain. "Cannot fit" is judged after ladder steps 1 and 2 (owner's
  decision): a set-aside cuts groups down to their floors like a purchase
  does, and skips only when the cards would be next.
- A set-aside goes on the card only when it is the wish's last month
  before its date (i.e. the purchase itself) or the wish is must and
  skipping would leave too few months to catch up *(Claude's call)*.
- Verdict wording for a skipped month: "Nothing set aside in November;
  €X/month from December." A wish is "Not affordable" only when its amount
  cannot be reached by its date even using card room in the final month.

### Credit cards, plural

- Settings hold a **list of cards**, each with a name, limit, "used this
  month so far" (entered by hand, flagged stale after the month ends, as
  today) and an optional linked bill. The existing single card becomes the
  first entry on upgrade.
- **Card room is pooled**: the ladder's step 3 may use the sum of every
  card's free room. When a wish does not fit in a month, the shortfall goes
  on the cards; the charge comes back the following month and that month is
  run through the ladder again, exactly as today.
- *(Claude's call)* Charges fill cards in the order they are listed, each
  up to its limit, so the owner controls which card is used first by
  ordering the list. The page shows the next month's payback as a total
  and per card.
- A wish paid from the card (source = card) is charged against pooled room
  the same way. When marked bought, the owner picks which card(s) took it;
  the recorded usage on each card rises accordingly.
- Wording: "€X on the cards, so December starts €X short." and, at the top
  of each month, "Pay back €Y on 1 December (Visa €a, Mastercard €b)."

### Settled after the builder check (Claude's calls unless marked)

- **Ladder step 1 is the free money pool.** "Savings drop to €X" means
  free money as defined above shrinks. Goal contributions are neither
  subtracted nor cut; they are simply outside the wishes arithmetic.
- **Plans are stateless.** Every view recomputes each wish's set-aside from
  (amount − saved) and the months left. Months are judged in order; a skip
  in month i spreads the gap evenly over months i+1 … due. No forward
  simulation beyond what the ladder already does month by month.
- **"Too few months to catch up"** for a must set-aside: the month skips
  only if (gap ÷ months left after it) is no more than the smallest
  baseline free money among those later months. Otherwise the set-aside is
  paid now, on the cards if it must be. A must set-aside never shows "Not
  affordable" while pooled card room covers it.
- **Linked bills**: each card's predicted charge replaces only its own
  linked bill in the outlook; a card with no linked bill replaces nothing.
- **Card room and payback**: room = limit − used; next month's payback per
  card = used + what the ladder put on it, i.e. the whole usage.
- **Bought on several cards**: the form lists the cards with an amount
  field each, prefilled by list order, and refuses a save whose amounts do
  not sum to the bought amount.
- **Must on families**: the implicit "rest" sub-wish inherits the parent's
  flag. Un-flagging a must parent un-flags its sub-wishes (cascade).

### Not changing

- The cutting ladder's steps and group floors.
- How bought wishes write their cuts into the budget.
- Sub-wish mechanics, recurring wishes and bill changes.

### Left to the builder

- Storage shape for the card list and the migration of the single card.
- How "too few months to catch up" is decided for a must set-aside.
- Tests: must-before-optional ordering, free money ignoring goals,
  skip-and-catch-up, pooled card room and per-card fill order.

## Addendum (2026-10-29, later): the savings pot

Owner: "I have savings, so let's track the savings better and I am free to
use them … it takes from never spendable first."

- **Savings pot** = the balances of savings-role accounts (what the
  allocation reports as `savings_balance`, "never spendable") plus goal
  amounts protected in ordinary accounts (`protected_savings`). One number
  per currency, shown on the Wishes page as "Savings: €P".
- **Ladder order** becomes: 1 free money → **2 savings pot** → 3 budget
  groups by importance down to their floors → 4 cards → 5 short. Savings
  come before lifestyle cuts (owner's decision).
- **Tracking** is stateless like the rest: month 0 starts with the pot as
  reported; each month's draws lower the pot the later months see; nothing
  refills it. The view shows per month "Savings before / after".
- A must wish draws before an optional one, as with every other step.
- Wording: "Affordable — €X from savings, leaving €Y." When both savings and
  a later step are used, the savings phrase comes first in the sentence
  (e.g. "Affordable — €X from savings, then going out drops to €Z.").
- *(Claude's call)* Buying a wish does not move money between accounts;
  the owner transfers it themself. The pot is a judging aid.
- *(Claude's call)* The skip test for a set-aside is unchanged: "cannot fit
  without the cards" now includes savings and group cuts before the cards.

## Addendum (2026-10-29, evening): set aside what fits, cards only for the purchase

Owner: "not that we would use that card to set aside but we would set aside
maximum possible and then probably the last purchase is used with the credit
card, not that every month I put something and then pay the money back."

- A set-aside takes **as much as fits** of everything still unsaved, from
  free money, the savings pot and group cuts, never the cards. There is no
  equal monthly share any more: whatever was not saved is asked for again
  next month. A month where nothing fits is a skip, as before.
- Owner: "the earlier savings should be covered first and then we save more
  for later savings, not that we try to save for all equally … mostly
  everything is dumped to the earlier ones." So within a month, after the
  must tier, purchases due now come first, then set-asides **soonest due
  first**: the nearest wish takes all it can, the next one gets what is
  left.
- This holds for must set-asides too: they come first in the month, but
  never go on the cards. The "can later months carry it" test is gone.
- The cards are for the **purchase** in its due month (and for a card-source
  wish), as before. A wish is "not affordable" only if the purchase does not
  fit even with the cards.
- A month that already starts short (money committed before any wish) still
  covers its hole on the cards, as before (owner: "that is fine").
