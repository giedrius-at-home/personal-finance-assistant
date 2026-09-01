# Local Operations

## Foundation service

```bash
uv sync --all-extras --dev
make check
make run
```

The service listens on `127.0.0.1:8000` when run through Docker Compose.

```bash
docker compose up --build --detach
docker compose ps
curl --fail http://127.0.0.1:8000/health
docker compose logs gateway
docker compose down
```

## Configuration

Copy `.env.example` to `.env` locally and change only documented settings. The
local `.env` file must never be committed. No secrets are needed by the initial
foundation service.

## Backups and recovery

Persistence is not implemented in Phase 0. Backup and restore commands will be
delivered with the upstream local stack and must be tested before real data is
introduced.

## Troubleshooting

- Confirm `uv run pytest` succeeds before investigating runtime behaviour.
- Inspect `/health` without enabling request-body logging.
- Use `docker compose config` to validate local substitutions.
- Record operational discoveries here rather than relying on shell history.
