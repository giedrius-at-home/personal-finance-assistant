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
| Oversized or malformed signed webhook | 1 MiB limit; authenticate raw bytes before strict parsing |
| Duplicate imports | Immutable source identifiers and idempotent upserts |
| Replay after gateway restart | Durable unique webhook delivery IDs |
| Conflicting or over-precise currency data | Explicit minor units; reject conflicts and implicit rounding |
| LLM invents or mutates data | Schema validation, vocabulary, review gates |
| Prompt injection in merchant text | Treat descriptions as data, not instructions |
| Notification storm | Stateful deduplication, cooldown, daily aggregation |
| Concurrent or post-sleep scheduler replay | Durable due-slot lease; only latest missed slot runs |
| AppleScript injection from financial text | Fixed script; bounded title/body passed only as argv |
| Alert cannot be explained from a hash | Canonical aggregate evidence retained with policy version |
| Notification content retained in audit | Persist outcome metadata only; message stays transient |
| Double-counted investments | Canonical account ownership mapping |
| Derived state becomes a hidden ledger | Read-only upstream APIs; projections retain authoritative lineage |
| Audit deletion or accidental schema rollback | Separate projection delete/privacy purge; populated rollback refused |
| Supply-chain compromise | Lock files, pinned images/actions, audits |
| Accidental remote exposure | Localhost binding; no initial self-hosting |
| DNS rebinding or forged browser host | Loopback listener plus strict Host allowlist |
| Cross-site review/settings mutation | Same-origin POST, process CSRF cookie/token, strict SameSite |
| Browser caches financial pages | `Cache-Control: no-store` on every response |
| Third-party content leaks viewing behaviour | Self-only CSP; no telemetry, scripts, fonts, or remote assets |
| Oversized or malformed review form | URL-encoded-only boundary, 8 KiB and field-count limits |
| Stale policy confirmation | Content-bound preview token and optimistic current-hash check |
| Duplicate or stale net-worth valuation | Canonical owner counted once; mixed/stale total withheld |
| Database reached from host or peer service | No published DB ports; separate internal networks |
| Weak or reused local secrets | Cryptographic one-time generation; owner-only `.env` |
| Over-privileged or expired upstream credential | GET-only client surface, synthetic contract identity, owner-only storage, explicit rotation |
| Credential or financial body leaked by client error | Typed body-free errors; no redirect following or request/response logging |
| Corrupt or substituted backup | SHA-256 manifest verified before restore |
| Accidental destructive restore | Explicit backup path and confirmation flag |
| Backup disclosure | Git exclusion, owner-only files; encrypted off-device copy required |

## Prohibited capabilities

- initiating transfers or payments;
- placing or recommending automatic trades;
- accepting arbitrary model-generated SQL;
- giving an LLM database credentials;
- public ingress without a new security decision.

This document will be updated when each new boundary is implemented.
