# Validation

This guide defines current validation expectations. Exact historical results
belong in the pull request that produced them so they remain attached to the
reviewed commit and CI run.

## Local quality gate

Run before every pull request:

```bash
make setup
make check
docker compose --env-file .env.example config --quiet
docker build --tag personal-finance-assistant:validation .
```

`make check` enforces formatting, linting, strict typing, Bandit, dependency
auditing, tests, and the coverage threshold. CI repeats these checks with
read-only permissions and SHA-pinned actions.

## Runtime gate

Changes to the local stack or integrations must additionally demonstrate, with
synthetic data only:

```bash
make bootstrap
make up
make doctor
make status
make backup
make down
```

Typed upstream-client changes must also pass authenticated read-only contracts
against the pinned local versions:

```bash
make upstream-contracts
```

The identities and credentials used for this gate remain ignored, owner-only,
and listed in a local cleanup ledger. Evidence records counts and outcomes only,
never response bodies or credential values.

Webhook changes must prove valid Firefly-compatible SHA3-256 signatures are
accepted, invalid and stale signatures are rejected before parsing, concurrent
duplicate UUID claims are atomic and survive restart, malformed envelopes fail
closed, and neither secrets nor raw bodies appear in error responses.

Ingestion changes must use explicitly synthetic fixtures and prove pagination,
currency precision, transfers, updates, malformed-contract rejection, ambiguous
ownership rejection, idempotent persistence, immutable source lineage, and
exact reconciliation without implicit rounding.

Goal and alert changes must additionally prove piggy-bank and budget-limit
contracts, ineligible-goal reasons, forecasts, policy boundaries, exact aggregate
evidence, concurrent deduplication, cooldown expiry, severity escalation,
resolution, delivery failure retry, restart persistence, timezone scheduling,
single-run sleep catch-up, native argument isolation, and backup survival. CI
must never invoke an actual desktop notification.

Restore changes must prove that checksums are verified, explicit confirmation
is required, original synthetic values are recovered, and every service passes
`make doctor` afterward. Destructive cleanup is never part of the routine gate.

## Required evidence

A pull request records:

- exact automated commands and results;
- relevant manual checks and environment constraints;
- synthetic fixtures or probes used;
- security, privacy, failure-mode, and recovery checks;
- known limitations and deferred follow-up Issues.

No credentials, real financial data, account identifiers, bank exports, model
prompts containing personal data, or unredacted service logs may appear in
validation evidence.

## Historical baseline

- [PR #1](https://github.com/giedrius-at-home/personal-finance-assistant/pull/1)
  established the Python 3.14 service, deterministic financial domain logic,
  container build, and initial CI baseline.
- [PR #4](https://github.com/giedrius-at-home/personal-finance-assistant/pull/4)
  validated the digest-pinned Firefly III, Data Importer, Ghostfolio, databases,
  Ollama/Qwen connectivity, restart behaviour, and checksummed backup/restore
  workflow using synthetic state.
- [PR #5](https://github.com/giedrius-at-home/personal-finance-assistant/pull/5)
  strengthened repository security controls and revalidated the complete local
  quality gate.
