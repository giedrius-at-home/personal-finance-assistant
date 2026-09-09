# ADR 0013: Keep Firefly authoritative for reviewed categories

- Status: Accepted
- Date: 2026-09-06

## Context

The local Qwen categoriser can produce bounded suggestions, and the owner
dashboard already records approvals and corrections. Those decisions previously
remained only in assistant-owned SQLite state. Firefly is the authoritative cash
ledger and has the stronger long-term categorisation and rule-management
surface, so a local-only approval could drift from Firefly and disappear when
derived state is rebuilt.

Model output must not gain a direct write path. A review may also become stale
if the owner edits the transaction in Firefly while the dashboard remains open.
The integration token is sensitive and has broader owner permissions than the
single operation the dashboard needs.

## Decision

Treat Firefly's transaction category as authoritative. Ingest `category_name`
with every Firefly snapshot and prefer it over local proposal state in reports.
Add a separately opt-in adapter that can update only `category_name` for the
exact Firefly transaction journal represented by a pending proposal.

Require a same-origin, CSRF-protected owner action for every approval or
correction. Immediately before the update, fetch the exact transaction and
compare its current category with the proposal's original category. Refuse a
stale preview, an ambiguous split, a non-Firefly source, or an unexpected API
response. Send no amount, account, description, or other editable transaction
fields. Do not apply Firefly rules as part of the update. Keep the feature
disabled by default and never place the token in source control or logs.

## Consequences

Reviewed categories survive rebuilding local state and are visible in Firefly's
native reporting and rule workflows. The dashboard follows Firefly after the
next normal snapshot. Rejections and unreviewed Qwen suggestions remain local
and cannot mutate the ledger.

A Firefly API token remains a high-value owner secret despite the narrow client
surface. The local deployment must keep it in the ignored environment file and
rotate it if exposure is suspected. If Firefly is unavailable or the preview is
stale, the proposal stays pending for explicit review rather than silently
falling back to a local approval.

## Rejected alternatives

- Keep approved categories only in SQLite: derived state would compete with the
  ledger and could drift or be lost on rebuild.
- Let Qwen write categories automatically: confidence is not authorization and
  model mistakes would directly mutate the ledger.
- Send the full transaction back to Firefly: this expands the mutation surface
  and risks changing financial facts unrelated to categorisation.
- Run Firefly rules on every approval: rule side effects are wider than the
  single reviewed decision and belong in Firefly's own explicit rule workflow.
