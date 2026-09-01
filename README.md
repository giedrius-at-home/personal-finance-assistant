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

The project now contains a typed FastAPI service, tested domain logic, and a
pinned local deployment of Firefly III, its Data Importer, and Ghostfolio. The
domain includes explicit currency precision and canonical source-lineage
contracts; the gateway does not yet ingest upstream data, and no real financial
data has been introduced. See the [GitHub Project](https://github.com/users/giedrius-at-home/projects/4)
for current status and roadmap, and the [operations guide](docs/OPERATIONS.md)
for the local runtime.

## Quick start

Prerequisites:

- Python 3.14+
- [uv](https://docs.astral.sh/uv/)
- Docker with Compose
- Ollama with `qwen3:8b`, optional until model integration is implemented

```bash
uv sync --all-extras --dev
make bootstrap
# Review non-secret settings such as owner email and timezone in .env.
make up
make doctor
```

The generated `.env` is owner-readable only, ignored by Git, and never
overwritten by `make bootstrap`. All browser endpoints bind to loopback:

- gateway API: <http://127.0.0.1:8000/docs>
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
```

No test or CI workflow may use real financial records. Synthetic fixtures are
the default and are reviewed like source code.

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
