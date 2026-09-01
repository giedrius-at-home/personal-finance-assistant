# Privacy Model

## Default boundary

All application services, derived data, Ollama inference, and notifications are
local. Services bind to loopback unless a future ADR explicitly changes this.

## Data minimisation

- Source applications retain authoritative financial records.
- Derived storage keeps only fields needed for analytics, mappings, review, and
  audit.
- The LLM receives task-specific context, not unrestricted database access.
- Logs exclude transaction descriptions, account identifiers, balances, tokens,
  and model prompts containing personal data by default.
- CI and tests use synthetic records exclusively.

## Web retrieval

Web access is not implemented in the foundation. A future adapter must sanitize
queries, enforce an allowlist where practical, cache sources, and treat all
retrieved content as untrusted. Transaction amounts, full bank descriptions,
account identifiers, and balances must not enter search queries.

## Optional external model use

Any future Codex or hosted-model integration must expose aggregate, explicitly
documented tools. Connecting a local MCP server to a hosted model does not make
the resulting prompts local.

## Retention

Retention periods and deletion workflows will be defined alongside persistence.
Backups containing personal data must be encrypted and tested for restoration.
