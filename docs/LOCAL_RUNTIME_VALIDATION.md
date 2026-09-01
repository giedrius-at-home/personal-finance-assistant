# Local Runtime Validation

This document records the disposable, synthetic-data smoke test performed on
2026-09-01 before introducing the full upstream deployment into this
repository. It is evidence of local feasibility, not the production deployment
definition.

## Host and tooling

- macOS 26.6.2 (`arm64`)
- Colima 0.10.3 using Apple Virtualization Framework, Docker, and `virtiofs`
- Docker client 29.7.2 and server 29.5.2
- Docker Compose 5.5.0
- Docker Buildx 0.36.1
- Ollama 0.33.2
- Python 3.14.5 for local development and `python:3.14-slim` for the gateway
- Colima allocation: 4 CPUs, 8 GiB memory, and a 60 GiB requested disk

Colima, the Docker CLI, Compose, and Buildx were installed with Homebrew. The
Docker client plugin directory is configured in the user's untracked Docker
configuration. No Docker Desktop installation is required.

## Applications exercised

| Component | Tested version | Local endpoint | Result |
| --- | --- | --- | --- |
| Foundation gateway | repository image `0.1.0` | `127.0.0.1:8000/health` | healthy |
| Firefly III | `6.6.6` | `127.0.0.1:8080/health` | `200 OK` |
| Ghostfolio | `3.65.0` | `127.0.0.1:3333/api/v1/health` | `{"status":"OK"}` |
| Ollama / Qwen | `qwen3:8b`, Q4_K_M | `127.0.0.1:11434` | deterministic completion passed |

Firefly III completed its initial migrations, correction jobs, integrity
report, OAuth key generation, and version security check. Ghostfolio completed
its PostgreSQL migrations and started its Nest application. Both stacks passed
a full-container restart followed by a fresh health check, which also exercised
their named-volume state.

The gateway reached Ollama from its container at
`http://host.docker.internal:11434`. Qwen returned exactly `LOCAL_LLM_OK` for a
temperature-zero, non-thinking smoke prompt. This validates the local network
path planned for later model-assisted categorisation; it does not validate the
categorisation quality or safety policy.

After selecting Python 3.14 as the project baseline, the full local quality
suite ran on CPython 3.14.5. The rebuilt gateway reported CPython 3.14.7 from
the pinned base image, remained healthy, reached Ollama, and received the exact
Qwen smoke response `PYTHON_314_OK`.

## Resolved image evidence

The test used native `linux/arm64` images. These registry digests were resolved
at test time:

- `fireflyiii/core@sha256:ae69fdd95cdef9038cd7a460a5aec731f14813973e4f096511d5a4ea9ff0e972`
- `ghostfolio/ghostfolio@sha256:5762d47097bb30f6e8c51a1a0ebd9a39798631b512d8a78223bcfe68202e5286`
- `python@sha256:656d12e70054d5fda18a045e2494c96701e9792dd1445f95b3d038df954f57e9`
- `mariadb@sha256:dd9b303aed4f4890ed09f766d8ca9ddfd176c0c6f6267feff53b3192ec65a979`
- `postgres@sha256:fe0737ba566a2c5b2a28f34433c0a423261900ec17b9bf7ad115e1aae7e57f1b`
- `redis@sha256:becdda6c7f4b3fb42e42fd7f120bbf5c54c4caaaf16f26da24e4563d2c1f0576`

These are validation evidence, not permanent dependency pins. Deployment pins
will be reviewed and automated in the PR that adds the upstream stack.

## Isolation and data handling

- Only application ports were published, each on `127.0.0.1`.
- MariaDB, PostgreSQL, and Redis remained on private Compose networks.
- Credentials were synthetic, disposable, stored outside the repository, and
  never used for real accounts.
- No real transaction, portfolio, identity, or bank data was introduced.
- Firefly III used `APP_ENV=local` only for this smoke test and correctly warned
  that a durable deployment must use `production`.
- The Ghostfolio application ran with all Linux capabilities dropped and with
  `no-new-privileges` enabled, matching its upstream hardening pattern.

## Reproduction shape

The disposable validation used separate Compose projects and named volumes:

```bash
docker compose config --quiet
docker compose pull
docker compose up --detach
docker compose ps
curl --fail http://127.0.0.1:8080/health
curl --fail http://127.0.0.1:3333/api/v1/health
docker compose restart
```

The exact smoke files are intentionally outside version control because they
contain disposable credentials and are not yet the reviewed deployment design.
The next implementation phase will add secret templates, pinned images,
healthchecks, backup/restore verification, and automated synthetic integration
tests to the repository.

## Current local state and cleanup

At the end of validation, all three application stacks were left running for
developer inspection. The pages are available at:

- gateway API documentation: <http://127.0.0.1:8000/docs>
- Firefly III: <http://127.0.0.1:8080>
- Ghostfolio: <http://127.0.0.1:3333>

To stop a disposable stack while retaining its synthetic volume data, run
`docker compose down` from its Compose directory. To remove that stack's
synthetic data as well, run `docker compose down --volumes` only after verifying
the current directory and Compose project name. Do not use that destructive
form once real data exists.
