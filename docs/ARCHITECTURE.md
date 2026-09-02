# Architecture

## Context

The project provides a local intelligence and control layer above two upstream
systems:

- Firefly III owns cash-flow transactions, accounts, categories, budgets, and
  piggy-bank balances.
- Ghostfolio owns investment activities, holdings, valuations, and portfolio
  performance.

The assistant owns neither ledger. It owns mappings, derived features, goal
metadata, alert state, categorisation examples, decisions, and audit records.

## Intended components

```text
                         +----------------------+
Firefly API/webhooks --->|                      |
                         | Read-only gateway    |---> canonical events
Ghostfolio API/MCP ----->|                      |          |
                         +----------------------+          v
                                               deterministic domain services
                                                           |
                          +--------------------------------+----------------+
                          |                                |                |
                       goals                         categorisation       alerts
                          |                                |                |
                          +----------------------+---------+----------------+
                                                 |
                                          review and audit
                                                 |
                                       desktop notifications
```

Qwen through Ollama is outside the deterministic calculation boundary. It
receives the minimum context necessary for a classification or explanation and
returns validated structured output. A future restricted retrieval adapter may
fetch public information using sanitized queries.

## Data ownership and double counting

Every account is assigned one canonical owner. Transfers to an investment
account may exist in Firefly III, while holdings and performance exist in
Ghostfolio. Combined net-worth views must exclude duplicate representations by
using an explicit account mapping.

## Current vertical slice

The current implementation provides pure functions and HTTP contracts for:

- linear savings-goal progress and required weekly contribution;
- spending ratio, pace, forecast, remaining budget, and safe-to-spend;
- explainable severity selection;
- explicit currency precision and canonical account, transaction, and source
  lineage contracts;
- narrow read-only Firefly III and Ghostfolio clients with typed upstream DTOs,
  explicit authentication, bounded retry and pagination policies, and
  fail-closed response validation;
- a Firefly webhook receipt boundary that authenticates raw request bytes before
  JSON parsing, rejects stale timestamps, validates the pinned envelope, and
  durably acknowledges duplicate delivery UUIDs;
- fail-closed conversion of Firefly accounts and transactions plus Ghostfolio
  accounts into canonical records, with explicit cross-system account links;
- idempotent current projections, immutable source-version lineage, and exact
  explainable account-balance reconciliation in assistant-owned SQLite state.

Upstream DTOs remain inside `finance_assistant.integrations`; conversion into
canonical domain records occurs in the ingestion application layer. This keeps
upstream schema drift from silently changing deterministic domain logic.
No client has a write method, and failures never include response bodies or
credentials.

SQLite is a Phase 2 workload prototype, not a permanent database selection.
Issue `#17` retains that decision for review after operational evidence exists.
It stores only assistant projections, source-version metadata, validated
canonical JSON, and webhook delivery IDs. It is never queried by an LLM and has
no connection to either upstream database. Firefly remains authoritative for
cash transactions; Ghostfolio activities remain investment-domain records and
are not misrepresented as cash transactions.

Every cross-system account link explicitly selects either Firefly III or
Ghostfolio as canonical owner. A Ghostfolio-owned brokerage account uses its
portfolio value and retains the corresponding Firefly account only as lineage
and a transaction endpoint; a Firefly-owned link uses the Firefly balance. This
choice is configuration, never inferred from matching names or amounts.

## Local deployment topology

```text
localhost only
  :8000 gateway -------- application network -------- Firefly III :8080
       |                         |                          |
       |                         |                    internal network
       |                         |                          |
       |                         |                       MariaDB
       |                         |
       |                         +--------------------- Importer :8081
       |                         |
       |                         +------------------- Ghostfolio :3333
       |                                                    |
       |                                              internal network
       |                                             /                \
       +--> host.docker.internal --> Ollama    PostgreSQL             Redis
```

Only application HTTP ports are published, always on `127.0.0.1`. Database
services have no host bindings and live on separate internal Docker networks.
The gateway joins only the application network and cannot address either
database directly. Images are pinned by release tag and immutable digest.

The upstream databases, Firefly upload directory, and assistant-derived SQLite
state use named volumes. Redis is treated as disposable cache state. The
repository supplies checksummed backup and confirmed restore operations for all
durable state.

## Quality attributes

- Local-first and localhost-bound by default.
- Exact decimal arithmetic for money.
- Explicit currency minor units with no implicit rounding.
- Idempotent ingestion and immutable source references.
- Explainable decisions with audit records.
- Model abstention instead of forced classification.
- No direct database access by an LLM.
- No money movement or trading.
