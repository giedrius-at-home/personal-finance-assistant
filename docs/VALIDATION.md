# Validation

This guide defines current validation expectations. Exact historical results
belong in the pull request that produced them so they remain attached to the
reviewed commit and CI run.

## Local quality gate

Run before every pull request:

```bash
uv sync --frozen --all-extras --dev
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
