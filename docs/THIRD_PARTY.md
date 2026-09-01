# Third-Party Software

This repository integrates upstream software and does not fork or vendor it.
Deployment versions and image digests will be pinned in the PR that introduces
the local stack. The versions below were exercised in the preliminary local
runtime validation; resolved test-time digests are recorded in
[LOCAL_RUNTIME_VALIDATION.md](LOCAL_RUNTIME_VALIDATION.md).

## Firefly III

- Purpose: cash-flow ledger, budgets, categories, rules, and piggy banks.
- Website: <https://www.firefly-iii.org/>
- Source: <https://github.com/firefly-iii/firefly-iii>
- API documentation: <https://api-docs.firefly-iii.org/>
- License: GNU Affero General Public License v3.0.
- Integration: documented API and signed webhooks.
- Locally validated release: 6.6.6.

## Firefly III Data Importer

- Purpose: import bank exports and supported data sources into Firefly III.
- Source: <https://github.com/firefly-iii/data-importer>
- License: GNU Affero General Public License v3.0.

## Ghostfolio

- Purpose: investment activities, holdings, allocation, and performance.
- Website: <https://ghostfol.io/>
- Source: <https://github.com/ghostfolio/ghostfolio>
- License: GNU Affero General Public License v3.0.
- Integration: public API; experimental MCP is not a foundation dependency.
- Locally validated release: 3.65.0.

## Ollama and Qwen

- Ollama website: <https://ollama.com/>
- Qwen models: model-specific licensing must be recorded when the exact local
  variant is selected.
- Integration: localhost HTTP API with schema-constrained outputs.
- Locally validated baseline: `qwen3:8b` (8.2B parameters, Q4_K_M).
