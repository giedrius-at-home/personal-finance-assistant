# Changelog

All notable changes to this project will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project intends to use semantic versioning for its own releases.

## [Unreleased]

### Added

- Private GitHub repository foundation and review-driven governance.
- Typed FastAPI service with a non-sensitive health endpoint.
- Deterministic savings-goal schedule evaluation.
- Deterministic budget pace, forecast, and safe-to-spend evaluation.
- Unit and property-based tests using synthetic data.
- Local container definition and CI/security checks.
- Reproducible local runtime validation for Firefly III, Ghostfolio, and Qwen3.
- Python 3.14 development, typing, linting, and container baseline.
- Digest-pinned Firefly III, Data Importer, Ghostfolio, and isolated data services.
- One-time local secret generation and operational health diagnostics.
- Private, checksummed backup and explicitly confirmed restore tooling.
- Architecture, privacy, threat-model, operations, and decision documentation.
- Security reporting policy and non-persistent CI checkout credentials.
- GitHub Dependabot alerts, security updates, and enforced action SHA pinning.
- GitHub Project-backed roadmap, structured Issue Forms, and consolidated
  validation documentation.
- Explicit currency minor-unit validation and float rejection at financial
  boundaries.
- Canonical account, transaction, ownership, and immutable source-lineage
  contracts.
- Typed, read-only Firefly III and Ghostfolio API clients with bounded retries,
  pagination, response validation, and credential-safe failures.
- Opt-in contract tests against the pinned local upstream services.
- A Python 3.14 development setup check, including the macOS hidden-virtualenv
  compatibility repair.
