# Hold to the budget: wishes before wiggle room, nudges as fields

Date: 2026-11-20
Status: built. Approved by the owner in chat ("Yes, build it"); the card
rule for goals was added by the owner while it was being built.

## Intent

Owner: "wiggle room should be 0 if we cover the wishes and plans. and these
should not be plain strings but value fields … the idea is not to grow my
budget for overspending but constrain me (fewer per week) to stick to the
budget and I would need to know also in the future how well I could spend
later." And: "immediate goals are not short per se if they could be covered
by the credit card … then it is the problem of the next month when the card
is charged."

## Goals

1. **Wiggle room.** This month's spare money is spoken for in order: next
   month's plan shortfall, then the wishes still to save for, and only then
   wiggle room (`allocation::spare_split`). Overview's bar gets a "To
   wishes" part; next month's "To wishes" is less what this month sends.
2. **"What could be better" as fields.** One field per group that is over
   its cap, heading over it, or visited more than usual: the overrun or
   what is left, a status chip, and labelled facts (spent, cap, visits and
   the usual, what a visit costs, visits left this month, a week's
   allowance, next month's allowance, the store that drives it).
3. **Constrain, not grow.** The cap is the group's limit; without one, the
   owner's usual month. Over the cap means zero visits left; next month
   starts from its own limit, never raised by an overrun.
4. **Goals and the card.** A goal the cards cover is not "Short": what the
   card charge lacks the month after is `GoalStatus.card_short`, shown as
   "Card bill short".

## Non-goals

- The Wishes screen's own verdicts and the savings standing on Plan and
  Wishes are unchanged.
- Nothing new is stored.

## Approaches

- **A. Raise the cap to what is being spent** and show the new pace:
  rejected by the owner (it rewards overspending).
- **B. Hold the cap, translate what is left into visits and a weekly
  amount** (chosen): the numbers already exist per group (visits, usual
  visits, spending), so it is arithmetic in the UI over the report.
- **C. A new core "allowance" section in the report**: more plumbing for
  the same numbers; worth it only if the email should carry them too.

## Calls made while building

- A visit costs this month's average; the usual month's when there was no
  visit yet.
- Less than one visit a week next month is shown as the whole month.
- A week is the days left ÷ 7, at least one (the last days are one week).
- Every card-covered goal reads "Card bill short", not only this month's.
