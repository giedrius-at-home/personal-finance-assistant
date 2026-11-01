# Closure balance check: confirm every account before the split

Date: 2026-11-01
Status: draft for owner approval

## Intent

The month closure splits a leftover computed from transactions. If the
ledger has drifted from the bank (a missed import, a fee the bank never
listed, a Swedbank account with no balance feed), the split is made on wrong
money and the pots, buffer and next month inherit the error.

Before the split, the owner looks at every account's balance, confirms it
or types the real one, and the ledger is corrected. The correction is an
**unexplained adjustment**: it fixes the balance but is never counted as
income, spending or leftover. The split cannot be saved until every account
is confirmed.

## Goals

- On the closure screen, before Settle, show every spending and savings
  account with its ledger balance and, when a bank sync reported one, the
  bank's balance.
- Let the owner confirm an account as-is or type the real balance.
- Book the difference as a reconciliation entry on that account, dated the
  day of confirmation, and keep the ledger as the sum of postings.
- Block Settle until every account is confirmed for this month.
- Show on the review what each account was adjusted by.

## Non-goals

- No change to what the leftover is made of. Reconciliation entries stay
  out of income, spending, leftover, carry-in, groups, forecast history,
  daily review and the Transactions tab, exactly as the existing Banks-screen
  reconciliation already is.
- No automatic reconciliation. A bank figure only prefills the field; the
  owner's confirm books it.
- No balance "as of month end". The ledger balance shown is today's total,
  the same figure the review already calls `closing_balance`.
- Ignored accounts and inactive accounts are not checked.
- One check per account, in the account's own currency (the one most of
  its postings use). Revolut pockets that are connected are own accounts of
  their own and get their own row. Postings in another currency on the same
  account are left out, as the review already leaves them out
  (`skipped_foreign`); a bank figure in another currency is dropped and the
  row prefills with the ledger figure. *(Owner's decision: every connected
  pocket is its own account; the unsynced savings account is typed by hand.)*
- The credit card is not an account here; its balance is typed on the
  wishes side already.
- No change to Resettle, Finalize, Reopen, Mark or Unmark.

## Anti-goals

- Settling a month while one account is still unconfirmed.
- A confirmation that silently stays valid after the ledger changed under
  it (a later import or a reconciliation made on the Banks screen).
- Stacking two adjustments for one month when the owner retypes a figure.
- Counting an adjustment as money earned or spent anywhere.

## What exists already (reuse, do not rebuild)

- `crates/core/src/bank_sync/mod.rs` `reconcile_bank_account`: books a
  `reconciliation`-kind transaction between the account and a
  `<account> reconciliation (<currency>)` counterparty account, via
  `insert_transaction`. Its date is the day of the write.
- `crates/core/src/load.rs` loads only `withdrawal`, `deposit` and
  `transfer` kinds into the transaction model, so reconciliation entries are
  invisible to every report; the account balance is the sum of all postings
  of own accounts, so they still count there.
- Bank syncs store each account's `bank` and `ledger` balance per bank run
  (`AccountBalance`, read by `bank_runs()`); the Banks screen shows them.
- Closure actions go through `ClosureAction` → `writes::closure::act`, each
  naming the period revision it read; a save bumps `revision`.

## Design

### Stored state

`MonthPeriod` gains one field, `balance_checks`, a map from account id to:

```
BalanceCheck {
  balance: Decimal,          // the balance the owner confirmed
  ledger_balance: Decimal,   // the ledger total at the moment of confirming,
                             // after the adjustment (equals `balance`)
  confirmed_at: String,      // RFC 3339, as other instants in the payload
  transaction_id: Option<String>, // "firefly:<id>" of the reconciliation
                             // entry this check booked; None when nothing
                             // was off
}
```

Serde default and skipped when empty, so old payloads read unchanged and
serialize byte-identical. *(Claude's call.)* The payload keeps
`deny_unknown_fields`; the field is added to the Rust type and to
`stored_period_round_trips`-style tests.

### New closure action

```
ClosureAction::ConfirmBalance { month, revision, account_id, balance }
```

Refused ("invalid") when: the month is closed or changed after close, or a
later month is already closed (mirror `require_no_later_closure`); the
account is unknown, ignored, inactive or unclassified; `balance` is not a
number with at most the account currency's minor units. A stale revision is
refused with the usual "month closure changed; refresh".

Effect, in one write transaction:

1. If the period already holds a check for this account that booked a
   reconciliation entry, delete that entry (its transaction, postings and
   search row). This is what makes a retype replace, not stack.
2. Compute `difference = balance − ledger balance of the account in its
   currency` (the same `ledger_balance` helper bank sync uses).
3. If the difference is non-zero, book a reconciliation entry exactly as
   `reconcile_bank_account` does (same counterparty account, same kind),
   dated today, description
   `Balance confirmed at <balance> <currency> for <YYYY-MM> closure`.
   Pull the booking into a shared helper both callers use.
4. Store the check with `transaction_id` set when an entry was booked.
5. Bump the revision. When an entry was booked, record it under the
   `reconciliation` audit source exactly as `reconcile_bank_account` does
   (closure steps record nothing today; this keeps the "last import"
   indicators that read `import_runs` honest). *(Claude's call.)*

Deleting the earlier entry first and recomputing from the live ledger keeps
the result right even if imports arrived between the two confirms.

There is no un-confirm. Confirming again with the ledger figure removes any
earlier adjustment and leaves a check with no transaction.

### When a check counts as confirmed

A check is **current** when the account's ledger balance now equals the
check's `ledger_balance`. Anything that moves the balance later (an import,
a Banks-screen reconciliation, a CSV import) makes it stale; the screen
shows it as "changed since confirmed" and the owner confirms again.

`ClosureView` gains, per spending/savings account, a `BalanceStatus`:

```
BalanceStatus {
  account_id, name, currency,
  ledger: Decimal,
  bank: Option<Decimal>,           // last bank run's figure, if any
  bank_reported_on: Option<String>,
  check: Option<BalanceCheck>,
  state: Unconfirmed | Confirmed | Stale,
  adjusted_by: Option<Decimal>,    // the booked difference, when any
}
```

The server computes `state`; the UI never derives it.

### The gate

`Settle` is refused with
`"confirm every account balance first: <names>"` when any spending or
savings account that is active and not ignored has no current check for the
month. Resettle is not gated (the month is closed; balances were confirmed
at settle time).

Sync is not gated. Confirming is allowed in `open`, `settling` and
`settled` (a settled month can be resettled by a new Settle, which then
needs current checks again).

### Screen

On the closure page, between the Sync step and the Split step, a
**Balances** panel listing every checked account:

| Account | Ledger | Bank | Confirm as | |
|---|---|---|---|---|
| ABN current | 1,234.56 | 1,234.56 (31 Oct) | [1,234.56] | ✓ Confirmed |
| Revolut | 310.00 | 270.00 (1 Nov) | [270.00] | Off by −40.00 · Confirm |
| Swedbank | 2,000.00 | — | [2,000.00] | Confirm |

- The input prefills with the bank figure when there is one, else the
  ledger figure. When ledger and bank agree the row reads as a match.
- Confirm sends `ConfirmBalance` with the input's value. A row whose check
  is stale shows "changed since confirmed" and its Confirm button again.
- One "Confirm all matching" button confirms every row whose input equals
  its ledger figure (one action per row, in sequence; stop on the first
  failure and show it).
- The Split step's Settle button is disabled while any row is unconfirmed
  or stale, with the reason next to it. The server refuses anyway.
- Each account in the review shows "Adjusted by −40.00 on 1 Nov" when its
  check booked an entry.
- Dark and light theme, same design system as the rest of the closure page.

### Effects elsewhere

- Overview and net worth read the postings sum, so they show the corrected
  balance with no change.
- The Banks screen's Reconcile keeps working. A reconciliation made there
  makes the closure check stale (balance moved), so the owner confirms once
  more; the closure confirm then books nothing because the ledger already
  matches.
- The daily review and AI daily report never saw reconciliation entries and
  still do not.

## Constraints

- Payload shape stays readable by the Python-era reader where one still
  exists: new field only, serde default, snake_case.
- All money as `Decimal`; rounding to the account currency's minor unit.
- Every new server rule has a test in `crates/core/src/writes/closure/tests.rs`
  or next to it: gate refused and allowed, retype replaces (one entry, not
  two), stale after an import, no entry when nothing is off, closed month
  refused.

## Left to the builder

- Exact Leptos component layout inside the existing closure page modules
  (`app/ui/src/pages/closure/`), reusing the Banks page's `BalanceCheck`
  table styling where it fits.
- How `ClosureView` obtains the last bank figures (through `bank_runs()` or
  a small store read); one source, read once per view.
- The transaction-deletion helper (none exists for a single transaction
  today; `writes/transactions.rs` has the search-row delete).
- Whether "Confirm all matching" is one request or a loop of single
  confirms; the server side exposes only the single action.
