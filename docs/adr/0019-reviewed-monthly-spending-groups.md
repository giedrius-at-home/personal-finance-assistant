# ADR 0019: Reviewed monthly spending groups

Status: Accepted

## Context

Firefly categories often describe stores. The owner wants one broader spending
level without losing that detail or creating a second transaction ledger.
Salary received late in one month can fund the next month, so a calendar-month
spending report must not equate cash flow with available money.

## Decision

- Keep Firefly authoritative for transactions and their confirmed categories.
  Pending classifier proposals never determine this report.
- Store exact category-to-group mappings locally in SQLite. Schema 9 appends
  immutable owner-reviewed revisions, including unassignment. Atomic expected-
  revision writes prevent stale forms from overwriting another decision.
- Map to a bounded broad vocabulary. Unmapped and uncategorised expenses remain
  visible in “Needs a group”; a missing category is distinct from a category
  literally named “Uncategorised”. No account or personal-name heuristics.
- Calculate calendar-month spending with Decimal, separately per full currency
  identity. Refunds reduce net spending. Gross purchases determine group shares;
  zero-purchase windows have zero shares. Explicit transfers and income are
  excluded, as are future instants and non-Firefly sources.
- Compare equal calendar-day counts against the previous month, clamping to its
  shorter length. Label today's partial day and unverified bank import coverage.
  With no prior expense/refund evidence for a currency, withhold the comparison
  instead of presenting an assumed zero. This is not a spending allowance.
- Optional local Ollama suggestions use only the selected category and up to
  48 reviewed group examples. Strict output permits a known group or abstention.
  The model cannot save mappings, change Firefly, or browse the web. A separate
  owner save is always required. Existing suggestion opt-in and model settings
  control availability; requests are serialised and bounded to one attempt.
- Use same-origin CSRF-protected POSTs. Keep expandable sections, search, and
  unsaved group choices across progressive dashboard updates. Ordinary mapping
  saves work without JavaScript; model suggestions require JavaScript.

## Consequences and limits

Mappings regroup all available history, not individual purchases. Category
renames need a reviewed new mapping; removed mappings remain in audit history.
Mixed-purpose stores may need transaction-level categories in Firefly; no
per-transaction group override or custom group editor is introduced here.
Stored group history belongs in the existing local derived-state backup and
must never enter Git. Empty schema rollback is supported; populated rollback
is refused. No automatic bulk assignment, salary allocation, reconciliation,
bank synchronisation, or source-ledger mutation is part of this feature.

All tests use synthetic data. Monetary, temporal, source-filter, model-output,
CSRF, restart, unassignment and stale-write behaviour have dedicated coverage.
