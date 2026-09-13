# ADR 0022: Commit authoritative projections atomically and refresh incrementally

## Status

Accepted

## Context

The owner dashboard is useful only when local projections cannot silently diverge
from Firefly III. The earlier implementation committed records one by one, retained
goals that disappeared upstream, treated pending category suggestions as reporting
inputs, and consumed CSV previews before a write completed. Daily lifetime-history
polls also made one transient failure last for a full interval and would eventually
reach a fixed pagination ceiling.

## Decision

- Firefly remains authoritative for transaction categories. Pending suggestions do
  not affect reports, and approve/correct actions fail closed when the narrow
  category writer is unavailable.
- Category writes are serialized per proposal. A confirmed Firefly write is retried
  idempotently if the following local transition fails; it is never blindly undone.
- A validated source snapshot replaces accounts, transactions, eligible goals, and
  subscriptions in one SQLite transaction. Missing goals and their active alert
  projections are removed while immutable source history remains.
- Daily refreshes reread a configurable recent window. A complete history refresh
  runs periodically in calendar-year slices, avoiding one lifetime pagination cap.
- Failed refreshes retry with bounded exponential backoff.
- Optional alert evaluation failures are reported without invalidating an already
  committed authoritative Firefly snapshot.
- CSV previews remain claimed but recoverable until successful completion. A retry
  accepts only exact external-ID transitions from new to duplicate.
- Balance freshness uses the local successful observation time. Upstream modification
  timestamps remain provenance, not observation evidence.
- Ghostfolio balance ingestion is opt-in. Cross-system mirrors require ignored local
  configuration that explicitly selects the canonical owner.

## Consequences

The dashboard can continue serving the last complete generation during a failed
refresh. A first or periodic complete refresh makes more API calls because history is
partitioned by year; ordinary refreshes are much smaller. Edits older than the recent
lookback are reflected at the next complete refresh. Native notification delivery is
still a separate macOS host boundary and is shown as unavailable until installed;
dashboard alerts and deterministic Firefly budget evaluation remain active.

No raw transaction, account-link, token, or bank-export content is added to logs,
Git, readiness responses, or validation evidence.
