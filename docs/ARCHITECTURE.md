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
returns validated structured output. The public retrieval adapter accepts only
sanitized domain requests and returns cited, explicitly untrusted excerpts.

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
  fail-closed response validation;
- a Firefly webhook receipt boundary that authenticates raw request bytes before
  JSON parsing, rejects stale timestamps, validates the pinned envelope, and
  durably acknowledges duplicate delivery UUIDs;
- fail-closed conversion of Firefly accounts and transactions plus Ghostfolio
  accounts into canonical records, with explicit cross-system account links;
- idempotent current projections, immutable source-version lineage, and exact
  explainable account-balance reconciliation in assistant-owned SQLite state.
- typed Firefly piggy-bank and budget-limit boundaries, canonical savings goals,
  deterministic completion forecasts, and versioned alert candidates;
- durable alert lifecycle and daily evaluation leases that bound duplicate,
  concurrent, restart, and desktop-sleep behavior;
- deterministic merchant normalization, a versioned category vocabulary, and
  exact/normalized baseline rules with fail-closed conflict handling, including
  bounded local supervision rebuilt from current categorized Firefly history;
- a constrained local-only Ollama/Qwen client and adapter with identifier-redacted
  merchant features, JSON-isolated untrusted text, bounded schema-validated output,
  and abstention on unapproved primary or alternative categories;
- versioned confidence calibration gated on owner-approved correction holdouts,
  durable categorisation proposals, and preview-token batch approval/reversal
  through a typed write port that stays disabled in Phase 4;
  suggestions superseded by later direct Firefly categories are retired;
- an opt-in host-side macOS adapter that receives only pre-rendered minimal text.
- a server-rendered localhost dashboard and typed JSON projection built from the
  same deterministic reporting service, with bounded review items and one
  first-party progressive-enhancement script but no third-party resources;
- ISO-week cash-flow reports that model refunds and transfers explicitly, keep
  currencies separate, and bind output to a stable evidence hash;
- net-worth aggregation that counts each canonical account once, exposes linked
  mirrors as excluded evidence, and withholds a combined total for stale or
  mixed-currency state;
- a provider-neutral, in-memory CSV staging boundary with semantic column
  detection, explicit account selection, duplicate/ambiguity classification,
  exact owner confirmation, and narrow Firefly transaction creation;
- versioned owner policy settings with an exact preview token, explicit apply,
  audit history, and last-write-safe rollback.
- a disabled-by-default public research boundary with internally constructed
  exact-host Wikipedia and SEC requests, public-address checks, bounded fetches,
  fail-closed parsers, citations, and a separate disposable cache;
- an opt-in STDIO MCP server whose schemas expose only currency-separated
  aggregates and bounded public-research inputs, never raw records or IDs.

Upstream DTOs remain inside `finance_assistant.integrations`; conversion into
canonical domain records occurs in the ingestion application layer. This keeps
upstream schema drift from silently changing deterministic domain logic.
Write-capable clients are separately opt-in and narrowly limited to reviewed
category updates or exact owner-confirmed CSV transactions. Failures never
include response bodies or credentials.

SQLite is the selected derived-state database for the local, single-owner v1.
ADR 0010 records its single-writer constraint and the operational triggers for
reconsidering PostgreSQL. It stores only assistant projections, source-version
metadata, validated canonical JSON, and webhook delivery IDs. It is never
queried directly by an LLM and has no connection to either upstream database.
Firefly remains authoritative for cash transactions; Ghostfolio activities
remain investment-domain records and are not misrepresented as cash
transactions.

Optional Firefly polling runs as a background task inside the gateway process,
so SQLite retains one write-capable process. The task performs an immediate
read-only snapshot at startup and repeats at the configured interval. It uses
only Firefly asset accounts, transactions, currencies, and piggy banks; errors
are isolated from HTTP availability and logged by exception type without raw
upstream content or credentials. ADR 0011 records this production ingestion
boundary.

The Data Importer receives durable Firefly and Enable Banking credentials only
through ignored owner-only configuration and a read-only PEM secret. Its local
TLS identity is persisted independently, while browser sessions remain
ephemeral because they may contain several live credentials. ADR 0012 records
this separation.

Every cross-system account link explicitly selects either Firefly III or
Ghostfolio as canonical owner. A Ghostfolio-owned brokerage account uses its
portfolio value and retains the corresponding Firefly account only as lineage
and a transaction endpoint; a Firefly-owned link uses the Firefly balance. This
choice is configuration, never inferred from matching names or amounts.

Alert audit records retain exact canonical aggregate evidence, hashes, policy
version, reason codes/text, and lifecycle outcomes. They never contain raw
transactions, webhook bodies, notification message content, or credentials.
Qwen does not calculate severity or control delivery.

The MCP process is launched inside the gateway container and opens the same
assistant-derived SQLite volume using SQLite `mode=ro`, without migrations.
The implementation is additionally limited to report service methods and has no
write tool. The public-research cache is a separate writable SQLite file in the
volume and is not part of the durable backup manifest.

Codex connects over STDIO through `docker compose exec`; no MCP TCP listener is
created. Codex is outside the local trust boundary: aggregate outputs and public
query inputs may leave the workstation according to its configured model and
service policy. Ollama/Qwen remains available for workflows that must stay local.

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

The upstream databases, Firefly upload directory, and assistant-derived SQLite
state use named volumes. Redis is treated as disposable cache state. The
repository supplies checksummed backup and confirmed restore operations for all
durable state.

## Quality attributes

- Local-first and localhost-bound by default.
- Exact decimal arithmetic for money.
- Explicit currency minor units with no implicit rounding.
- Idempotent ingestion and immutable source references.
- Explainable decisions with audit records.
- Model abstention instead of forced classification.
- No direct database access by an LLM.
- No money movement or trading.
