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

The SQLite local-v1 decision is reviewed through practical operation as well as
tests. Record lock failures, representative ingestion/dashboard latency,
database size growth, restart and sleep recovery, `PRAGMA integrity_check`, and
backup/restore duration without recording financial values. Multiple writers,
multi-owner or remote hosting, recurring contention, or unmet recovery needs
trigger a new database ADR rather than an unreviewed topology change.

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

Production polling changes must additionally prove disabled-by-default startup,
required secret/config validation, immediate first execution, bounded intervals,
failure isolation, cancellation on shutdown, asset-account filtering, and
content-free logs. Runtime validation records counts only and confirms a second
sync changes no unchanged projections.

Goal and alert changes must additionally prove piggy-bank and budget-limit
contracts, ineligible-goal reasons, forecasts, policy boundaries, exact aggregate
evidence, concurrent deduplication, cooldown expiry, severity escalation,
resolution, delivery failure retry, restart persistence, timezone scheduling,
single-run sleep catch-up, native argument isolation, and backup survival. CI
must never invoke an actual desktop notification.

Categorisation changes must additionally prove identifier-redacting merchant
normalization, baseline precedence and conflict abstention, vocabulary/schema
rejection of unapproved
categories and prompt injection, loopback-only model connectivity, bounded
structured output, abstention on timeout/absence/schema drift, identity-separated
holdout precision with dataset/model/vocabulary binding, preview-token batch
approval, write-conflict rollback, idempotent reversal, restart
persistence, and unavailable-model degradation. The Phase 4 write port must
remain disabled in CI and production validation.

CSV resync changes must prove size and row bounds, common delimiter/date/decimal
and debit-credit layouts, explicit or unambiguous account selection, stable
opaque identities, exact and possible-duplicate separation, repeated-row
ambiguity, preview expiry, Firefly recheck-before-write, transfer creation only
for exact asset-account IBAN matches, rule application, CSRF/same-origin
protection, and disabled-by-default operation. Fixtures must be wholly synthetic.

Phase 5 dashboard/reporting changes must prove deterministic output under input
reordering, ISO-week timezone boundaries, refunds, transfer exclusion, explicit
currency separation, linked-account deduplication, stale-valuation fail-closed
behaviour, and sparse/duplicate-like/overloaded taxonomy signals. Browser tests
must cover the invented empty/filled views, narrow-screen layout, semantic
controls, exact review decisions, settings preview/apply/rollback, invalid Host,
cross-origin and missing-CSRF rejection, no-store/CSP headers, and the absence of
third-party requests. Progressive updates must retain server-rendered fallback,
pause during edits, serialize requests, avoid browser storage, and leave exact
CSV/settings previews as normal navigation. Screenshots and console/network observations must contain
invented data only and stay in pull-request evidence rather than the repository.

Phase 6 changes must prove that private/query-like merchant inputs are rejected,
market identifiers and questions are enumerated, URLs are internally built, and
exact hosts, HTTPS/443, public DNS answers, redirects, response content types,
sizes, parsing, citations, and cache expiry fail closed. Network-off and stale-
cache behavior must be deterministic. Prompt-injection text remains visibly
labelled untrusted and never changes a category or initiates advice/trades.

macOS launcher changes must prove the generated bundle identifier, executable
mode, icon presence, fixed loopback dashboard/health URLs, non-overwriting
installation, fixed expected container names, non-macOS failure, and absence of
repository/config access or optional-feature overrides in the executable.
Manual validation may install the owner app, launch the healthy synthetic/local
stack, and inspect the fixed operational log; removal must not touch application
data.

MCP validation must enumerate the complete tool list and schemas, confirm
read-only annotations and SQLite `mode=ro`, pass private account/source/raw-input
canaries through populated synthetic state, and prove none appear in aggregate
output. Extra arguments must not widen the contract. Public research remains
network-disabled without explicit opt-in, and no MCP test may contact a live
provider. Cache purge must be independently recoverable and content-free.

Phase 6 retrieval changes must prove that private/query-like merchant text is
rejected, only internally constructed allowlisted URLs are reachable, redirects
are revalidated, private/non-global DNS answers fail closed, proxies are ignored,
response sizes and content types are bounded, citations and untrusted labels are
always present, stale cache use is explicit, and disabled research performs no
network call. MCP tests must inspect its complete tool list and annotations,
seed synthetic source/account/argument canaries, and prove that no raw record or
identifier reaches aggregate output. They must also prove extra holdings/trade
arguments cannot enable advice or action and that all research remains disabled
without explicit opt-in. Cache purge must be independently recoverable.

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

Importer changes must also prove that both default and configured HTTPS ports
render through `docker compose config`, the PEM is mounted read-only, the TLS
volume survives importer recreation, and the browser session directory is not
mounted. Validate callbacks with synthetic credentials only. Evidence may name
paths and report file modes, but must never include application IDs, tokens,
PEM contents, bank configurations, or session-file output.

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
