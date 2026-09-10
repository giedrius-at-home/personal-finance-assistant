# ADR 0018: Observed account health and payday overview

Status: Accepted

## Context

Calendar-week net cash flow is hard to interpret with monthly income. A current
dashboard render also gives no evidence that a bank export was recently loaded
or that the statement history is complete. Existing data provides Firefly
accounts, dated transactions, saved categories and source modification dates.

## Decision

Add an optional owner-configured monthly payday using the existing exact
settings preview, revision history and rollback flow. Compute the current period
from the most recent calendar payday to now; clamp days 29–31 to the final day
of short months. Payday starts a new cycle. Do not infer salary timing from
transaction text or predict bank-holiday adjustments.

Aggregate Firefly transactions with Decimal per currency. Count each explicit
Firefly transfer once, excluding it from income/spending. Refunds reduce
spending. Unlike the legacy weekly name heuristic, the new overview does not
classify a transaction as a transfer merely because its description names the
owner. Include only transactions observed up to the report instant.

Show per-account earliest/latest recorded transaction dates, counts, and saved
expense category coverage. Transfers contribute to both account histories but
only once to the cycle transfer count. Pending model proposals never increase
the confirmed category-coverage count.

SQLite schema 7 adds one content-free refresh-status row: last attempt, last
successful full Firefly ingestion, and failure flag. Record it after ingestion
returns, including unchanged snapshots. A failed attempt preserves the last
success. This evidence is distinct from bank import time and reconciliation.

Bank import time, complete statement coverage and bank balance reconciliation
are explicitly unknown/unverified in this version. The existing ingestion
reconciliation compares a copied projection to its source; it cannot verify the
source against the bank statement. Suppress any new spending allowance until
upcoming commitments, protected cash and coverage have been established.

## Consequences

The dashboard can give useful pay-cycle context with explainable limits.
Matched subscription payments, balance observations and import receipts are
follow-ups needed for an available-to-spend forecast. The LLM remains
suggestion-only. No new financial data is sent to external services.

Schema 7 is additive. Older code can ignore its status table; downgrading a
populated database remains disallowed. Existing settings JSON defaults payday
to unset. The separate weekly report retains its current behavior in this PR.

## Subscription evidence

Use the existing Firefly subscriptions via the read-only bills endpoint with an
explicit date window. Reuse Firefly's calculated expected dates and linked
payment dates rather than duplicating its recurrence or matching engine.
Store a bounded typed snapshot in the additive schema-8 table; omit notes and
transaction identifiers. Replace only after a validated fetch, so deleted bills
disappear while failed fetches retain last-known evidence. The health flag makes
failures visible.

Display configured cost ranges and upper-bound totals for distinct expected
dates through the day before payday (or the next 30 days when unset), respecting
active state and end dates. This is schedule evidence, not verified outstanding
debt. Payment dates are shown as historical links, not used to infer that a
future bill was paid. Missing links remain explicit. Stale/out-of-window totals
and totals after a failed refresh are withheld in the UI. Do not present an
available-to-spend number until the other planning inputs are verified.
