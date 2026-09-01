# Project Status

Last updated: 2026-09-01

## Completed before the first PR

- Repository created privately under `giedrius-at-home`.
- `main` protected from direct pushes, force pushes, and deletion.
- Squash-only merging and automatic branch deletion configured.
- Product boundaries and local-first trust model documented.
- Initial Python service, deterministic goal/alert logic, and synthetic tests
  implemented on `feat/project-foundation`.

## In review

- Phase 0 repository foundation.

## Not started

- Firefly III and Ghostfolio local deployment.
- Upstream API adapters, webhook ingestion, and persistence.
- Ollama/Qwen integration.
- Native desktop notifications and review UI.
- Real financial-data onboarding.

## Open decisions

- Project license.
- Operational database for derived state after SQLite prototyping.
- Currency minor-unit and valuation precision policy.
- Concrete alert thresholds and cooldown defaults.
- Qwen model variant and evaluation target.

## Review checklist

- Confirm project boundaries and terminology.
- Review ADRs and rejected alternatives.
- Review goal schedule assumption: linear progress between start and target dates.
- Review alert threshold defaults in `domain/alerts.py`.
- Review the conservative first-day forecast assumption: one effective elapsed day.
- Confirm licensing preference before any public release.
