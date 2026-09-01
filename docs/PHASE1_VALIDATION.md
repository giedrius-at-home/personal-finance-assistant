# Phase 1 Validation Record

Date: 2026-09-01

This record covers the repository-managed local upstream stack. All validation
used fresh project volumes, generated local secrets, and synthetic probes only.
No user account, financial record, bank export, or portfolio activity was used.

## Configuration and security

- `.env` was generated from `.env.example` with distinct cryptographic values.
- File mode was verified as `0600`; no generation marker remained.
- A second bootstrap attempt is covered by a unit test and refuses overwrite.
- The committed Compose template resolved successfully without local secrets.
- Every application binding resolved to `127.0.0.1`.
- MariaDB, PostgreSQL, and Redis had no published host ports.
- Firefly and Ghostfolio data networks reported `internal=true`.
- The gateway joined only the application network.
- Every upstream image ran natively on `linux/arm64` from its pinned digest.

## Service validation

| Service | Version | Result |
| --- | --- | --- |
| Python gateway | 0.1.0 / Python 3.14.7 | healthy |
| Firefly III | 6.6.6 | migrations, integrity, health passed |
| Data Importer | 2.3.4 | production startup and UI health passed |
| Ghostfolio | 3.65.0 | migrations, startup, health passed |
| MariaDB | LTS digest pin | healthy, internal only |
| PostgreSQL | 15 Alpine digest pin | healthy, internal only |
| Redis | Alpine digest pin | authenticated, healthy, internal only |
| Firefly cron | Alpine 3.22 digest pin | crond running read-only |
| Ollama / Qwen | 0.33.2 / `qwen3:8b` | host health passed |

`make doctor` passed after initial startup, after full-container restart, after
backup quiescence/resume, and after restore. The full Compose project survived a
simultaneous restart of every service.

## Backup and restore proof

The test created three synthetic values:

- `firefly-original` in a temporary Firefly database probe table;
- `ghostfolio-original` in a temporary Ghostfolio database probe table;
- `firefly-upload-original` in a temporary Firefly upload file.

`make backup` produced owner-only SQL dumps, a compressed upload archive, image
inventory, metadata, and SHA-256 checksums. All three source values were then
changed to `*-mutated`. A confirmed restore verified the backup manifest before
stopping applications and restoring data.

Post-restore queries returned exactly the three original values. All services
then passed `make doctor`. The temporary probe tables and file were removed from
the live stack. Historical synthetic backups remain ignored by Git and contain
no personal data.

A second backup exercised the application-consistent path: write-capable clients
stopped, Firefly entered maintenance mode, durable state was captured, Firefly
returned to live mode, clients resumed, and all health checks passed.

## Code quality

- 32 tests passed on CPython 3.14.5.
- Coverage remained 97.45%, above the required 90%.
- Ruff formatting and lint passed with the Python 3.14 target.
- Strict mypy passed for application and operational Python code.
- Bandit covered `src` and `scripts`.
- `pip-audit` reported no known dependency vulnerabilities.
- Docker Compose validation and the pinned Python container build passed.

## Residual local state

- The repository-managed stack is running for owner inspection.
- Generated `.env`, named volumes, and backups are local and ignored by Git.
- Preliminary smoke containers were stopped without deleting their old synthetic
  volumes.
- Permanent upstream users and access tokens have not been created.
- Real-data onboarding remains explicitly out of scope for this phase.
