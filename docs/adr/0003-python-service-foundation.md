# ADR 0003: Python service foundation

- Status: Accepted
- Date: 2026-09-01

## Context

The owner has professional Python and data-engineering experience. The domain
needs exact arithmetic, typed contracts, data transformations, statistical
evaluation, and local HTTP integrations.

## Decision

Use Python 3.12+, FastAPI, Pydantic, uv, pytest, Hypothesis, Ruff, and strict
mypy. Keep the initial review interface server-rendered when it is introduced.
Use `Decimal` for all monetary amounts and reject unknown API fields.

## Consequences

- One primary implementation language covers APIs, analytics, and evaluation.
- Property-based testing can express financial invariants.
- A separate frontend toolchain is deferred until demonstrated necessary.
- Currency precision policy remains an explicit follow-up decision.
