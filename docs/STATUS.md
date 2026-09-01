# Project Status

Last updated: 2026-09-01

## Completed before the first PR

- Repository created privately under `giedrius-at-home`.
- `main` protected from direct pushes, force pushes, and deletion.
- Squash-only merging and automatic branch deletion configured.
- Product boundaries and local-first trust model documented.
- Initial Python service, deterministic goal/alert logic, and synthetic tests
  implemented on `feat/project-foundation`.
- Colima/Docker development runtime installed and the foundation image verified
  healthy on native Apple Silicon.
- Firefly III 6.6.6, Ghostfolio 3.65.0, and their isolated databases verified
  locally with synthetic state, including restart health checks.
- Existing `qwen3:8b` verified through Ollama, including gateway-to-host network
  reachability.

## In review

- Phase 0 repository foundation.

## Not started

- Reviewed Firefly III and Ghostfolio deployment definitions in this repository.
- Upstream API adapters, webhook ingestion, and persistence.
- Ollama/Qwen application integration and categorisation evaluation.
- Native desktop notifications and review UI.
- Real financial-data onboarding.

## Open decisions

- Project license.
- Operational database for derived state after SQLite prototyping.
- Currency minor-unit and valuation precision policy.
- Concrete alert thresholds and cooldown defaults.
- Qwen categorisation evaluation corpus, quality target, and abstention threshold
  (`qwen3:8b` is the initial local baseline).

## Review checklist

- Confirm project boundaries and terminology.
- Review ADRs and rejected alternatives.
- Review goal schedule assumption: linear progress between start and target dates.
- Review alert threshold defaults in `domain/alerts.py`.
- Review the conservative first-day forecast assumption: one effective elapsed day.
- Confirm licensing preference before any public release.
