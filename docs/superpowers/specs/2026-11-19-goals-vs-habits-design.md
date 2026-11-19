# Goals against habits: what the wishes and an emergency fund ask of spending

Date: 2026-11-19
Status: approved by the owner in chat ("all good approved the design");
how it is built is the author's call.

## Intent

Owner: "it should tell me that dude, if you want to save for this and this,
you should spend less … keep the wishes in mind and then savings of 6 months
(tweakable) of bare living expenses (add this to the wish too but sort of
savings goal … we have to arrive eventually). also reactive to the planning
as well if I change my rent or add the subscription."

The Wishes screen already judges every wish month by month (free money, the
savings pot, cuts to budget groups, the cards). It judges against the
*budget*, so a budget far below the habit (food out €100 against a typical
€570) makes wishes look affordable, and only budgeted, ranked groups can be
cut. There is no emergency-fund goal.

## Goals

1. A **habit scenario** of the same judging: from next month on, every
   spending group costs its typical month (the Adjustments card's rows,
   budgeted and unbudgeted), and every non-essential group can be cut.
2. A **Goals** card on Spending → Plan & patterns, above Adjustments:
   what the goals need a month, what the habits leave, the gap, each wish's
   status at current habits, and the cuts that close it, in habit terms.
3. An **emergency fund**: months (default 6, 0 turns it off) × bare living
   cost (fixed bills + the typical month of the essential groups). It has no
   due date and takes what is left after the wishes (owner: "Leftovers after
   wishes"); the card shows its target, what it holds and when it is full.
   Editable on the Wishes page.
4. Reactive to planning: bills, bill-change wishes (a higher rent) and
   recurring wishes (a new subscription) change both what is left each month
   and the fund's target.

## Non-goals

- The Wishes screen's own verdicts, cuts and writes are unchanged.
- No nudges (next step), no time of day.
- The running month is not re-judged: Overview and Adjustments cover it.
  The habit scenario starts with next month.
- No new table. Nothing in the habit scenario is saved.

## Anti-goals

- Advice to cut an essential group, or a group below a floor the owner set.
- Counting a group's spending twice (as budget and as habit).
- A fund date that assumes cuts the owner has not made.

## Approaches

- **A. Change the ladder's inputs for everyone** (Wishes judged on habits).
  Honest, but changes verdicts and the cuts Wishes writes into budgets.
- **B. A separate, simpler goals calculation.** Easy, but a second engine
  that ignores cards, set-asides and bill changes and can disagree with
  Wishes.
- **C. Run the existing ladder a second time on a habit scenario** (chosen):
  same engine, different baselines, read-only. Wishes stays as it is; the
  card says what the same engine concludes when spending stays as it is.

## Design

### Settings (`WishSettings`, `wish_settings.payload_json`)

`emergency_months: u32` (default 6; 0 = off; at most 36) and
`essential_groups: Vec<String>` (default `Groceries`, `Transport`,
`Health & personal care`, `Housing & utilities`). Saved with the existing
`save_wish_settings`.

### Habit scenario (`crates/core/src/wishes/goals.rs`, new)

Input per spending row, from `Dashboard.adjustments` (allocation currency):
id, name, typical (median of the earlier months above zero, needs three;
rows without one are left out), budget, typical visits a month, essential.

Baselines are the plan's (`baseline::baselines`), then for every month
after month 0:

- `free −= Σ (typical − budget)` over the rows (budget is zero for an
  unbudgeted row): spending as usual instead of as budgeted.
- `groups =` the rows as rooms: `limit = typical`; importance and floor
  from the latest plan that ranks a group of that name, else importance 1
  with `min = typical` for an essential row (never cut) and importance 2
  with `min = 0` for the rest.

Month 0 is left as the plan has it. The ladder then runs unchanged
(`ladder::judge`) with the same items, repeats and saver.

### Output (`WishesView.goals: Option<GoalOutlook>`)

- `needed_monthly`: Σ of the planned top wishes' `monthly_set_aside` in the
  plan view (what the screen already asks to put away each month).
- `habit_free` / `plan_free`: a later month's free money in each scenario.
- `gap = max(0, needed_monthly − habit_free)`; `coverage` = habit_free /
  needed_monthly as a percentage, 0–100 (`None` without wishes).
- `goals`: per planned top wish: name, due, remaining, must, `short` in the
  habit scenario and `plan_short` in the plan.
- `cuts`: per group, over the habit scenario's months: total cut, months
  with a cut, the largest month's cut, typical, and typical visits and
  ticket so the UI can say "about N fewer visits".
- `fund`: months, fixed and essentials a month, target, `balance` (the
  savings pot not earmarked for wishes), and `eta`:
  - balance at the end of the wishes' horizon = the last month's
    `savings_left` + Σ `free_left` of the habit months;
  - the first month the running total reaches the target, else the horizon's
    end + ⌈(target − balance) / steady⌉ months, where steady is `habit_free`
    less the repeating costs that never end; `None` when steady ≤ 0.
- Fixed a month = the allocation's `next_fixed` less the card bills, plus
  the monthly equivalent of every planned bill change or recurring wish
  that does not end (weekly × 52/12, biweekly × 26/12, yearly ÷ 12).

### UI

- `GoalsCard` (`app/ui/src/pages/goals.rs`) on Plan & patterns above
  Adjustments; loads the wishes view.

```
Goals                                        habits cover 67% of what they need
Goals need €750/mo · your habits leave €500/mo · gap €250
  Moving €3,000 · Mar 2027 · must      on track
  Japan €4,000 · Sep 2027              €1,100 short at current habits
  Emergency fund €8,400 (6 months)     holds €611 · full by mid 2028
To close it, at most:
  Food & drinks out   −€150 a month for 6 months   ≈ 8 fewer visits
  Shopping            −€70 a month for 4 months
```

- Wishes page: an "Emergency fund" block with the months field, the bare
  living cost and its two parts, target and date; saving calls
  `save_wish_settings`.

### Tests

Core: free money and rooms of the habit scenario; essentials and floors
never cut; month 0 untouched; a wish affordable on plan but short on habits;
cuts aggregated per group; fund target reacts to a bill change and a
recurring wish; eta inside the horizon, after it, and none; fund off at 0
months; settings round trip and validation. UI: card sentences, visits
equivalent, fund block parsing.

## Left to the builder

Wording, layout, and how the essential groups are edited (a comma list is
enough for now).
