# Closure: move money to another account from the split

Date: 2026-11-01
Status: draft for owner approval

## Intent

The owner wants ABN AMRO to hold only income and fixed spending. Everything
else moves on: groceries and going-out money to the Revolut pockets, the
rest to savings. With pots gone, the split has no line that says "move
€300 to Revolut Groceries", so that redistribution cannot be planned or
ticked off in the closure. This adds it.

## Goals

- A split line can send part of an account's leftover to another own
  account: destination `account:<account_id>`, shown as
  `Move to <account name>`.
- The line expects a bank transfer from the source to the target, routed
  through via-accounts and matched or marked by hand in the Transfers step,
  exactly as goal lines are today.
- On the receiving account the money counts as **next month's buffer**
  (carry-in), so a pocket that then spends its whole budget shows a leftover
  of zero, not a deficit. *(Owner's decision.)*

## Non-goals

- No change to wish, goal, plan, buffer or stay lines.
- No automatic suggestion of how much to move; the owner types the lines.
- No new account roles or settings.
- No transfer for a line whose target is the same account (refused).

## Anti-goals

- A move that silently creates no transfer, or one that counts the money
  twice (as the source's buffer and the target's).
- A move to an ignored or unknown account, or across currencies.
- Breaking old periods: every stored line keeps its meaning.

## Design

### Destination

`AllocationLine.destination` accepts `account:<account_id>` next to the
existing kinds. `AllocationLine::target_account_id()` returns the id after
`account:`.

### Validation (in `validate_allocations`)

Given the own accounts (id → currency, role):

- unknown id, or role `ignored`/unclassified → `"move to an unknown account"`;
- target == `from_account_id` → `"cannot move money to the same account"`;
- amount ≤ 0 (split and delta mode alike) → `"a move must be positive"`;
- target currency ≠ line currency → `"move currency does not match"`.

Moves still count toward the per-account total that must equal the
leftover (split mode) or the leftover change (delta mode).

### Transfer

`with_transfer_targets` treats an `account:` line like a goal line whose
savings account is the target: `to_account_id = target`,
`transfer_status = Pending`, then `routed(...)` through the via routes. The
`checked_lines` regex accepts `account:firefly:<digits>`.

### Carry-in

`carry_in_for(month)` adds each `account:` line of the previous month to the
**target** account's carry-in (today only `buffer` lines count, on their own
account). The source account's leftover is reduced by the line as for any
other destination. The review's carry-in row is labelled
`Carry-in (buffer, moved in, planned purchases)`.

Only a target with the **spending** role is credited. A move to a savings
account is money saved, like a goal line: it leaves the source's leftover,
expects its transfer, and adds nothing to the savings account's next
leftover (otherwise the owner would have to split the same money again).
*(Claude's call.)*

### Decisions the builder check raised *(Claude's calls)*

- Validation reads accounts from `review.accounts` (id, currency, role):
  a target that is not there, or whose role is `Ignored`/`Unclassified`,
  is "unknown". No new parameter.
- A move gets the same next-month guards as a buffer line: settling a month
  whose next month is already settled is refused with the existing
  `"next month already uses this buffer"`, and a closed next month is
  flagged ChangedAfterClose, in the same places buffer lines are handled.
- In delta mode a move is positive only; shrinking an existing move means
  editing the stored line on a re-split. A delta move gets its own Pending
  transfer like a goal delta.
- Carry-in from a move is a review figure only; the balance check and the
  Transfers matching are unchanged.

### Suggestion

`account:` lines count as "already assigned in this split", like buffer
lines.

### Screen

- Split editor destination list, in order: Stays on this account, Next
  month's buffer, `Move to <name>` for every other active spending or
  savings account in the same currency (sorted by name), each wish, each
  goal, Plan a purchase….
- Lines list, Transfers, Review and Closed views label the line
  `Move to <name>`; the Transfers step shows its hops like a goal line's.
- Resettle (ChangedAfterClose) may add a move as a delta line.

### Data

No migration; payload shape unchanged.

## Constraints

- Error texts verbatim as above.
- `account:` lines are `LineKind::Split` (or `Delta`), start `Pending` with
  `to_account_id` set by the server; the client sends `to_account_id: None`.
- Tests: validation rules (closure/tests.rs); `with_transfer_targets`
  routes a move, including via a via-account (transfers tests); `carry_in_for`
  credits the target, not the source; settle on the fixture db with a move
  stores a Pending transfer; the Transfers step marks it (existing `mark`
  path); UI `destination_options` lists moves in the stated order and
  excludes the line's own account and ignored accounts.
- `cargo test --workspace`, both clippy targets, fmt clean.

## Left to the builder

- Whether the target list for the editor comes from `ClosureView.accounts`
  (has role) joined with the review's currencies, or a new small view field.
- Exact placement of the moved-in amount in the review row (one row with
  the buffer, as the label says).
