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
databases are not published. See the [runtime validation
record](LOCAL_RUNTIME_VALIDATION.md) for the host and image evidence.

## First-time setup

Install dependencies and generate the local configuration:

```bash
uv sync --frozen --all-extras --dev
make bootstrap
```

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

The importer intentionally starts without a Firefly access token. Create and
configure permanent user accounts only after the deployment PR is reviewed and
merged. Real-data onboarding is a separate controlled step.

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
- the resolved image inventory;
- versioned metadata and SHA-256 checksums.

Backup directories and files are owner-only and excluded from Git. They are not
encrypted by the repository. Store any off-device copy only on an encrypted
destination. Redis is a cache and is not backed up.

## Restore

Restore is destructive and replaces current durable application state. Stop and
verify the exact backup directory before running:

```bash
make restore BACKUP=backups/20260901T121856Z
make doctor
```

The tool validates the manifest and every checksum before stopping application
services. It restores both databases and Firefly uploads, then starts the stack.
It refuses to run without the explicit confirmation supplied by the Make target.

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
