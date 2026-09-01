# ADR 0004: Local container topology and secrets

- Status: Accepted
- Date: 2026-09-01

## Context

Firefly III, its Data Importer, Ghostfolio, the Python gateway, and their data
services must run reproducibly on one developer workstation. The deployment
must not expose databases, commit credentials, or make an LLM a database peer.
It must also support recovery before real financial data is introduced.

## Decision

Use one Docker Compose project with official upstream images pinned by release
tag and immutable digest. Publish only application HTTP ports on `127.0.0.1`.
Place MariaDB and the Ghostfolio PostgreSQL/Redis services on distinct internal
networks; the gateway joins only the shared application network.

Generate high-entropy application and database secrets from a reviewed template
into an ignored, owner-only `.env`. Refuse automatic overwrite or rotation.
Keep durable data in named volumes. Back up both databases and Firefly uploads
under an ignored, owner-only directory, record SHA-256 checksums, and require an
explicit confirmation flag before restore. Redis remains disposable cache.

## Consequences

- A clean setup requires only the documented runtime, `make bootstrap`, and
  `make up`.
- Database traffic is unavailable from the host and unrelated containers.
- Dependency updates require a reviewed digest change, normally from Dependabot.
- Losing `.env` can make encrypted or signed application state unusable; it must
  be included in the owner's secure credential backup, separate from Git.
- Local backups are sensitive and not encrypted by this repository. Any
  off-device copy must use an encrypted destination.
- Secret rotation and remote/self-hosted networking require later procedures and
  security decisions.
