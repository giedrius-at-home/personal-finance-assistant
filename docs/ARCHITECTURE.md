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
  fail-closed response validation.

Upstream DTOs remain inside `finance_assistant.integrations`; conversion into
canonical domain records will occur in the ingestion application layer. This
keeps upstream schema drift from silently changing deterministic domain logic.
No client has a write method, and failures never include response bodies or
credentials.

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

The upstream databases and Firefly upload directory use named volumes. Redis is
treated as disposable cache state. The repository supplies checksummed backup
and confirmed restore operations for all durable state.

## Quality attributes

- Local-first and localhost-bound by default.
- Exact decimal arithmetic for money.
- Explicit currency minor units with no implicit rounding.
- Idempotent ingestion and immutable source references.
- Explainable decisions with audit records.
- Model abstention instead of forced classification.
- No direct database access by an LLM.
- No money movement or trading.
