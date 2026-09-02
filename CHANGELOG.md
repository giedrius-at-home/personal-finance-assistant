# Changelog

All notable changes to this project will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project intends to use semantic versioning for its own releases.

## [Unreleased]

### Added

- Private GitHub repository foundation and review-driven governance.
- Typed FastAPI service with a non-sensitive health endpoint.
- Deterministic savings-goal schedule evaluation.
- Deterministic budget pace, forecast, and safe-to-spend evaluation.
- Unit and property-based tests using synthetic data.
- Local container definition and CI/security checks.
- Reproducible local runtime validation for Firefly III, Ghostfolio, and Qwen3.
- Python 3.14 development, typing, linting, and container baseline.
- Digest-pinned Firefly III, Data Importer, Ghostfolio, and isolated data services.
- One-time local secret generation and operational health diagnostics.
- Private, checksummed backup and explicitly confirmed restore tooling.
- Architecture, privacy, threat-model, operations, and decision documentation.
- Security reporting policy and non-persistent CI checkout credentials.
- GitHub Dependabot alerts, security updates, and enforced action SHA pinning.
- GitHub Project-backed roadmap, structured Issue Forms, and consolidated
  validation documentation.
- Explicit currency minor-unit validation and float rejection at financial
  boundaries.
- Canonical account, transaction, ownership, and immutable source-lineage
  contracts.
- Typed, read-only Firefly III and Ghostfolio API clients with bounded retries,
  pagination, response validation, and credential-safe failures.
- Opt-in contract tests against the pinned local upstream services.
- A Python 3.14 development setup check, including the macOS hidden-virtualenv
  compatibility repair.
- Firefly III webhook receipt with raw-body HMAC SHA3-256 verification, a
  five-minute timestamp window, strict envelopes, bounded bodies, and replay
  acknowledgement.
- Deterministic Firefly/Ghostfolio account mapping, Firefly transaction and
  transfer mapping, explicit ownership links, and fail-closed currency handling.
- Idempotent SQLite-derived projections with immutable source-version lineage,
  restart-safe webhook deduplication, exact reconciliation reports, reversible
  empty-state migrations, privacy purge, and checksummed backup/restore coverage.
- A labelled synthetic end-to-end ingestion contract covering duplicate
  prevention, transfers, account linking, persistence, and exact balances.
- Typed Firefly piggy-bank and budget-limit ingestion with canonical goal
  lineage, priority, protected savings, contribution schedules, and deterministic
  completion forecasts.
- Versioned deterministic goal/budget alert policy with exact aggregate evidence,
  durable cooldown, deduplication, escalation, resolution, and failed-delivery
  retry state.
- Offline-safe timezone-aware daily scheduling and an opt-in argument-safe native
  macOS notification adapter, both inactive until explicitly invoked.
- Deterministic, Unicode-aware merchant normalization with bounded feature
  extraction and a versioned category vocabulary plus exact/normalized baseline
  rules that fail closed on conflicts or unsupported transaction kinds.
- A constrained, local-only Ollama/Qwen client and categorisation adapter that
  passes redacted merchant text as untrusted JSON data, validates bounded
  structured output, rejects unapproved categories, and abstains on timeouts,
  schema drift, or model absence.
- Versioned confidence calibration and abstention with owner-approved correction
  holdouts, identity-separated splits, per-category precision gates, and a
  review-only default until automation evidence clears.
- Durable categorisation proposals, owner corrections, preview-token batch
  approval, and idempotent reversal through a typed write port that remains
  disabled in Phase 4 (no production Firefly write path).
- Synthetic Phase 4 lifecycle tests covering baseline, model, abstention,
  correction, preview, approval, conflict, reversal, replay, restart, and
  unavailable-model degradation.
- A responsive localhost owner dashboard and matching typed JSON view for a
  bounded category-review inbox, weekly cash flow, savings goals, active alert
  counts, taxonomy health, and net worth.
- Deterministic ISO-week and refund/transfer reporting with currency isolation,
  stable evidence hashes, and incomplete-week labelling.
- Canonical net-worth aggregation that counts linked mirrors once and fails
  closed on stale valuations or mixed currencies.
- Preview/confirm owner policy settings with validation, immutable audit
  revisions, and safe rollback.
- Same-origin POST and CSRF protection, strict host validation, no-store browser
  responses, restrictive browser security headers, bounded forms, and an
  ephemeral invented-data demo mode.
