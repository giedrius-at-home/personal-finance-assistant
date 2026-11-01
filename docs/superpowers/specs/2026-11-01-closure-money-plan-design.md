# Closure: where the money goes, and pocket transfers that pair

Date: 2026-11-01
Status: owner said "go" (review, merge, update the app); built on the
owner's answers in chat.

## Intent

After a close the owner moves money by hand: ABN AMRO keeps what next
month's fixed costs need, groceries and going-out money go to the Revolut
pockets, and everything else goes to Revolut Savings, where money already
marked "saved" toward wishes is spoken for. The closure screen does not say
any of those numbers today, and the Revolut sync books pocket transfers as
purchases, so the ledger lies about where the money is.

Owner's words: "all the money goes to fixed, groceries, planned, fun out,
and wishes"; "in every closure I wanna see how much I can transfer to
Revolut … how much I can keep in ABN AMRO, and how much is in Swedbank is
okay"; "the closure should take into account the current balances"; the
safety buffer "can be disabled now"; the three "unplanned" November rows
"are just transfers to the right accounts".

## Goals

1. A **Money plan** card on the closure screen, computed from the current
   ledger balances: per account what to keep and what is spare, the
   transfer to make (ABN → Revolut, split into pocket top-ups and savings),
   and for the savings account what must stay (wishes already saved) and
   the wiggle room above that.
2. The Wishes page's savings note says the same must-keep and wiggle room.
3. The savings suggestion stops showing a "− Safety buffer" row when the
   setting is zero (the owner sets it to zero in Manage → Settings).
4. Revolut pocket transfers import as transfers, both legs as one
   transaction, and the unconnected savings pocket ("To Instant Access
   Savings") is recognised as a transfer to the savings account routed via
   Revolut.
5. The rows already imported wrong are repaired once, by a migration.

## Non-goals

- The split (leftover) and its lines are unchanged; the plan is read-only
  guidance next to it. *(Prefilling split lines from the plan is a follow-up;
  the split works on the leftover, the plan on balances, and mixing them now
  would be guesswork.)*
- No change to how wishes judge affordability, nor to the cutting ladder.
- No new setting. The safety buffer setting stays; zero means off.
- No bank transfers made by the app.

## Anti-goals

- A plan that counts money twice (a pocket top-up that also lands in savings).
- Pairing two entries that are not the same transfer (different amount,
  different day, or a real counterparty).
- Repairing a row the owner categorised on purpose.

## Design

### Money plan (`crates/core/src/closure/plan.rs`, new)

Server-computed, EUR only, as of now, from the same figures Overview uses:

```
MoneyPlan {
  as_of: String,            // YYYY-MM-DD
  month: String,            // the month being funded now (allocation.current_month)
  currency: Currency,
  accounts: Vec<PlanAccount>,
  moves: Vec<PlanMove>,
  savings: Option<SavingsStanding>,
}
PlanAccount {
  account_id, name, role: AccountRoleKind,
  balance: Decimal,                     // ledger, now
  keep: Decimal,                        // what must stay
  keep_parts: Vec<(String, Decimal)>,   // "Bills still to pay", "Planned payments", "Groceries budget left", …
  needs: Decimal,                       // top-up it should receive (pockets)
  spare: Decimal,                       // max(0, balance − keep)
  short: Decimal,                       // max(0, keep − balance)
}
PlanMove { from_account_id, to_account_id, via_account_id: Option<String>, amount, reason }
SavingsStanding {
  account_id, name,
  balance, incoming,                    // incoming = moves into it
  must_keep,                            // Σ saved of planned one-off wishes in EUR
  wishes: Vec<(String, Decimal)>,       // name, saved — only saved > 0
  wiggle_room,                          // balance + incoming − must_keep (may be negative)
}
```

Accounts considered: the balance-check accounts (active, spending or
savings). Balances: the ledger figure `balance_statuses` already shows.

Per spending account, `keep` =
- bills still to pay this month (`allocation.current_fixed_remaining`) on the
  **salary account** (`settings.salary_account_id`), else nothing; *(Claude's
  call: bills carry no account; the salary account pays them)*
- planned outflows due this month, unpaid, with `account_id` = this account
  (`allocation.current_plans_remaining` is the total; per account from
  `overview.planned_payments` the same way `monthly_allocation` filters
  them);
- for each budget group of the current month's plan homed on this account:
  the group's `remaining` from `allocation.groups` (matched by name).

`needs` (pockets and any spending account that is not the salary account) =
`max(0, keep − balance)`; `spare` on the salary account = `balance − keep`.
Other spending accounts (Swedbank) are never a source: their spare stays
where it is and the row reads "fine". *(Owner: "how much is in Swedbank is
okay".)*

Moves, from the salary account's spare only, in this order:
1. every account with `needs > 0`, by name, `min(needs, spare left)`;
2. the rest to the savings account: the EUR savings-role account with a
   `via_account_id`, else the first savings account by name. None → no move.

`via_account_id` on a move is the target's role `via_account_id`. The UI
groups moves by `via` so they read "Transfer €X to Revolut, then €a →
Groceries, €b → Fun out, €c → Revolut Savings".

`SavingsStanding.must_keep` = Σ `saved` of wishes with `status == Planned`,
`kind == OneOff`, currency EUR (sub-wishes included, parents' own `saved`
included). Wishes marked bought or dropped do not count.

`ClosureView` gains `plan: Option<MoneyPlan>` (`serde(default)`); `None` when
there is no EUR allocation.

### Screen

`MoneyPlanPanel` (new, `app/ui/src/pages/closure/plan.rs`) as a card under
the wizard, above the Accounts panel, shown whenever the view has a plan:

- heading "Where the money goes · as of 1 Nov", one line per account:
  `ABN AMRO · €1,000.08 · keep €1,020.00 (bills €1,018.90, …) · short €19.92`
  / `Revolut Groceries · €300.00 · keep €300.00 · fine` / `Swedbank · €7.00
  · fine`;
- "Transfer €X from ABN AMRO to Revolut" with the per-target breakdown, or
  "Nothing to transfer";
- "Revolut Savings: €1,200.84 (+€X incoming). Keep €624.00 for wishes
  already saved (Dinner with Everyone €300, …). Wiggle room €576.84." A
  negative wiggle room reads "€N short of what the wishes need".
- Light and dark theme, same design system as the closure page.

### Wishes page

The free-money note ends with `Savings: €T, of which €M is saved for
wishes; wiggle room €W.` (the pot the schedule starts from already leaves
the saved money out, so it is the wiggle room and `T = W + M`). `WishesView` gains `savings_must_keep: Decimal`
(`serde(default)`), computed the same way as `must_keep` above (shared
helper in `wishes`).

### Safety buffer

`SuggestionPanel` omits the "− Safety buffer" row when it is zero. The owner
sets the setting to 0 in the app after the update (done in this task).

### Sync: pocket transfers (`crates/core/src/bank_sync`)

1. **Names from the session.** `granted` also collects each session
   account's `name` (uid → name); `preflight` returns them next to the
   expiry; `attempt` imports with `connection.names` when non-empty, else
   the session's names. Nothing is persisted. *(The saved Revolut consent
   has `names: {}`, so `sibling` never matched.)*
2. **Second leg adopts the first.** Before creating a withdrawal or deposit
   with an unknown counterparty, `import` looks for a transaction with the
   same `external_id` on another own account mapped in this consent, the
   opposite direction, the same amount and date, whose counterparty is a
   bare expense/revenue account. If found, that transaction becomes the
   transfer: `kind = 'transfer'`, source/destination set to the two own
   accounts (money out of the entry's own account on the outgoing side),
   the counterparty posting re-pointed, `category_id` cleared, search row
   rebuilt; nothing new is created and `created` is not incremented.
3. **Unconnected savings pocket.** An entry with no IBAN or sibling match
   whose description starts with `to ` (outgoing) or `from ` (incoming),
   case-insensitive, and matches the savings hint
   (`savings?|vault|pocket|spaar`) is a transfer to/from the **one** own
   active asset account whose role is `savings` with
   `via_account_id == firefly:<this account>`. Zero or several candidates →
   no change (withdrawal/deposit as today). *(Claude's call.)*

### Repair migration (`crates/storage/src/pockets.sql`, migration 6)

Pure SQL, idempotent:

- Rule 2 on existing rows: pairs `(W withdrawal, D deposit)` with equal
  `external_id`, `date`, amount, `W.source_account_id` and
  `D.destination_account_id` distinct asset accounts, `W.destination` an
  `expense` account and `D.source` a `revenue` account. `W` becomes the
  transfer (destination = `D.destination_account_id`, posting re-pointed,
  category cleared), `D` and its postings and search row are deleted, `W`'s
  search row is rebuilt from names.
- Rule 3 on existing rows: a withdrawal from an asset account to an
  `expense` account named `(unknown destination)` whose lower-cased
  description starts with `to ` and contains `saving`/`vault`/`pocket`/
  `spaar`, when exactly one `account_roles` row has `role = 'savings'` and
  `via_account_id = 'firefly:' || source` → transfer to that account,
  category cleared, search row rebuilt. Deposits from `(unknown source)`
  starting with `from ` likewise.

The October balance check of Revolut Savings goes stale (ledger moves from
€287.02 to €1,200.84); October is closed, so nothing is gated.

### Builder-check calls *(Claude's calls)*

- A pocket's `keep` is its group's `remaining` (budget minus what was spent
  from it this month), compared straight against the ledger balance. A
  group with no allocation row keeps its full budget.
- `plan.rs` takes the EUR allocation, the dashboard's planned payments and
  the current month's stored budget plan (for group → account); one
  dashboard call still, pieces passed in. Two groups with one name: the
  first wins.
- The salary is not anticipated: before payday the salary account shows
  "short" and no moves.
- Groups homed on the salary account count in its `keep`; `needs` is only
  meaningful on accounts that are not the salary account.
- `incoming` is the rest-to-savings move only; pocket top-ups never count.
- Rule 2 searches the database, so the first leg may come from an earlier
  run or the same batch; order does not matter. Two identical pairs are
  paired 1:1 in id order.
- Rule 3 in the importer applies whenever no IBAN or sibling matched; the
  migration only touches `(unknown destination)` / `(unknown source)` rows.
- A savings account in another currency is left out of the plan.

## Constraints

- Serde: new fields only, `default`, old payloads unchanged.
- Money as `Decimal`, rounded to the currency's minor unit for display only.
- Tests: plan (keep per account, pockets top-up, rest to savings, Swedbank
  never a source, must-keep ignores bought/dropped wishes, negative wiggle);
  wishes note; suggestion hides the zero buffer; import (names from session,
  second leg adopts, savings pocket by via, no match with two candidates);
  migration on a fixture with the three November shapes.
- `cargo test --workspace`, both clippy targets, fmt clean.

## Left to the builder

- Exact card layout and the money formatting helper used elsewhere on the
  closure page.
- Whether `plan` is computed inside `closure_view` from the dashboard
  already built for the suggestion (preferred: one dashboard call).

## Addendum (2026-11-01, later): the split starts from the plan

Owner started the follow-up. A **fresh** split (no stored lines) of an
account the plan moves money out of starts with one `account:<target>`
line per plan move, in the plan's order, each capped by what is left of
the leftover and skipped when the split cannot reach that account, then a
`stay` line for the rest (none when nothing is left). The account's note
reads "Prefilled from the money plan; edit freely." A confirmed split keeps
its lines; a resettle and a deficit account are unchanged. Validation and
the destination list are untouched.
