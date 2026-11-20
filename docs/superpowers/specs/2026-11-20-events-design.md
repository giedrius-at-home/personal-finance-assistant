# Events: planned spending with dates, and fields without dots or sentences

Date: 2026-11-20
Status: approved by the owner in chat ("good but also event can have
wishes and subwishes, so the object can be the same"), with that change
folded in below.

## Intent

Owner: "can you also plan the events (like on the trips, friends over, etc
I will name it in the app. now the most approximate is parents in NL from
30 nov till dec 5, so will be extra spending there, so it may not fall to
the regular budget parts … I would suggest how much I want to spend for
this and it will tell me if it is reasonable or not … a sort of a wish but
we give adjustments to spending because we can … it would tell me how I
should behave … also I am not the one who pays for everything (but if the
transaction did not come it means someone else paid)."

And on the screens that exist: "i dont like these middle dots, maybe also
another field" (the "What could be better" fields) and "wishes also have
lots of long strings".

Why: a visit or a trip is days of spending that are not a normal week.
Today those days land in Groceries, Eating out and the rest, the groups
read "over usual month", and nothing says whether the amount the owner has
in mind is affordable or how to pace it while it runs.

## Decided with the owner

1. An event owns **all day-to-day spending inside its dates**. Nothing is
   tagged by hand for that.
2. Events get **their own section** (approach C below), not a wish kind.
3. A wish is the saving-up phase; an event is the spending phase. **A
   planned wish moves to Events** and is planned financially there.
5. **An event is the same object as a wish** and can have wishes and
   sub-wishes inside it (Japan: Flights, Hotel; parents: Zoo, Naturalis),
   planned financially like any wish.
4. An event also counts **hand-picked payments outside its dates** (a
   flight booked in January; zoo or museum tickets bought ahead for the
   parents' visit).

## Goals

### Part 1: one value per field (ships first, on its own)

`app/ui/src/pages/overview/nudges.rs` joins two values with " · ". Each
becomes its own field:

| Today | Becomes |
|---|---|
| Visits: `6 this month · 0 this week` | `Visits this month` 6, `Visits this week` 0 |
| Usually: `5 a month · 1.5 a week` | `Usual visits a month` 5, `Usual visits a week` 1.5 |
| A week, to stay inside: `€40 · 2 visits` | `A week, to stay inside` €40, `Visits a week, to stay inside` 2 |
| December, a week: `€18 · 1 visit` | `December, a week` €18, `December, visits a week` 1 |
| December: `€74.74 · 1 visit` | `December` €74.74, `December visits` 1 |
| Mostly Clothing: `€250.10 · 5 visits` | `Clothing spent` €250.10, `Clothing visits` 5 |

Only these fields. Middle dots elsewhere in the app (eyebrows, Plan,
Transactions) are not touched.

### Part 2: wish verdicts as fields (ships with part 1)

`verdict_line` in `app/ui/src/pages/wishes.rs` builds one sentence
("Nothing set aside in November; €1,300.06/month from February. Affordable
— free money drops to €0. €599.62 saved for Moving first."). The chip next
to the name already carries the verdict, so the sentence goes and its
numbers become labelled fields in the same grid as "Put aside this month"
and "Saved up by". Only the fields that apply are shown:

| Sentence part | Field |
|---|---|
| short by €50 | `Short by` €50 |
| short by €50 from March (repeating) | `Short from` March, `Short by` €50 |
| Fits until February | `Fits until` February |
| €120 from savings, leaving €30 | `From savings` €120, `Savings left` €30 |
| €100 on the cards — February starts €100 lighter | `On the cards` €100, `February starts lighter by` €100 |
| free money drops to €40 | `Free money after` €40 |
| if going out drops to €40 | `Going out down to` €40 (one field per cut) |
| Nothing set aside in November; €300/month from December | `Saving starts` December, `A month from then` €300 |
| €300 saved for Bike first | `Saved first for Bike` €300 (one per earlier wish) |
| Not judged yet: reason | stays one short line |

The past-wishes list (`Bike · bought €900 on …`) keeps its lines.

### Part 3: events

**What an event is.** A name, a start and end date, the amount the owner
wants to spend, a currency, and optionally: must-have, hand-picked
transactions, excluded transactions, the wish it came from and the money
saved for it.

**What it counts as spent.** A transaction belongs to an event when it
counts for the budget (`budget::counts`), is not excluded by hand, and
either

- it was hand-picked into the event, whatever its date or group, or
- its date is inside the event's dates and it is day-to-day spending: not
  a fixed bill or subscription (`fixed_ids`), not a paid planned payment,
  and not in the Transfers or Investments groups.

A transaction the owner did not make does not exist, so what someone else
paid is never counted and never entered. A refund lowers what is spent.
When two events overlap, a hand-pick wins, then the event that starts
first.

**What it does to the groups.** Event transactions leave their groups:
`Ownership::key` gives them `event:<id>`, in the running month and in the
history months alike. That covers the two places `Ownership` is used, the
budget (`budget.rs`) and the adjustments (`adjustments.rs`), and so group
spending, visits, "usual month" and the nudges. The per-group forecast the
nudges read for "heading over" must leave them out too. The month's budget
gets one **Events** row (spent per event) so its total stays whole.
Trends, category trends, the outlook, alerts and the email keep reading
the raw transactions; that is accepted for now.

**The usual share moves with the days.** A group's limit is not raised or
rewritten, but for a month an event touches (upcoming or running), each
day-to-day group hands the event its *usual those days* share: the group
reserves that much less (never below what it has already spent), and its
row shows it as a field, `To Parents in NL` €x. Without this the groups
would keep reserving money for days they no longer own and wiggle room
would fall by the whole planned amount instead of the extra.

**Is the amount reasonable.** The owner would have spent something on
those days anyway, so only the extra is judged:

- *Usual those days*: for every day-to-day group, its usual month
  (`Habit::typical_spent`) ÷ the days of that month × the event's days in
  that month, summed.
- *Paid ahead*: hand-picked payments dated before the event starts, plus
  amounts entered by hand (below). That money is already gone, so it is
  not judged.
- *Extra to find*: planned − paid ahead − usual those days, not below
  zero, split over the calendar months by the event's days in each
  (parents: one day of November, five of December).
- Each month's share is its own demand for the existing wish ladder
  (`wishes::ladder::judge`), due in its own month, in the same order as
  the wishes (must-haves first, then by date). Money saved for the event
  pays the earliest share first. The event's verdict is the worst of its
  shares: Affordable, With trade-offs (cuts, savings or cards, each a
  field), Not affordable. Months are calendar months throughout, as in the
  ladder.
- *Saved so far* is a field the owner records, as on a wish, and is
  earmarked in the savings pot the same way.
- *Fits without trade-offs up to*: usual those days + the free money the
  ladder has left for it.
- Until the event starts, its extra is saved for monthly like a wish, and
  the money it still needs is spoken for in `allocation::spare_split`
  along with the wishes, before wiggle room.

**How to behave while it runs.** From the first day to the last:

- *Spent* (paid ahead included), *Left* (planned − spent), *Days left*
  (today included), *A day, to stay inside* (left ÷ days left).
- Chip, on the money meant for the days themselves (planned − paid
  ahead): `On track` when what was spent on the days ≤ that × days gone ÷
  days; `Spending fast` above it; `Over` when spent > planned.
- What the event still asks of the money is what is left (planned −
  spent, not below zero): spending lowers the demand as it lowers the
  cash, so nothing is counted twice, and the saved money stays earmarked
  only up to what is left.
- The amount stays editable; the verdict follows.
- Spending is real outflow, so an overrun already lowers this month's
  spare money the way any overspending does. Nothing is raised to fit it.

**When it is over.** The day after the end date the event is past: it
shows planned against spent and stops asking for money. What was not
spent is simply not spent. Its transactions stay with it, so that week
never bends the groups' norms.

**One object.** An event is a stored wish (same table, same `Wish`
type, same save, drop, must and saved-money writes) that also carries
event details: start and end date, the hand-picked and the excluded
transaction ids. A wish with those details shows on Events and not on
Wishes. Its amount is the planned total (a parent's cap).

**Parts.** An event has sub-wishes exactly as a parent wish has: "Zoo
€18", "Flight €780", each with its own date, source and saved money,
judged and saved for by the existing wish code in the month it is due.
Buying a part ("Bought", as today, with the paid amount) counts it as
spent on the event; that is also how a purchase the ledger cannot show is
entered, such as one put on the credit card. A hand-picked transaction can
be tied to a part, which marks the part bought at that amount, so it is
never counted twice. The parts sit inside the planned total; what is left
after them (the parent's `rest`) is the money for the days themselves, and
that, less the usual share, is the *extra to find* judged per month as
above. Read "paid ahead" above as: bought parts plus hand-picked
transactions not tied to a part.

**Wish to event.** Any one-off wish that is still planned, with or
without sub-wishes, gets "Plan as event": the owner gives the dates and
the wish gains the event details; nothing else about it changes, so its
parts, bought parts, saved money and source all stay. "Back to wishes",
offered until the event starts, removes the details again. Recurring
wishes and bill changes cannot become events.

**Where it shows.**

- A new top tab, **Events**, after Wishes: a form (name, dates, amount,
  must), then the running, upcoming and past events, each a block of
  fields in the style of the Wishes rows, with its parts listed under it
  as sub-wishes are (add a part, Bought, Drop).

```
Parents in NL   30 Nov – 5 Dec   [Affordable]
Planned              €400.00     Usual those days        €150.00
Paid ahead            €46.00     Extra to find           €204.00
Saved so far           €0.00     Fits without trade-offs €520.00
Paid ahead: Naturalis €28.00, Zoo €18.00            [Add a payment]
```

- "Add a payment" searches the transactions (the existing search) and
  picks one in; each picked or dated transaction is listed with "Not part
  of this" to take it out.
- On Overview, a running event is one more field in "What could be
  better", first in the list:

```
PARENTS IN NL
€212.40 left      [On track]
Planned               €400.00
Spent                 €187.60
Days left             3
A day, to stay inside €70.80
```

## Non-goals

- Splitting costs with others, or entering what someone else paid.
- Per-day or per-category budgets inside an event.
- Events in the spending email.
- Repeating events.
- An event in two currencies: it has one, and counts that currency's
  transactions.

## Anti-goals

- An event must not make overspending look fine: no group limit and no
  wiggle room is raised because an event ran over.
- No sentences and no " · " pairs in what is added.
- The owner never has to tag the ordinary spending of the event's days.

## Constraints

- Rust workspace: `crates/types` (shared types), `crates/core` (maths and
  writes), `crates/storage` (SQLite migrations), `app/src-tauri` and
  `app/devserver` (commands; every command needs both, plus a Tauri
  permission file), `app/ui` (Leptos).
- Stored in the `wishes` table: the event details are an optional field
  of the wish payload (absent on every wish stored today), so no
  migration is needed.
- Tests cover the calculations and the writes, not wording.
- Every merge to main ships a release, so parts 1–2 and part 3 are two
  pull requests, each complete on its own.

## Approaches

- **A. An event is a new kind of wish.** Reuses the wish store, form and
  judging; smallest build. But an event is tracked live against bank
  transactions, which no wish is, and `wishes.rs` is already 3,100 lines.
- **B. An event is a temporary budget group with dates.** Budget plans are
  per month and an event crosses months; there is no verdict before it
  starts. Dropped.
- **C. Events as their own section, borrowing the wish judging** (chosen
  by the owner, with wishes moving into it once planned). Saving-up and
  spending stay two clear phases, and paid-ahead payments have a natural
  home. The owner then asked for the same object as a wish, so C keeps its
  own tab and its own live tracking but takes A's storage: no `events`
  table, no second set of writes.

## Calls made by the designer (not asked of the owner)

- Events is a top tab rather than a section of Wishes.
- "Usual" is the owner's usual month per group, not the group's limit.
- The planned amount is split over months by calendar days; what was
  really spent lands in the month the budget engine gives each
  transaction (so card purchases follow the card's month).
- An overlap goes to the event that starts first.
- "Not part of this" exists, so a laptop bought during the visit can be
  kept out.
- A past event is derived from its end date, not a stored status.
- Parts sit inside the planned total rather than adding to it, as
  sub-wishes sit inside a parent's cap today.

## Settled with the owner after the review

- Credit-card purchases are not transactions in the ledger (card usage is
  a typed number, the statement one fixed bill). The owner rarely pays by
  card during an event, so a hand entry is enough; events do not touch
  card usage or the card bill.

## Left to the builder

- Type and module layout (`crates/core/src/events/`), command names,
  whether the Events rows reuse the Wishes row components, and
  how the event demands are threaded into `wishes::Inputs` and
  `spare_split`.
- How `Ownership` learns about events without every caller changing.
- The exact field order and tones, following the Wishes rows and
  `nudges.rs`.
