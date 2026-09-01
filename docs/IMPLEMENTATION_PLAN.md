# Implementation Plan

## Delivery principles

- Each phase is delivered through a reviewable pull request.
- Real data is introduced only after synthetic contract tests pass.
- Every architectural decision is recorded in an ADR.
- Deterministic calculations precede model-generated explanations.
- Deployment remains local until self-hosting has an explicit threat model.

## Phase 0 — Repository foundation

Deliverables:

- private repository, protected `main`, squash-only merge strategy;
- Python package, lock file, container, local commands;
- CI, dependency updates, security checks, synthetic tests;
- initial domain vertical slice and documentation set.

Acceptance criteria:

- all local checks and GitHub CI pass;
- container binds only to localhost;
- no secret or real-data fixture exists in Git history;
- savings and safe-to-spend invariants are property-tested.

## Phase 1 — Local upstream stack

Deliverables:

- pinned Firefly III, importer, and Ghostfolio deployment definitions;
- separate upstream databases and volumes;
- documented local secrets bootstrap;
- health checks, backup, restore, and version-report commands.

Acceptance criteria:

- all services survive a restart;
- a clean-machine setup is reproducible;
- backup restoration is demonstrated with synthetic records;
- upstream versions and licenses are recorded.

## Phase 2 — Canonical ingestion

Deliverables:

- typed Firefly and Ghostfolio API clients;
- verified Firefly webhook receiver;
- canonical account and transaction contracts;
- idempotency keys, deduplication, and account ownership mapping;
- contract tests against pinned upstream versions.

Acceptance criteria:

- replaying an event does not duplicate a record;
- late updates replace derived state without losing lineage;
- mixed currencies and ambiguous account ownership fail closed;
- synthetic balances reconcile across source and canonical views.

## Phase 3 — Goals and alerts

Deliverables:

- Firefly piggy-bank mapping;
- goal dates, priority, protected amounts, and contribution schedules;
- scheduled and event-triggered budget evaluation;
- alert cooldown, deduplication, and escalation state;
- native macOS notification adapter.

Acceptance criteria:

- alert calculations trace back to exact inputs;
- duplicate webhooks cannot produce notification storms;
- goals show required contribution and completion forecast;
- sleeping/offline desktop behaviour is documented.

## Phase 4 — Explainable categorisation

Deliverables:

- deterministic Firefly-rule baseline;
- merchant normalization and feature extraction;
- Qwen structured classifier with approved category vocabulary;
- confidence calibration and abstention;
- review inbox, corrections, and reversible batch changes;
- evaluation dataset derived only from approved local corrections.

Acceptance criteria:

- model output cannot create arbitrary categories;
- high-confidence automation meets an agreed precision target;
- every change has before/after preview and audit data;
- prompt injection in transaction text is handled as untrusted content.

## Phase 5 — Review experience and reports

Deliverables:

- local review interface;
- weekly cash-flow, goal, and portfolio summary;
- category taxonomy recommendations;
- alert-sensitivity and notification settings.

Acceptance criteria:

- the owner can understand why every alert or classification occurred;
- reports never double-count mapped investment accounts;
- no real financial data appears in application logs by default.

## Phase 6 — Restricted web research and optional Codex access

Deliverables:

- domain allowlist, sanitized queries, caching, and source citations;
- merchant and public-market research tools;
- aggregate-only MCP tools for optional Codex use.

Acceptance criteria:

- account identifiers and transaction context cannot enter search queries;
- web content is isolated as untrusted data;
- tool outputs expose only documented aggregate fields.

## Deferred — Self-hosting and deployment

Self-hosting, remote access, email delivery, container publishing, and continuous
deployment require a separate decision covering identity, TLS, backups, network
boundaries, monitoring, and incident response.
