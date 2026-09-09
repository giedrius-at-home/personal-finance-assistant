# ADR 0011: Poll Firefly from the single gateway writer

- Status: Accepted
- Date: 2026-09-05

## Context

The owner dashboard reads assistant-derived SQLite state. Firefly III already
contains the authoritative cash ledger, but the production gateway had no
runner that invoked the tested ingestion service. Firefly's native webhook
cannot safely reach the gateway in the current isolated Compose topology, and a
separate scheduled container would violate the SQLite single-writer decision in
ADR 0010.

The dashboard needs an immediate initial load and bounded ongoing refreshes.
The integration must remain read-only, local, explicit, idempotent, and safe to
disable or revoke without losing Firefly data.

## Decision

Run optional Firefly polling as a background task inside the existing gateway
process. Keep it disabled until the owner provides a dedicated personal access
token through the ignored `.env` file and explicitly enables synchronization.
Run once at gateway startup and then every 86,400 seconds by default, with a
five-minute minimum interval.

Fetch only typed Firefly v1 API resources needed by the existing ingestion
service. Request asset accounts explicitly so expense and revenue counterparties
cannot enter net-worth state as assets. Preserve Firefly's explicit
`include_net_worth` preference so excluded assets remain available for cash-flow
lineage without entering the combined total. Map and validate the full snapshot
before one idempotent SQLite update. Contain poll failures so the dashboard
continues serving the last valid projection. Logs may include aggregate counts
and exception class names only, never transaction content, account identifiers,
balances, or credentials.

## Consequences

The dashboard becomes useful immediately after a configured gateway restart and
refreshes daily without a second database writer. Firefly remains authoritative;
the derived database can be regenerated. API pagination makes the first full
snapshot slower as history grows, so practical latency and database growth must
be monitored.

The personal access token inherits the owner's Firefly permissions even though
the client exposes GET operations only. It must remain owner-only, ignored, and
revocable. Incremental cursors, webhook relay topology, and Ghostfolio polling
remain separate future decisions.

## Rejected alternatives

- Querying Firefly's MariaDB directly: couples the assistant to private upstream
  storage and bypasses the supported API contract.
- A second cron or worker container: creates another SQLite writer and conflicts
  with ADR 0010.
- Weakening Firefly's SSRF protection for a webhook: expands the trust boundary
  to avoid a problem that bounded polling already solves.
