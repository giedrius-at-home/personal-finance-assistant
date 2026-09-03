# ADR 0008: Local owner dashboard and deterministic report contracts

- Status: Accepted
- Date: 2026-09-02

## Context

Phase 5 needs one place to review category proposals, weekly spending, savings
progress, alerts, net worth, taxonomy signals, and policy settings. A rich
client would introduce another build chain and more browser-side financial
state before the interaction model has stabilised.

## Decision

Use server-rendered HTML and a matching typed JSON projection backed by the same
Python application service. Bind deployment to loopback and admit only expected
Host values. Serve no third-party assets and no client JavaScript. All responses
are non-cacheable; all mutations are bounded, same-origin, CSRF-protected POSTs.

Cash-flow and net-worth calculations remain pure deterministic contracts.
Reports use local ISO weeks, represent refunds and transfers explicitly, never
combine currencies, and expose a stable evidence hash. Net worth counts each
canonical account once and withholds its combined total when any valuation is
stale or currencies differ.

Taxonomy analysis produces proposals only. Qwen may later explain aggregate
signals but cannot mutate the approved vocabulary. Policy settings require an
exact preview, confirmation, durable before/after audit record, actor/time, and
rollback that fails if a newer state exists. Upstream category writes remain
disabled.

## Consequences

The local workflow is usable and inspectable now with a small attack surface,
and the JSON contract leaves room for a future desktop client. Page updates are
full refreshes and settings confirmations expire on process restart. The weekly
safe-to-spend figure uses the most conservative latest budget evaluation per
currency, preventing category-level cash availability from being counted more
than once.
