# Threat Model

## Protected assets

- financial transactions, balances, account identifiers, and goals;
- upstream API tokens and webhook secrets;
- Ollama prompts and classification examples;
- derived behavioural profiles and alert history;
- backups and exports.

## Trust boundaries

1. Browser to localhost application.
2. Gateway to Firefly III and Ghostfolio.
3. Firefly webhook sender to webhook receiver.
4. Deterministic services to Ollama.
5. Future retrieval adapter to the public internet.
6. Developer workstation to GitHub Actions.

## Initial threats and controls

| Threat | Initial control |
|---|---|
| Credential committed to Git | Ignore rules, synthetic-only policy, CI scan |
| Financial data enters logs | Redaction policy and no request-body logging |
| Webhook spoofing or replay | HMAC verification, timestamp window, idempotency |
| Duplicate imports | Immutable source identifiers and idempotent upserts |
| LLM invents or mutates data | Schema validation, vocabulary, review gates |
| Prompt injection in merchant text | Treat descriptions as data, not instructions |
| Notification storm | Stateful deduplication, cooldown, daily aggregation |
| Double-counted investments | Canonical account ownership mapping |
| Supply-chain compromise | Lock files, pinned images/actions, audits |
| Accidental remote exposure | Localhost binding; no initial self-hosting |

## Prohibited capabilities

- initiating transfers or payments;
- placing or recommending automatic trades;
- accepting arbitrary model-generated SQL;
- giving an LLM database credentials;
- public ingress without a new security decision.

This document will be updated when each new boundary is implemented.
