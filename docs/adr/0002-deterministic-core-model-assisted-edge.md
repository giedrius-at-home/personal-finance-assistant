# ADR 0002: Deterministic core, model-assisted edge

- Status: Accepted
- Date: 2026-09-01

## Context

Language models are useful for semantic categorisation and explanation but are
not reliable canonical calculators or unrestricted data mutators.

## Decision

Implement balances, forecasts, goal schedules, thresholds, and audit behaviour
as deterministic typed Python. Qwen may classify and explain through constrained
schemas and approved vocabularies. It receives no database credentials and may
not initiate money movement or trades.

## Consequences

- Every financial figure is reproducible and testable without a model.
- LLM unavailability degrades explanations, not correctness.
- Classification requires confidence calibration, abstention, and human review.
- Extra adapter code is preferred over direct model access to source systems.
