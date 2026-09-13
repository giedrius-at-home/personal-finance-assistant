# ADR 0023: Schedule bank API imports independently of dashboard refreshes

Status: Accepted

## Context

The Firefly maintenance cron and the gateway poller do not retrieve bank data.
Owner-exported Enable Banking configurations contain the consent references and
reviewed account mappings needed by the pinned upstream importer CLI. These
exports are secrets and must never enter Git or Docker build contexts.

## Decision

On the local macOS deployment, a user LaunchAgent invokes `scripts/bank_imports.py`
every 15 minutes. Each configured bank is due daily after success, or hourly after
failure. Startup and wake catch up once rather than replaying missed intervals.
The host lock serializes runs; the CLI has an internal 15-minute timeout followed
by forced termination. The host timeout exceeds that bound.

The adapter calls the existing upstream importer inside its running container,
without restarting the browser importer, mounting a Docker socket, opening an
autoimport HTTP endpoint, or forking upstream. It sends configuration on stdin,
suppresses raw CLI output and upstream logging, and stores only allowlisted status
fields. Credentials and standalone installed adapters remain in a private macOS
Application Support directory, outside the repository and Documents folder.
The runner discovers exactly one running importer by its fixed Compose labels,
so recreation does not leave it using an obsolete container ID. No browser session
files are inspected or reused by the adapter.

Before every import, require active provider consent, membership of each source
account in that consent, and exact nonempty IBAN equality with an active Firefly
asset account. Reject missing, mismatched or new target mappings. Remove obsolete
asset-account creation instructions and embedded Firefly tokens from the runtime
configuration. Preserve classic duplicate detection and saved rule behavior; use
only booked transactions and a rolling 30-day overlap. Do not force balances.

If a bank omits the IBAN from subsequent account-details responses, an operator
may record `pfa_verified_identity` only after matching the actual authorization
screen IBAN to the existing ledger account. The binding records the exact source,
target, consent list and SHA-256 of the normalized IBAN, in private configuration.
It cannot override a conflicting live IBAN, expired consent, source not in consent,
changed target, changed ledger IBAN, or inactive/non-asset account. Renewal needs
new verification; never infer identity from the account name or balance.

Validate transaction pagination before invoking the writing CLI: repeated
continuation tokens or an unfinished 50-page scan block that bank. Some feeds
ignore date filters and repeat the same page; the upstream CLI otherwise repeats
that page up to its cap and can report a misleading success. This validation adds
one read-only traversal before importing and preserves upstream duplicate rejection.
The pinned version is checked before invoking its internal command/status API.

Swedbank or any other expired consent blocks only that bank. Upstream exit 73 alone
is not treated as success because importer 2.3.4 can use it after conversion errors.
Clean duplicate-only runs require completed conversion and submission stages with
no warnings, errors or rate limits. Enable the upstream option to ignore narrowly
recognized duplicate errors; duplicate rejection itself remains enabled.
Resolve explicit date bounds in the adapter because the CLI does not calculate
the browser's relative date range. The API upper bound is today (inclusive);
Swedbank rejects tomorrow with ASPSP_ERROR. The pinned importer also applies a
local midnight cutoff, so timestamped intraday records may arrive on the next run;
the rolling overlap recovers them.
Successful API access does not prove that a provider supplied complete bank data.

## Consequences

The Mac must be logged in with Docker running; sleep delays imports. This scheduler
is local to macOS, and has no Codex automation dependency. A portable always-on
deployment needs its own invocation of the runner. The runner uses the current
installed adapter copies; repository branch changes do not alter running imports.
Bank import status is available with `bank_imports.py status`, separately from the
dashboard's Firefly refresh timestamp. Native notifications are not introduced.
Normal five-minute gateway polling with a matching 30-day overlap makes new ledger records visible without a
gateway restart or a second SQLite writer.

Consent renewal requires a fresh owner-approved bank configuration. A missing
purchase may still be pending or absent from the provider feed. Imports older than
the rolling window require a separate reviewed backfill. Exported configurations
and upstream transient import-job files remain sensitive; the adapter deletes
its temporary configuration, but does not inspect or purge upstream sessions.
