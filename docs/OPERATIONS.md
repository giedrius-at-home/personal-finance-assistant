# Local Operations

## Runtime

The validated Apple Silicon runtime uses Colima with Docker and Compose:

```bash
colima start --cpu 4 --memory 8 --disk 60 --vm-type vz --runtime docker
colima status
docker version
docker compose version
docker buildx version
```

Docker Desktop is not required. Application ports bind to `127.0.0.1` only;
databases are not published. See the [validation guide](VALIDATION.md) for the
current runtime gate and links to reviewed historical evidence.

## First-time setup

Install dependencies and generate the local configuration:

```bash
make setup
make bootstrap
```

`make setup` synchronizes the Python 3.14 environment and verifies the package
is importable. On macOS it also clears an inherited filesystem flag that can
cause Python to skip editable-install `.pth` files inside a hidden `.venv`.

`make bootstrap` reads `.env.example`, creates distinct cryptographic secrets,
writes `.env` with mode `0600`, and refuses to replace an existing file. Review
these non-secret values before startup:

- `PFA_TIMEZONE`
- the four `PFA_*_PORT` values
- `FIREFLY_SITE_OWNER`
- `PFA_OLLAMA_MODEL`

Do not edit generated secrets after an application has stored data. Keep a
secure credential backup of `.env`; do not commit or email it.

Start and inspect the stack:

```bash
make pull
make up
make doctor
make status
make versions
```

The local interfaces are:

- gateway: <http://127.0.0.1:8000/docs>
- owner dashboard: <http://127.0.0.1:8000/dashboard>
- Firefly III: <http://127.0.0.1:8080>
- Firefly III Data Importer: <http://127.0.0.1:8081>
- Ghostfolio: <http://127.0.0.1:3333>

The importer intentionally starts without a Firefly access token. Real-data
onboarding is a separate controlled step.

## Synthetic upstream contract identities

Before connecting any personal data, create dedicated synthetic local users in
Firefly III and Ghostfolio. Use generated, unique passwords and keep them only
in the local password manager or another encrypted store. Then create:

- a Firefly III personal access token for the synthetic user;
- a Ghostfolio bearer session for the synthetic user. The pinned 3.65.0
  self-hosted admin role cannot create a dedicated API key, so this credential
  expires after 180 days and must be rotated.

These upstream credentials may inherit broader user permissions even though the
assistant clients expose only GET operations. Treat both as sensitive and use
them only for these local synthetic identities.

Add only those two credentials to the ignored, owner-only `.env` file:

```dotenv
PFA_FIREFLY_API_TOKEN=<local synthetic token>
PFA_GHOSTFOLIO_BEARER_TOKEN=<local synthetic bearer token>
```

Do not place credentials on a command line, in shell history, test output, issue
comments, or commits. With the pinned stack healthy, verify the real response
contracts:

```bash
make upstream-contracts
```

This command performs read-only requests and reports test outcomes without
printing response bodies or secrets. It fails when either credential is absent;
the ordinary offline test suite skips these live-only checks.

Keep a local owner-only cleanup ledger under the ignored `credentials/`
directory containing each synthetic identity, its purpose, creation time,
credential kind, and deletion instructions. Remove the users and revoke or
rotate their credentials when the pinned contract environment is retired.

## Firefly webhook receipt

Firefly generates a random 24-character secret when a webhook is created. Copy
that value into `PFA_FIREFLY_WEBHOOK_SECRET` in the ignored `.env` before
recreating the gateway. The intended container-network URL would be:

```text
http://gateway:8000/v1/webhooks/firefly
```

However, Firefly 6.6.6 rejects Docker-private destination addresses as an SSRF
control, while loopback inside its container does not reach the gateway. Do not
weaken that protection or configure this native webhook in the current Compose
topology. The receiver is validated with source-compatible signed synthetic
requests while a polling-first or isolated-loopback relay design is reviewed.

Firefly signs the timestamp and exact JSON request bytes using HMAC SHA3-256.
The gateway verifies that signature before parsing, accepts at most five minutes
of clock skew in either direction, limits bodies to 1 MiB, and never logs the
secret or payload. Keep the host and container clocks synchronized. A valid
duplicate receives another successful acknowledgement without being claimed
twice.

Replay UUIDs are retained in assistant-owned SQLite state and survive gateway
restart. Acknowledgement proves authenticated durable receipt, not that a later
poll/reconciliation cycle has completed. Do not connect a real-data Firefly
webhook until the current polling-versus-relay topology limitation is reviewed.

## Assistant-derived state

The gateway initializes `/var/lib/pfa/assistant.sqlite3` in its dedicated named
volume. SQLite is the selected local single-owner v1 store for canonical
projections, immutable source-version lineage, and webhook delivery IDs only.
Firefly III and Ghostfolio remain authoritative. ADR 0010 records the
single-writer constraint and the conditions that require another database
review.

Derived projections can be deleted while retaining audit lineage, or a specific
source identity can be privacy-purged from both current and historical state.
Schema rollback refuses populated state. Exercise migration reversal only on an
empty synthetic database; never treat a migration rollback as a deletion tool.

During practical use, record any `database is locked` error, slow dashboard or
ingestion operation, unexpected database growth, failed integrity check, or
backup/restore delay. Also verify restart and desktop-sleep recovery. These are
decision inputs: do not add a second writer process or move the database to a
network filesystem as an attempted workaround.

## Routine commands

```bash
make doctor       # check every host-visible service, including Ollama
make status       # show container and health state
make versions     # report resolved container images
make logs         # show the latest 200 log lines per service
make stop         # stop containers while retaining volumes
make up           # recreate or restart the reviewed stack
make down         # remove containers and networks while retaining volumes
```

Never use `docker compose down --volumes` after introducing data. That command
deletes the databases and Firefly uploads.

## Alert scheduling and macOS delivery

Policy version `2026-09-v1` evaluates after a successful source event and against
the latest 18:00 slot in `PFA_TIMEZONE`. The SQLite scheduler lease ensures only
one worker claims a due slot. After sleep or downtime, the coordinator evaluates
once using current state; it does not replay each missed day. A failed evaluator
does not record success, and its 15-minute lease eventually permits recovery.

The macOS adapter is intentionally disabled unless the host-side caller opts in.
It cannot run from the Linux gateway container. When enabled on macOS, the OS may
request notification permission for the invoking terminal/application. Denial,
non-macOS execution, timeout, and command failure are recorded only as content-free
outcomes. A non-delivered attempt becomes retryable after one hour.

No launch agent is installed automatically in Phase 3. The scheduling and native
delivery components are ready for the local review application in Phase 5, where
the owner can enable them and tune policy without hidden background processes.
Until then, evaluations are invoked explicitly by the application/test boundary;
merging this phase does not begin sending notifications.

## Backup

Create an application-consistent local backup:

```bash
make backup
```

The operation briefly stops write-capable clients, places Firefly III in
maintenance mode, and writes a timestamped directory under `backups/`. It then
resumes the applications. Each backup contains:

- a MariaDB dump for Firefly III;
- a compressed archive of Firefly uploads;
- a PostgreSQL dump for Ghostfolio;
- a consistent SQLite copy of assistant-derived state;
- the resolved image inventory;
- versioned metadata and SHA-256 checksums.

Backup directories and files are owner-only and excluded from Git. They are not
encrypted by the repository. Store any off-device copy only on an encrypted
destination. Redis is a cache and is not backed up.

The assistant-state addition advances the manifest to backup schema 2. Phase 1
schema-1 backups remain historical evidence but are intentionally rejected by
the all-durable-state restore command because they contain no assistant state.

## Restore

Restore is destructive and replaces current durable application state. Stop and
verify the exact backup directory before running:

```bash
make restore BACKUP=backups/20260901T121856Z
make doctor
```

The tool validates the manifest and every checksum before stopping application
services. It restores both upstream databases, Firefly uploads, and assistant
derived state, then starts the stack. It refuses to run without the explicit
confirmation supplied by the Make target.

The Phase 1 validation created synthetic records in both databases and a
synthetic Firefly upload, backed them up, mutated all three, restored the backup,
and verified the original values returned. No user or financial data was used.

## Development service

The gateway can run without the upstream containers for Python development:

```bash
make check
make run
```

For a safe visual walkthrough with no upstream accounts or stored records:

```bash
make demo
```

Open <http://127.0.0.1:8002/dashboard>. `PFA_DEMO_MODE=true` creates only
invented in-memory accounts, transactions, a goal, and category proposals. It
does not write SQLite or either upstream system, and stopping the process is the
complete cleanup. Do not enable demo mode on a real-data runtime because the
invented view intentionally replaces its displayed derived state.

The normal dashboard uses SQLite-derived state. Category decisions and policy
changes accept URL-encoded POSTs only, require the process CSRF cookie/token and
same-origin requests, and return no-store responses. A process restart
invalidates open forms. Settings are shown once more as an exact preview before
apply and can be rolled back only while that revision is still current.

## Codex MCP and public research

Phase 6 does not enable Codex tools or outbound research automatically. After
updating an older generated `.env`, add and review:

```dotenv
PFA_CODEX_TOOLS_ENABLED=false
PFA_RESEARCH_ENABLED=false
PFA_RESEARCH_CACHE_PATH=/var/lib/pfa/research-cache.sqlite3
PFA_RESEARCH_USER_AGENT=personal-finance-assistant/0.1 (https://github.com/giedrius-at-home/personal-finance-assistant)
```

Keep research disabled for the first tool review. Rebuild and start the gateway,
then verify the STDIO server can start with `make codex-tools`; terminate it with
Control-C after its initialization message. It is a protocol process and has no
browser page or TCP listener.

To add it to Codex, configure a STDIO MCP server using Docker as the command and
these arguments, replacing both absolute repository paths for this workstation:

```toml
[mcp_servers.personal_finance]
command = "docker"
args = [
  "compose",
  "--env-file", "/absolute/repository/path/.env",
  "-f", "/absolute/repository/path/compose.yaml",
  "exec", "-T", "-e", "PFA_CODEX_TOOLS_ENABLED=true",
  "gateway", "python", "-m", "finance_assistant.mcp_server",
]
```

Restart Codex after changing its configuration. The local stack must be healthy
while the MCP server is in use. The process opens `assistant.sqlite3` with
SQLite `mode=ro` and offers exactly three tools:

- `get_finance_aggregates`: currency-separated cash flow, safe-to-spend, alert
  counts, de-identified goal progress, net worth, and taxonomy counts;
- `research_public_merchant`: an owner-approved public business name and optional
  two-letter country code;
- `research_public_market`: a validated ticker/CIK/ISIN and one enumerated public
  question. SEC lookup currently abstains for ISIN rather than guessing.

The server does not expose raw transactions, account/source/goal identifiers,
merchant bank text, SQL, arbitrary URLs, writes, payments, or trades. Codex is a
hosted-model boundary: anything returned by these tools may leave the device.
Do not enable the server if this aggregate disclosure is unacceptable; use the
local Ollama/Qwen path instead.

To opt into public retrieval, set `PFA_RESEARCH_ENABLED=true` and retain a
bounded, identifying user-agent string consistent with
[Wikimedia API etiquette](https://www.mediawiki.org/wiki/API:Etiquette) and
[SEC fair-access guidance](https://www.sec.gov/about/developer-resources). A
project URL is sufficient; do not put a secret in the value. Recreate the
gateway with `make up`. Public names/identifiers are disclosed only to their
fixed provider; raw financial data is never a valid query input.

Results expire after 24 hours by default. A cited expired result may be returned
with status `stale_cache` during an outage. Purge all cached public excerpts and
citations without changing derived financial state:

```bash
make purge-research-cache
```

The research cache is disposable, owner-readable only, excluded from Git, and
not included in `make backup`. Disable retrieval immediately and purge the cache
if a public subject was entered in error. Removing the MCP entry from Codex and
restarting Codex removes the hosted tool connection; it does not purge the local
cache.

## Troubleshooting

- Run `make doctor` before inspecting application-specific behaviour.
- Confirm `colima status` and `docker context show` before debugging containers.
- Use `make compose-config` to validate the committed template safely.
- Use `make logs` without enabling request-body or importer JSON logging.
- If Ollama is unavailable, confirm it uses the local model directory and listens
  on `127.0.0.1:11434`.
- Record operational discoveries here rather than relying on shell history.
