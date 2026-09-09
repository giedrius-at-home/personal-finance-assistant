# ADR 0015: Add reviewed provider-neutral CSV resync

## Status

Accepted

## Context

Open-banking coverage and freshness differ by provider. In particular, a bank
may return a shorter or repeating transaction window even while its official
export contains the complete ledger. The upstream Data Importer supports manual
CSV mappings, but provider-specific saved mappings are cumbersome for routine
resync and mistakes can attach rows to the wrong account or bill.

Firefly must remain authoritative. Raw exports can contain sensitive transaction
and account metadata and must not become a second local ledger or enter Git.
Silent fuzzy deduplication is also unsafe because two legitimate payments may
share date, amount, and merchant.

## Decision

Add one disabled-by-default CSV upload flow to the localhost owner dashboard.
Parse semantic headers across common delimiters and locale formats instead of
maintaining bank-specific positional templates. Require either an unambiguous
account-name match or explicit owner account selection, followed by an exact
preview and separate confirmation.

Keep raw rows only in a bounded, process-local 30-minute preview. Generate stable
opaque external IDs from normalized row facts and use upstream external IDs for
confirmed duplicates. Treat a same-day/amount/description match without a stable
reference as a possible duplicate and block the commit rather than guessing.
Recheck Firefly after preview and before any write.

On confirmation, create only new transactions through a narrow client that asks
Firefly to apply rules, fire webhooks, and reject duplicate hashes. Create a
transfer only when the counterparty IBAN exactly matches another Firefly asset
account; otherwise preserve the row's signed deposit or withdrawal direction.
Refresh the derived dashboard after a successful import without turning a
refresh failure into a misleading import failure.

## Consequences

One upload workflow can resync Trade Republic, ABN AMRO, Revolut, Swedbank, and
other connected accounts when their exports use supported semantic columns.
Firefly rules remain the reorganization and categorization engine. Re-uploading
the same export is idempotent when stable references are present, while
ambiguous rows require owner resolution instead of being silently dropped.

The gateway gains a narrowly scoped, opt-in transaction write path and a runtime
dependency for bounded multipart parsing. CSV files remain outside Git and are
not retained by the application. Formats without recognizable semantics must be
added with synthetic tests before use.

## Alternatives considered

- Persist provider-specific CSV mapping files: these may contain account metadata
  and reproduce the Data Importer's operational fragility.
- Fuzzy auto-deduplicate every similar row: this can erase legitimate repeated
  payments and is not reversible before import.
- Make a direct bank API the only ingestion path: provider coverage and consent
  freshness remain uneven and do not remove the need for authoritative exports.
- Store uploads for scheduled processing: this creates another sensitive data
  store and weakens the explicit owner-preview boundary.
