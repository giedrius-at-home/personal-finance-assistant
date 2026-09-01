# ADR 0001: Local-first integration repository

- Status: Accepted
- Date: 2026-09-01

## Context

The system combines Firefly III, Ghostfolio, a local model, and original
analytics. Forking either upstream application would increase upgrade and
security maintenance while obscuring data ownership.

## Decision

Create a standalone private repository that consumes official, pinned upstream
containers and public interfaces. Run locally and bind original services to
loopback. Do not copy or modify upstream source code.

## Consequences

- Upstream upgrades remain isolated and require contract testing.
- The gateway must normalize two distinct domain models.
- Self-hosting and remote ingress require a separate ADR.
- Third-party versions and licenses must remain documented.
