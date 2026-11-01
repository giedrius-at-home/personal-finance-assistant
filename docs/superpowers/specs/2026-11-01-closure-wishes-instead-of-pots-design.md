# Closure: wishes instead of pots

Date: 2026-11-01
Status: draft for owner approval

## Intent

Pots were an earmark on money inside an account, filled and emptied by the
month's split. Since wishes arrived, the owner plans purchases there, and
the pot ledger has drifted from reality (October moved €758 out of Revolut
Savings into pockets; the General savings pot still says €328.09). Pots go.
A split line can now save toward a wish instead.

## Goals

- Remove pots from the closure screen and the split: no Pots panel, no
  "New pot…", no `pot:` destination, no "take from pot", no General
  savings pot, no "take €X from General savings" offer.
- Add `wish:<wish_id>` as a split destination: a positive amount saved
  toward a planned wish. Confirming the split raises that wish's saved
  total; re-confirming replaces this month's contribution rather than
  stacking it.
- Keep every closed month readable: September's `pot:` lines, their
  transfers and the stored snapshots still render and still match.

## Non-goals

- Savings goals (`goal:`), "Plan a purchase…" (`plan`), buffer and stay
  stay exactly as they are. *(Owner named only wishes, buffer and stay as
  what replaces pots; goals and plans were not in question.)*
- No bank transfer for a wish line. Money stays on the account; only the
  wish's saved figure rises. *(Owner's decision: bookkeeping only.)*
- No change to how wishes judge affordability; they already read saved
  money against the savings balances, not against pots.
- No deletion of the `pots` / `pot_entries` tables or of old pot lines.

## Anti-goals

- A closed month that stops rendering, or whose pot transfers stop
  matching, because pots are gone.
- A wish saved twice for the same month after a resplit.
- A wish line on a wish that is bought or dropped, or that overshoots what
  the wish still needs.
- The split still offering a pot anywhere, or the suggestion still
  proposing a take.

## Design

### Destinations

`AllocationLine.destination` accepts `goal:<id>`, `plan`, `buffer`,
`stay` and the new `wish:<wish_id>`. `pot:<id>` is **read-only legacy**:
stored lines keep it and render, validation refuses it on a new or
resettled split with `"pots are no longer available"`. A take (negative
amount) is refused the same way; the only negative lines left are delta
lines, which the server makes.

`AllocationLine::wish_id()` returns the id after `wish:`.

### Validation of a wish line

In `validate_allocations` (given the planned wishes):

- unknown id, or a wish that is not `Planned` → `"unknown wish"`;
- amount must be positive → `"a wish line must be positive"`;
- line currency must equal the wish's → `"wish currency does not match"`;
- across the split, the lines for one wish may not exceed
  `wish.amount − (wish.saved − this month's earlier contribution)` →
  `"saves more than the wish still needs"`. For a parent wish with
  sub-wishes the cap is the parent's rest (`amount − sub-wishes' amounts`).

### Which wishes

Only one-off wishes (`WishKind::OneOff`) that are `Planned` can take a
line: top-level wishes without sub-wishes, planned sub-wishes, and a
parent's rest. Repeating and bill-change wishes have no "saved" meaning and
are neither offered nor accepted. *(Claude's call.)* A parent's `saved`
already means money saved toward its rest (the schedule's rest item is
`rest − parent.saved`), so the parent's cap is `rest − parent.saved` with
this month's earlier contribution added back; sub-wishes are capped on
their own `amount − saved`. *(Claude's call.)*

### What confirming does

`record_side_effects` (settle and resettle) stops writing pot entries and
instead, for every planned wish that has a line in the new or the previous
allocations:

```
saved' = max(0, wish.saved − Σ previous lines for the wish + Σ new lines for the wish)
saved_month = the later of the wish's saved_month and the closure's month
```

where "previous" and "new" are **all** lines for that wish in the period's
allocations before and after the write (split and delta lines alike), so
the same formula serves settle, re-settle and resettle. Written through the
same store as `record_saved` so the Wishes page sees it at once. Removing a
wish line gives its money back (saved falls). A manual "record saved" on
the Wishes page is absolute and wins; the next closure write adjusts
relative to it, clamped at zero. *(Claude's call.)* `saved_month` only
moves forward so a set-aside recorded this month is not forgotten by
closing last month. *(Claude's call.)*

In a resettle (delta mode) a wish line must still be positive: a shrunken
leftover cannot take money back from a wish; the owner lowers it on the
Wishes page. *(Claude's call.)*

A previous line whose wish is no longer planned (bought or dropped) is
skipped in the side effect; a new or re-confirmed split may not keep such a
line (`"unknown wish"`), the editor labels it `Wish (bought): <name>` /
`Wish (dropped): <name>` and the owner removes it. *(Claude's call.)*

Reopen keeps the split, so nothing changes. Finalize keeps the split, so
nothing changes.

### The suggestion card

`SavingsSuggestion` loses `general_pot_id` / `general_balance`. The card
keeps "Nov is short by €X" / "Nov has €X to spare" and "Why this number?",
with the same breakdown minus the "General savings balance" line and the
"Take €X" button. "Already assigned in this split" counts positive lines to
wishes, goals, plans and buffer.

### Screen

- Split editor: destination list is Stays on this account, Next month's
  buffer, one entry per planned wish as `Wish: <name> (€saved of €amount)`,
  Goal: … entries, Plan a purchase…. No New pot…, no Take from pot button,
  no take rows. For a parent wish with sub-wishes list the parent as
  `Wish: <name> rest` and each planned sub-wish on its own.
- The Pots panel on the closure page goes, and with it `closure_pots` /
  `save_pot` commands, `PotsView`, `PotDraft`, the Tauri capability
  entries and the Accounts panel's "Pots on X add up to more than its
  balance" notes.
- Transfers step: a legacy `pot:` line still shows its transfer and can be
  marked; label it `Pot (retired): <name>` using the stored pot name.
- Review and ChangedAfterClose views: unchanged.

### Legacy reads

`load_pots` / `pot_balances` stay for rendering old lines and matching old
transfers (`with_transfer_targets`, `candidates`). `general_pot` is no
longer created on read. `pots_view` and the pots store writes are deleted.

### Data

No migration. `pots` and `pot_entries` tables remain, untouched from now on
(the delete-and-rewrite of a month's pot entries on settle also stops; old
rows stay as history).

## Constraints

- Serde shapes of `MonthPeriod` and `AllocationLine` do not change; old
  payloads deserialise byte-for-byte.
- `wish:` lines are `LineKind::Split`, `TransferStatus::NotNeeded`,
  `to_account_id: None`.
- Tests for every rule above in `crates/core/src/closure/tests.rs`
  (validation) and `crates/core/src/writes/closure/tests.rs` (settle raises
  saved, resplit replaces, removing a line lowers, bought wish skipped,
  legacy pot line still renders and matches).
- `cargo test --workspace`, both clippy targets and fmt clean.

## Left to the builder

- Exact layout of the destination labels and where the saved/amount hint
  sits in the `Select`.
- Whether `validate_allocations` takes the wishes as a slice or a map.
- How the wish totals are formatted in the "Why this number?" breakdown.
