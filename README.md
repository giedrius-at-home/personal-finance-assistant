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

The foundation release contains a typed FastAPI service and tested domain logic
for savings goals and budget warnings. It does not yet connect to Firefly III,
Ghostfolio, or real financial data. The three upstream applications have been
smoke-tested together on the target local container runtime; see the
[runtime validation record](docs/LOCAL_RUNTIME_VALIDATION.md). See also the
[project status](docs/STATUS.md) and the [implementation
plan](docs/IMPLEMENTATION_PLAN.md).

## Quick start

Prerequisites:

- Python 3.14+
- [uv](https://docs.astral.sh/uv/)
- Docker with Compose, optional for the container workflow

```bash
uv sync --all-extras --dev
uv run uvicorn finance_assistant.api:app --reload
```

Open `http://127.0.0.1:8000/docs` for the generated API documentation.

Alternatively:

```bash
docker compose up --build
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

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Implementation plan](docs/IMPLEMENTATION_PLAN.md)
- [Status and review ledger](docs/STATUS.md)
- [Privacy model](docs/PRIVACY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Operations](docs/OPERATIONS.md)
- [Local runtime validation](docs/LOCAL_RUNTIME_VALIDATION.md)
- [Third-party software](docs/THIRD_PARTY.md)
- [Architecture decisions](docs/adr/)
- [Changelog](CHANGELOG.md)

## Licensing

No project license has been selected yet. The repository is private and all
rights remain reserved until that decision is documented. Firefly III and
Ghostfolio retain their respective upstream licenses; see
[third-party software](docs/THIRD_PARTY.md).
