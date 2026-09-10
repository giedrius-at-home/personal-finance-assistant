# ADR 0021: Subscription payment evidence

Status: Accepted

## Context

The dashboard reads Firefly bill schedules, but previously retained only payment
dates from the bill response. Firefly transaction splits can carry a native bill ID.
Some owner-maintained transaction categories also use the exact saved subscription
name even where Firefly has no native link. Amount or merchant guessing would create
false matches and could double-count spending.

## Decision

- Preserve the optional Firefly `bill_id` on a canonical transaction as an opaque
  `subscription_id`. Ignore `bill_name`: the separately validated bill snapshot is
  authoritative for display names. Older canonical payloads remain valid with null.
- Build six calendar months of recorded payment evidence from confirmed Firefly
  outflows only, through the current instant in the owner's timezone. Match the bill
  currency exactly. Exclude inflows, transfers, refunds, other sources, future records
  and older records. No source transaction is modified.
- Count a matching native bill ID as direct evidence. For transactions without a
  native bill ID, permit a weaker category-name match only when the trimmed,
  case-folded category equals exactly one active saved subscription name. Duplicate
  subscription names disable this fallback. Do not use amount, date proximity,
  descriptions, accounts, fuzzy matching, rules or a model.
- A transaction satisfying the native link is never counted again as category
  evidence. Recorded evidence is an additional lens over the same transactions and
  remains included once in monthly/category/time-trend totals.
- Keep Firefly's bill `paid_dates` as separate date-only link evidence. Do not retain
  its transaction-group identifiers in the subscription snapshot. Missing evidence
  does not prove a payment is missing; prior evidence does not prove the next payment.
- Add “Subscriptions & recurring” to the reviewed broad-group vocabulary. Exact
  category matches are marked in the mapping editor, but are never automatically
  assigned. The owner or an explicitly reviewed local-model suggestion must save it.
- Expose counts, exact recorded totals and most recent recorded dates only in the
  existing owner-local dashboard/JSON boundary. Do not add them to aggregate MCP
  tools or logs. Preserve currency separation and Decimal arithmetic.

## Consequences and limits

The next full Firefly refresh enriches canonical transactions with native bill IDs
where Firefly supplies them. This is an additive payload change without a schema
migration. Exact category-name evidence relies on owner naming and is intentionally
labelled weaker than a native link. Renamed categories or bills stop matching until
they agree again. Inactive/expired bills are excluded from displayed evidence.

The dashboard does not create Firefly links, alter rules, recategorize transactions,
infer whether a bill is overdue, or subtract scheduled and recorded values together.
Synthetic tests cover native/category precedence, duplicate names, currency/source/
kind/time boundaries, read-only rendering, legacy payloads and private-field removal.
