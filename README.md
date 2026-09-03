# Personal Finance Assistant

[![CI](https://github.com/giedrius-at-home/personal-finance-assistant/actions/workflows/ci.yml/badge.svg)](https://github.com/giedrius-at-home/personal-finance-assistant/actions/workflows/ci.yml)

Local-first personal finance intelligence layer for Firefly III and Ghostfolio,
powered by Ollama, with savings goals, explainable categorisation, and proactive
spending alerts.

> [!IMPORTANT]
> This project provides personal analytics and decision support. It does not
> provide financial advice, move money, or place trades.

## Why this exists

[Firefly III](https://www.firefly-iii.org/) is the source of truth for cash flow,
transactions, budgets, categories, and savings accounts.
[Ghostfolio](https://ghostfol.io/) is the source of truth for investments and
portfolio performance. This repository integrates their public interfaces; it
does not fork or vendor either application.

The assistant adds a controlled local layer for:

- savings-goal forecasting;
- safe-to-spend and spending-pace calculations;
- proactive, rate-limited warnings;
- explainable transaction categorisation with an abstention path;
- local Qwen analysis through Ollama;
- an auditable review workflow for every automated change.

## Trust model

```text
Firefly III ----\
                  -> read-only gateway -> deterministic analytics -> alerts/review
Ghostfolio -----/                               |
                                                   -> Qwen/Ollama explanations
Public web data -> restricted retrieval adapter --/
```

Financial calculations are deterministic Python code. The LLM may classify,
summarise, and explain, but does not calculate canonical balances or write
directly to application databases. Money movement and trading are permanently
out of scope.

## Current state

The project now contains a typed FastAPI service, tested domain logic, typed
read-only Firefly III and Ghostfolio clients, and a pinned local deployment of
both upstream systems. The domain includes explicit currency precision and
canonical source-lineage contracts. Deterministic adapters map accounts and
Firefly transactions into idempotent assistant-owned derived state, with exact
balance reconciliation and durable signed-webhook replay protection. SQLite is
the Phase 2 prototype pending the recorded database decision; it never replaces
either upstream ledger. No real financial data has been introduced. See the
[GitHub Project](https://github.com/users/giedrius-at-home/projects/4)
for current status and roadmap, and the [operations guide](docs/OPERATIONS.md)
for the local runtime.

Phase 3 adds typed Firefly piggy-bank and budget-limit ingestion, lineaged
savings goals, completion forecasts, versioned deterministic alert policy,
durable cooldown/escalation/resolution state, offline-safe daily scheduling, and
an opt-in native macOS notification adapter. Delivery remains disabled until the
owner explicitly enables and tests macOS notification permission.

Phase 4 adds explainable, review-only transaction categorisation: deterministic
merchant normalization and baseline rules, a constrained local-only Ollama/Qwen
adapter with a versioned category vocabulary, confidence calibration and
abstention gated on owner-approved correction holdouts, and durable preview/approve/
reversal workflows. No production Firefly write path is activated in this phase;
model output can never invent categories and always flows through preview and
audit.

Phase 5 adds a localhost owner dashboard and matching typed JSON view for the
category inbox, weekly cash flow, savings progress, alerts, and net worth.
Currency totals never mix, linked accounts are counted once, stale valuations
remove the combined total, and every report carries deterministic evidence.
Policy settings use preview/confirm, versioned audit history, and rollback.
Browser mutations are same-origin POSTs protected by CSRF; upstream category
writes and notifications remain disabled by default.

## Quick start

Prerequisites:

- Python 3.14+
- [uv](https://docs.astral.sh/uv/)
- Docker with Compose
- Ollama with `qwen3:8b`, optional until model integration is implemented

```bash
make setup
make bootstrap
# Review non-secret settings such as owner email and timezone in .env.
make up
make doctor
```

The generated `.env` is owner-readable only, ignored by Git, and never
overwritten by `make bootstrap`. All browser endpoints bind to loopback:

- gateway API: <http://127.0.0.1:8000/docs>
- owner dashboard: <http://127.0.0.1:8000/dashboard>
- Firefly III: <http://127.0.0.1:8080>
- Firefly III Data Importer: <http://127.0.0.1:8081>
- Ghostfolio: <http://127.0.0.1:3333>

```bash
make status
make versions
make backup
make down
```

## Development

```bash
make format
make lint
make test
make check
# After configuring synthetic local API credentials:
make upstream-contracts
```

To inspect Phase 5 without accounts or persisted records, run `make demo` and
open <http://127.0.0.1:8002/dashboard>. Every displayed record is invented in
memory and disappears when the process stops.

No test or CI workflow may use real financial records. Synthetic fixtures are
the default and are reviewed like source code. The end-to-end ingestion fixture
is explicitly labelled as invented and proves mapping, duplicate prevention,
persistence, transfers, account linking, and exact reconciliation together.

Planning lives in GitHub rather than duplicate repository ledgers: Issues hold
requirements and acceptance criteria, the Project holds priority and status,
and pull requests hold implementation evidence. Accepted architecture decisions
remain version-controlled in `docs/adr/` because they must change with the code.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Project roadmap](https://github.com/users/giedrius-at-home/projects/4)
- [Requirements and decisions](https://github.com/giedrius-at-home/personal-finance-assistant/issues)
- [Privacy model](docs/PRIVACY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Security policy](SECURITY.md)
- [Operations](docs/OPERATIONS.md)
- [Validation](docs/VALIDATION.md)
- [Third-party software](docs/THIRD_PARTY.md)
- [Architecture decisions](docs/adr/)
- [Changelog](CHANGELOG.md)

## Licensing

No project license has been selected yet. The repository is private and all
rights remain reserved until that decision is documented. Firefly III and
Ghostfolio retain their respective upstream licenses; see
[third-party software](docs/THIRD_PARTY.md).
