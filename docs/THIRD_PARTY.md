# Third-Party Software

This repository integrates upstream software and does not fork or vendor it.
Exact deployment versions and image digests will be recorded in the PR that
introduces the local stack.

## Firefly III

- Purpose: cash-flow ledger, budgets, categories, rules, and piggy banks.
- Website: <https://www.firefly-iii.org/>
- Source: <https://github.com/firefly-iii/firefly-iii>
- API documentation: <https://api-docs.firefly-iii.org/>
- License: GNU Affero General Public License v3.0.
- Integration: documented API and signed webhooks.

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

## Ollama and Qwen

- Ollama website: <https://ollama.com/>
- Qwen models: model-specific licensing must be recorded when the exact local
  variant is selected.
- Integration: localhost HTTP API with schema-constrained outputs.
