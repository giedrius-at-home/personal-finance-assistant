# Changelog

All notable changes to this project will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project intends to use semantic versioning for its own releases.

## [Unreleased]

### Changed

- Added currency-separated calendar-month spending groups with expandable
  store categories, gross-purchase shares, refund-aware totals and equal-day
  previous-month comparisons, without changing Firefly transactions.
- Added reviewed, revision-protected store-to-group mappings and optional local
  model suggestions that require a separate save; preserved open sections,
  search and unsaved group choices during dashboard updates.

- Made the dashboard category inbox a pending-only work queue while retaining
  resolved proposals in the local audit history.
- Added optional local embedding retrieval over bounded, identifier-redacted,
  owner-approved Firefly examples; pending reviews are regenerated after a
  classifier upgrade while existing Firefly categories remain untouched.
- Taught the local categorizer to use recognizable merchant identity and
  ordinary business type as evidence, with lower confidence or abstention when
  either is unclear.
- Added first-party progressive dashboard refresh and in-place category review
  actions with visible result toasts and full transaction context, while
  preserving server-rendered fallback and exact preview screens.
- Rebuilt category vocabularies, conflict-safe merchant rules, and bounded local
  Qwen examples from current categorized Firefly history after each sync while
  preserving owner review as the only approval path.
- Made Firefly the authoritative category source for dashboard reporting and
  added opt-in, owner-reviewed, category-only updates with stale-preview
  conflict detection; local model suggestions still cannot write automatically.
- Added bounded local-Qwen suggestion generation for recent uncategorised
  non-transfer transactions, with explicit opt-in and a dashboard trigger.
- Pruned deleted Firefly accounts and transactions from the current dashboard
  after complete syncs, added configurable owner-name transfer recognition, and
  made cash-account balances and pay-cycle context clearer on the dashboard.
- Formatted dashboard money at each currency's declared precision and honored
  Firefly's per-account net-worth inclusion preference.
- Retained SQLite as the supported local single-owner v1 derived-state database
  after prototype review, with explicit single-writer, practical-validation,
  recovery, and future migration triggers recorded in ADR 0010.

### Fixed

- Added an explicit confirmed resolution for blocked CSV previews so owners can
  import only definitely new rows while leaving possible duplicates and
  unreadable rows unchanged.
- Stopped guessing a CSV provider from transaction descriptions and rejected
  confident filename/account mismatches before staging an import.
- Passed the selected Firefly source account ID—not its internal canonical ID—
  from the dashboard CSV form, so explicit account selection bypasses ambiguous
  bank-name detection correctly.

### Added

- Read-only Firefly subscription ingestion and an upcoming-bills panel using
  Firefly-calculated expected dates, configured cost ranges and linked-payment
  evidence; stale totals are withheld and missing links are not called unpaid.
- An account-health panel with observed history, saved-category coverage and
  durable Firefly refresh outcomes, plus configurable monthly payday cash flow.
  Bank statement completeness and reconciliation remain explicitly unverified;
  no spending allowance is inferred from incomplete planning inputs.
- An explicit dashboard merchant-research action that accepts only a validated
  public store name, uses the existing allowlisted and cache-backed research
  boundary, labels results untrusted, and never submits transaction text.
- A disabled-by-default, provider-neutral dashboard CSV resync flow with
  semantic bank-export parsing, automatic or explicit account targeting,
  exact preview/confirm, duplicate and ambiguity protection, Firefly rule
  application, and an immediate derived-dashboard refresh.
- Durable, configurable importer HTTPS with a persisted local certificate,
  read-only injection of ignored Enable Banking credentials, separate durable
  Firefly importer authentication, and documented per-bank recovery/rotation.
- Disabled-by-default Firefly polling inside the single gateway writer, with an
  immediate startup sync, configurable daily interval, asset-account filtering,
  idempotent derived-state updates, credential-safe logging, and shutdown-safe
  cancellation.
- An owner-scoped native macOS launcher with a source-controlled icon that can
  start Colima and the existing local containers, wait for loopback health, and
  open the dashboard without reading the repository or embedding credentials or
  financial data.
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
- A disabled-by-default public research gateway with typed sanitized queries,
  internally constructed exact-allowlisted Wikipedia/SEC URLs, public-DNS and
  redirect checks, bounded streaming responses, fail-closed parsing, citations,
  untrusted-content labels, and an owner-only disposable cache with stale mode.
- Bounded merchant and public-market research contracts that prohibit bank text,
  arbitrary URLs, holdings, forecasts, personalized advice, category mutation,
  and trades.
- An opt-in read-only STDIO MCP server for Codex exposing only documented
  currency-separated financial aggregates and restricted public research, with
  no raw records, identifiers, database access, or mutation tools.
- Synthetic privacy/security tests for query leakage, prompt injection labels,
  SSRF and redirect rejection, response limits, cache recovery, aggregate
  canaries, disabled-by-default network access, and non-advisory market output.
