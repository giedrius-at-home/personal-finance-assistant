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
volume. This Phase 2 SQLite store contains canonical projections, immutable
source-version lineage, and webhook delivery IDs only. Firefly III and
Ghostfolio remain authoritative, and issue `#17` tracks the permanent database
decision after prototype evidence is reviewed.

Derived projections can be deleted while retaining audit lineage, or a specific
source identity can be privacy-purged from both current and historical state.
Schema rollback refuses populated state. Exercise migration reversal only on an
empty synthetic database; never treat a migration rollback as a deletion tool.

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

## Troubleshooting

- Run `make doctor` before inspecting application-specific behaviour.
- Confirm `colima status` and `docker context show` before debugging containers.
- Use `make compose-config` to validate the committed template safely.
- Use `make logs` without enabling request-body or importer JSON logging.
- If Ollama is unavailable, confirm it uses the local model directory and listens
  on `127.0.0.1:11434`.
- Record operational discoveries here rather than relying on shell history.
