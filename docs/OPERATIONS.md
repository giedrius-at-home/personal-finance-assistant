# Local Operations

## Container runtime

The initial Apple Silicon validation used Colima with Docker and Compose:

```bash
colima start --cpu 4 --memory 8 --disk 60 --vm-type vz --runtime docker
colima status
docker version
docker compose version
docker buildx version
```

Docker Desktop is not required. Keep application bindings on `127.0.0.1`; do
not publish database or Ollama ports beyond the host. See the [local runtime
validation](LOCAL_RUNTIME_VALIDATION.md) for the tested versions and upstream
smoke-test results.

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
- Check `colima status` and `docker context show` before debugging containers.
- Record operational discoveries here rather than relying on shell history.
