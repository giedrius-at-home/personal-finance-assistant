# Security Policy

## Supported versions

The project is not released yet. Security fixes target the current `main`
branch; older commits and feature branches are not supported versions.

## Reporting a vulnerability

Do not include credentials, tokens, personal financial data, transaction
descriptions, account identifiers, or exploit details in a normal issue.

Use the repository's private vulnerability-reporting channel when it is
available. Otherwise, contact
[@giedrius-at-home](https://github.com/giedrius-at-home) privately before
sharing sensitive details. A metadata-only issue may be opened to request a
private contact channel, but it must not describe the vulnerability.

Include only the minimum information required to reproduce and assess the
problem:

- affected commit or component;
- impact and prerequisites;
- synthetic reproduction steps;
- suggested mitigation, if known.

The maintainer aims to acknowledge a report within seven days and provide an
initial assessment within fourteen days. These are targets for a personal
project, not a guaranteed service-level agreement.

## Security boundaries

This software is local-first and must not be exposed publicly without a new
architecture and threat-model decision. The assistant must never initiate money
movement, place trades, receive unrestricted database access, or send personal
financial data to web retrieval services. See the [threat
model](docs/THREAT_MODEL.md) and [privacy model](docs/PRIVACY.md) for the
enforced design boundaries.

All reports and tests must use synthetic data. If real credentials or financial
data are exposed, revoke or isolate them first; deleting them from Git history
alone does not make them safe again.
