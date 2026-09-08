# ADR 0014: Do not replace the importer with bank-mcp

- Status: Accepted
- Date: 2026-09-08

## Context

Trade Republic transaction coverage through the current open-banking path can
be incomplete, so `bank-mcp` was evaluated as a possible replacement for the
Firefly III Data Importer. The package is a read-only MCP server over Enable
Banking rather than a Firefly importer. Its Enable Banking adapter follows
`continuation_key` values without detecting a repeated cursor.

A separate, read-only production consent was exercised through the local
coverage diagnostic. The provider returned one populated page and then returned
the same populated page with the same continuation key. It also did so for a
single-day request. The diagnostic stopped safely instead of looping or counting
the duplicate page. No transaction content, identifiers, balances, or credential
values were emitted during the comparison.

The evaluated `bank-mcp` revision also did not pass its own full test and lint
commands and its production dependency audit reported high-severity findings.
Those results make it unsuitable for live financial credentials in this stack.

## Decision

Do not install `bank-mcp`, give it live credentials, or replace Firefly III and
the Data Importer with it. Keep Firefly III authoritative and retain reviewed CSV
imports as the Trade Republic history fallback.

Keep the in-repository direct Enable Banking client as an isolated, opt-in,
read-only coverage diagnostic. It must fail closed on a repeated continuation
key, remain unable to write to Firefly, and use a separately consented session.

Any future direct daily collector requires a separate ADR and implementation. At
minimum it must prove cursor progress, deduplicate by a stable provider reference,
survive missed polling intervals, reconcile against Firefly, and expose no raw
financial content in logs or model context.

## Consequences

The current stack retains the richer Firefly history already imported. Direct
Enable Banking access does not solve the provider pagination limitation, and it
cannot backfill older Trade Republic records reliably in the observed behavior.

Daily automation continues at the Firefly-to-dashboard layer. Trade Republic
history is refreshed through reviewed CSV exports until the upstream feed or a
separately validated collector can provide complete, deduplicated coverage.

## Rejected alternatives

- Adopt `bank-mcp` unchanged: repeated cursors can loop and duplicate results,
  while its current validation and dependency posture is not acceptable for live
  credentials.
- Treat the first direct page as complete: silently loses older transactions.
- Persist and expose importer session storage to the diagnostic: mixes unrelated
  credentials and conflicts with ADR 0012.
