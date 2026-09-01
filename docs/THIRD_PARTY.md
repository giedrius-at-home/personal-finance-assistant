# Third-Party Software

This repository integrates upstream software and does not fork or vendor it.
Deployment versions and image digests are pinned in `compose.yaml`. The
reviewed runtime and native-architecture evidence is linked from the
[validation guide](VALIDATION.md).

## Firefly III

- Purpose: cash-flow ledger, budgets, categories, rules, and piggy banks.
- Website: <https://www.firefly-iii.org/>
- Source: <https://github.com/firefly-iii/firefly-iii>
- API documentation: <https://api-docs.firefly-iii.org/>
- License: GNU Affero General Public License v3.0.
- Integration: documented API and signed webhooks.
- Pinned release: 6.6.6.

## Firefly III Data Importer

- Purpose: import bank exports and supported data sources into Firefly III.
- Source: <https://github.com/firefly-iii/data-importer>
- License: GNU Affero General Public License v3.0.
- Pinned release: 2.3.4.

## Ghostfolio

- Purpose: investment activities, holdings, allocation, and performance.
- Website: <https://ghostfol.io/>
- Source: <https://github.com/ghostfolio/ghostfolio>
- License: GNU Affero General Public License v3.0.
- Integration: public API; experimental MCP is not a foundation dependency.
- Pinned release: 3.65.0.

## Data services

- MariaDB LTS stores Firefly III state.
- PostgreSQL 15 stores Ghostfolio state.
- Redis stores disposable Ghostfolio cache state.
- Alpine 3.22 runs the local Firefly III cron request.
- All container references are pinned by immutable registry digest.

## Ollama and Qwen

- Ollama website: <https://ollama.com/>
- Qwen models: model-specific licensing must be recorded when the exact local
  variant is selected.
- Integration: localhost HTTP API with schema-constrained outputs.
- Locally validated baseline: `qwen3:8b` (8.2B parameters, Q4_K_M).
