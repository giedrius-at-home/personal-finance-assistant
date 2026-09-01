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

The initial implementation contains no persistence or upstream adapters. It
provides pure functions and HTTP contracts for:

- linear savings-goal progress and required weekly contribution;
- spending ratio, pace, forecast, remaining budget, and safe-to-spend;
- explainable severity selection.

This creates a testable domain boundary before external data is introduced.

## Quality attributes

- Local-first and localhost-bound by default.
- Exact decimal arithmetic for money.
- Idempotent ingestion and immutable source references.
- Explainable decisions with audit records.
- Model abstention instead of forced classification.
- No direct database access by an LLM.
- No money movement or trading.
