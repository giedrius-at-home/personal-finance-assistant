# ADR 0012: Inject importer secrets and keep browser sessions ephemeral

- Status: Accepted
- Date: 2026-09-05

## Context

The Data Importer needs durable Firefly and Enable Banking credentials plus a
stable local HTTPS callback. Its file-backed browser sessions can also contain
live access and refresh tokens, provider sessions, and an uploaded private key.
Persisting the complete session directory would improve browser continuity but
would create an opaque secret store with unclear expiry, backup, and rotation.

Data Importer 2.3.4 supports a Firefly personal access token, an Enable Banking
application ID, and an Enable Banking private-key file. The container can also
reuse an existing TLS keypair instead of regenerating it.

## Decision

Inject a dedicated Firefly importer token and the Enable Banking application ID
from the ignored owner-only `.env`. Mount the matching PEM from the ignored
`credentials/` directory through a read-only Compose secret. Keep the dashboard
and importer Firefly tokens separate so either integration can be revoked
without changing the other.

Persist `/etc/ssl/private` in a dedicated named volume to retain the local
self-signed HTTPS identity across container recreation. Make the host HTTPS port
configurable while retaining 8444 as the default.

Do not persist `/var/www/html/storage/framework/sessions`. Treat browser sessions
as a temporary cache and use explicit credentials plus separately retained
per-bank configurations for recovery. Do not include importer sessions,
credentials, or bank configurations in routine application backups.

## Consequences

Routine importer recreation preserves the local TLS certificate and no longer
requires re-entering the Firefly token, Enable Banking application ID, or PEM.
Browser session continuity is intentionally sacrificed; a provider may require
fresh consent after recreation or session expiry.

Credential backup, rotation, and revocation remain explicit owner operations.
The default `/dev/null` secret keeps non-Enable-Banking setups usable and avoids
the 2.3.4 behavior that treats an unset private-key path as a directory.

## Rejected alternatives

- Persist the complete session directory: convenient, but retains several live
  credentials in an opaque application-managed cache.
- Put raw PEM contents in `.env` or Compose: increases accidental disclosure and
  makes rotation/error handling harder than a read-only file.
- Terminate TLS elsewhere: unnecessary for this loopback-only deployment and
  introduces another component and trust boundary.
