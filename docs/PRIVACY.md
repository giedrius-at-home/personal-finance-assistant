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

Web retrieval is disabled by default and accepts only an owner-approved public
business name or a validated ticker/CIK/ISIN plus an enumerated public question.
It rejects email addresses, URLs, IBAN-like values, long numbers, controls, and
query syntax at the merchant boundary. It never accepts a raw bank description,
transaction amount, account/transaction/goal identifier, balance, holding, or
arbitrary URL.

Only internally constructed HTTPS requests to exact English Wikipedia and SEC
hosts are permitted. Returned excerpts are bounded, cited, hashed, and labelled
as untrusted public web content. The separate `research-cache.sqlite3` contains
only sanitized public query keys and cited excerpts, is owner-readable only,
can be purged independently, and is not included in durable backups.
Merchant lookup exposes evidence only when exactly one returned page title
matches the approved public name; zero or multiple matches abstain without
returning excerpts to the model.

## Optional external model use

Codex integration is an explicit opt-in STDIO MCP server exposing only the
documented aggregate report and bounded research schemas. It has no generic
database, filesystem, SQL, HTTP, payment, mutation, or trading tool. Connecting
it to a hosted model can transmit tool inputs and outputs outside the device;
running the tool server locally does not make the resulting conversation local.
Use Qwen/Ollama when model-visible data must remain on the workstation.

## Retention

Retention periods and deletion workflows will be defined alongside persistence.
Local backups are owner-only and tested for restoration but are not encrypted by
the repository. Before real data is introduced, the owner must confirm an
encrypted local or off-device storage boundary for every retained backup.
Public-research cache entries expire after 24 hours by default and can be
removed immediately with `make purge-research-cache`.
