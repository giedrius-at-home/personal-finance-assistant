# Personal Finance Assistant

The owner dashboard includes an **Until payday** overview and **Account data
health** panel. Set an optional monthly payday through the previewable settings
form to see cycle income and spending. Account history and category coverage
come from Firefly; last successful Firefly refresh is distinct from a bank CSV
import. See [operations](docs/OPERATIONS.md#account-health-and-payday-overview).

Saved Firefly subscriptions also appear with expected dates, cost ranges and
linked-payment evidence. Until payday is configured, the panel covers the next
30 days. Expected costs are forecasts, separate from recorded spending.

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
                                                  |
                      opt-in aggregate STDIO MCP -+-> Codex
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
the selected local single-owner v1 derived-state database; it never replaces
either upstream ledger. No real financial data has been introduced. See the
[GitHub Project](https://github.com/users/giedrius-at-home/projects/4)
for current status and roadmap, and the [operations guide](docs/OPERATIONS.md)
for the local runtime.

Phase 3 adds typed Firefly piggy-bank and budget-limit ingestion, lineaged
savings goals, completion forecasts, versioned deterministic alert policy,
durable cooldown/escalation/resolution state, offline-safe daily scheduling, and
an opt-in native macOS notification adapter. Delivery remains disabled until the
owner explicitly enables and tests macOS notification permission.

Phase 4 adds explainable, review-first transaction categorisation: deterministic
merchant normalization and baseline rules, a constrained local-only Ollama/Qwen
adapter with a versioned category vocabulary, confidence calibration and
abstention gated on owner-approved correction holdouts, and durable preview/approve/
reversal workflows. Model output can never invent categories and always flows
through owner review and audit. An optional category-only adapter can copy an
exact approved decision to Firefly; unreviewed model output has no write path.

Phase 5 adds a localhost owner dashboard and matching typed JSON view for the
category inbox, weekly cash flow, savings progress, alerts, and net worth.
Currency totals never mix, linked accounts are counted once, stale valuations
remove the combined total, and every report carries deterministic evidence.
Policy settings use preview/confirm, versioned audit history, and rollback.
Browser mutations are same-origin POSTs protected by CSRF; Firefly category
writes and notifications remain disabled by default.
The server-rendered dashboard progressively refreshes its main region while
visible and applies category-review actions in place. It uses one local script,
no third-party browser resources or storage, and retains normal navigation when
JavaScript is unavailable or an exact preview is required. Review cards show the
full Firefly description, signed amount, currency, and transaction date only in
the localhost owner interface, and display a visible success or failure toast.

Phase 6 adds disabled-by-default public research and an opt-in STDIO MCP server
for Codex. Research accepts only sanitized public business names or validated
market identifiers, constructs exact-allowlisted Wikipedia/SEC URLs internally,
labels every excerpt as untrusted, and retains citations in a separate purgeable
cache. The Codex tool surface exposes only currency-separated reports, goal
progress without identifiers, alert/taxonomy counts, and those bounded public
queries. It cannot read raw transactions, execute SQL, mutate either ledger,
move money, recommend personalized trades, or accept arbitrary URLs.

The owner dashboard can optionally poll Firefly III into its local derived
state. Polling runs inside the single gateway writer, starts immediately with
the gateway, and repeats daily by default. It is disabled until an owner adds a
dedicated token to the ignored `.env` file and opts in explicitly.
When reviewed category writes are enabled separately, Firefly remains the
authoritative category store and the next dashboard sync reads its value back.
Local Qwen suggestion generation is also separately opt-in and processes a
bounded batch of recent uncategorised, non-transfer transactions after each
Firefly sync or when the owner requests suggestions from the dashboard. The
current categorized Firefly history supplies a bounded, versioned vocabulary,
conflict-safe normalized merchant rules, and local-only categorized examples;
the model still cannot write or approve a category. Pending suggestions are retired
when the corresponding transaction has since been categorized directly in
Firefly.

The dashboard also has a disabled-by-default provider-neutral CSV resync path.
It accepts common bank export layouts, auto-detects or explicitly selects one
connected Firefly asset account, stages an exact preview, skips confirmed
duplicates, and blocks ambiguous repeats. Only the owner's confirmation creates
transactions, after which Firefly rules run and the dashboard refreshes.

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
# Optional: install a normal owner-scoped Mac app icon.
make install-macos-app
```

To populate and refresh the owner dashboard from Firefly III, create a
dedicated personal access token, place it only in the ignored `.env`, and set:

```dotenv
PFA_FIREFLY_API_TOKEN=<owner-only token>
PFA_FIREFLY_SYNC_ENABLED=true
PFA_FIREFLY_SYNC_INTERVAL_SECONDS=86400
# Optional: enable exact preview/confirm CSV imports from the dashboard.
PFA_FIREFLY_CSV_IMPORTS_ENABLED=true
# Optional: comma-separated aliases that identify transfers between your accounts.
PFA_OWNER_NAME_ALIASES=<full name>,<abbreviated name>
```

Recreate only the gateway with `docker compose up --build --detach gateway`.
The first read-only sync runs at startup. Never commit or paste the token into
logs, shell arguments, issues, or chat.

Each successful full Firefly sync removes current dashboard projections that no
longer exist upstream while retaining immutable source-version history. Owner
name aliases are matched locally against normalized transaction descriptions;
matching inflows and outflows are reported as transfers and excluded from cash
flow income, spending, and category totals.

The generated `.env` is owner-readable only, ignored by Git, and never
overwritten by `make bootstrap`. All browser endpoints bind to loopback:

- gateway API: <http://127.0.0.1:8000/docs>
- owner dashboard: <http://127.0.0.1:8000/dashboard>
- Firefly III: <http://127.0.0.1:8080>
- Firefly III Data Importer: <http://127.0.0.1:8081>
- Firefly III Data Importer HTTPS: <https://localhost:8444> (self-signed)
- Ghostfolio: <http://127.0.0.1:3333>

After `make install-macos-app`, open **Personal Finance Assistant** from
`~/Applications` or Launchpad. It starts Docker through Colima when necessary,
starts the existing reviewed containers, waits for health, and opens the owner
dashboard. The app contains no repository path, credentials, or financial data.
Run `make up` from the repository once after initial setup or a code update.

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

To review the Phase 6 MCP surface, keep the local stack running and leave
`PFA_RESEARCH_ENABLED=false` initially. `make codex-tools` starts a STDIO server
in the gateway container; it is intended to be launched by an MCP client, not
opened in a browser. Adding it to Codex is an explicit privacy opt-in because
tool arguments and results may be sent to the configured hosted model. See the
[operations guide](docs/OPERATIONS.md#codex-mcp-and-public-research) for the
configuration and cache-purge workflow.

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
