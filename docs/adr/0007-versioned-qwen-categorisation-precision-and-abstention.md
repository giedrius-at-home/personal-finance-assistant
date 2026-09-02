# ADR 0007: Versioned Qwen categorisation precision and abstention policy

- Status: Accepted when merged
- Date: 2026-09-02
- Decision issue: [#20](https://github.com/giedrius-at-home/personal-finance-assistant/issues/20)

## Context

Phase 4 introduces explainable, Qwen-assisted transaction categorisation. A
model that may assign categories must not be able to invent them, silently
overwrite an upstream category, or automate on raw self-confidence alone.
False automatic categorisation is costlier than review volume at this stage, so
precision is the gate and coverage is reported but not optimised at its
expense.

## Decision

Adopt a conservative, versioned `2026-09-v1` policy:

- Qwen proposals default to review-only. Automation stays disabled until a
  frozen holdout of owner-approved corrections contains at least 100 examples
  overall and 20 predicted examples for every automated category.
- A category may be enabled only when its holdout precision is at least 95%,
  with zero label leakage across transaction identities.
- At inference, the calibrated category must have confidence of at least 0.92
  and must exceed the runner-up by at least 0.15; otherwise abstain.
- Unapproved categories, insufficient calibration support, malformed structured
  output, model unavailability, prompt injection, and conflicting deterministic
  rules always abstain.
- Deterministic exact and normalized merchant rules may propose independently
  but still flow through preview and audit. Phase 4 does not activate a
  production Firefly write path.

Thresholds and dataset fingerprints are versioned so the policy can be
recalibrated or rolled back without mutating upstream data or rewriting audit
history.

An automation allowlist cannot be supplied independently. It is derived from a
calibration report whose policy, vocabulary, model, and dataset fingerprint are
bound to the running categorisation service. Merchant features redact common
account numbers, long numeric identifiers, email addresses, and URLs before
they enter durable correction data or the local model prompt.

## Consequences

- Every automated category is traceable to an owner-approved holdout with
  measured precision, per-category support, and an explicit ambiguity margin.
- The review boundary is the only place corrections enter the evaluation set;
  nothing writes to Firefly in Phase 4.
- Categories are an explicit, versioned vocabulary. Model output that names an
  unapproved category, omits evidence, or exceeds size/schema bounds abstains.
- Abstention is a first-class, reviewable outcome rather than a silent failure.
- Holdout precision is evaluated offline from owner-approved corrections and
  cannot be polluted by the same transaction identity appearing in multiple
  splits.

## Alternatives rejected

- **Trust raw model self-confidence without holdout evidence:** non-deterministic
  and unverifiable for financial categorisation.
- **A global-only precision target:** can hide weak categories behind strong
  aggregate numbers.
- **Forced classification:** turns ambiguity and malformed output into incorrect
  categories instead of abstention.
- **Use unapproved or raw transaction history as training data:** leaks
  unverified labels into the evaluation gate.
- **Let the model write categories directly:** removes the review and reversal
  boundary that Phase 4 is designed to establish.

## References

- [Phase 4 parent issue](https://github.com/giedrius-at-home/personal-finance-assistant/issues/13)
- [ADR 0002: deterministic core, model-assisted edge](0002-deterministic-core-model-assisted-edge.md)
