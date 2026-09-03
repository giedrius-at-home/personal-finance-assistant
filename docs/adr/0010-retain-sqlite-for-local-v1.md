# ADR 0010: Retain SQLite for the local single-owner v1

- Status: Accepted
- Date: 2026-09-03

## Context

The assistant needs durable storage for derived projections, immutable source
lineage, webhook replay claims, savings goals, alert state, categorisation
reviews, and owner settings. Firefly III and Ghostfolio remain the authoritative
ledgers; this database can be regenerated or privacy-purged without writing to
either upstream database.

The intended v1 deployment is personal, local, and single-owner. One gateway
process performs application writes. The optional Codex MCP process opens the
same database with SQLite `mode=ro`. Prototype evidence now covers five
reversible-before-data migrations, exact Decimal serialization, idempotent
upserts, concurrent webhook claims, restart persistence, consistent backup and
restore, source-specific deletion, and populated-state rollback refusal.

The remaining question is whether that workload justifies operating another
database service before practical personal use begins.

## Decision

Retain SQLite as the supported assistant-derived-state database for local v1.
Use one write-capable application process, WAL mode, application-level
transactions and locking, immutable source identities, and the existing
checksummed backup/restore workflow. Keep public-research cache state in its
separate disposable SQLite file.

Practical use is the next validation stage, not a reason to pre-emptively
migrate. Revisit this decision before any of these changes:

- more than one write-capable process or more than one owner;
- remote or multi-host self-hosting;
- recurring lock contention or write latency that affects normal use;
- a requirement for database-level identities, network access control,
  replication, point-in-time recovery, or high availability;
- backup size or recovery time that no longer meets the owner's needs.

Any migration must preserve canonical identifiers, immutable lineage, alert and
review audit history, exact decimal values, idempotency, privacy purge, and a
tested rollback or restore path. It requires a new ADR and migration issue.

## Rejected alternatives

- A dedicated PostgreSQL service now: technically capable, but adds another
  credential, process, network boundary, upgrade path, and backup/restore path
  without evidence that the personal single-writer workload needs them.
- Reusing Ghostfolio PostgreSQL or Firefly MariaDB: rejected because it couples
  assistant state and lifecycle to an upstream application's private database.
- DuckDB: useful for analytical scans, but not a better primary store for the
  mutable webhook, review, scheduling, and audit workload.

## Consequences

Local setup and recovery remain small, inspectable, and inexpensive. SQLite's
file permissions are the database access boundary; it supplies no independent
user authentication, network authorization, replication, or encryption at
rest. The host and every off-device backup therefore remain responsible for
encryption and access control.

Single-writer operation is an explicit v1 constraint rather than an accidental
implementation detail. Practical testing should record lock errors, latency,
database growth, restart behavior, integrity checks, and backup/restore time so
a future decision is based on observed workload evidence.
