# ADR 0020: Recorded spending time trends

Status: Accepted

## Context

The owner wants category trends, cumulative daily comparisons and weekday patterns.
The existing canonical timestamp contains Firefly's date, without reliable evidence
that it is an actual purchase time. The CSV resync parser retains a booking date,
not purchase-time precision. Bank history coverage is not verified.

## Decision

- Add a read-only six-calendar-month report ending at the current instant in the
  owner's configured timezone. Use only confirmed Firefly outflows and refunds.
  Exclude transfers, inflows, other upstream sources and future instants.
- Keep exact Decimal totals and separate full currency identities. Apply the same
  owner-reviewed category-to-group snapshot to monthly and historical views.
  No mapping, source record, account or Firefly category is changed by analytics.
- A currency-month with no expense/refund evidence has a null total, not zero.
  A recorded zero net total remains zero. A group's zero within an evidenced
  currency-month means no net *recorded* spending, not confirmed bank coverage.
- Draw gaps for missing periods. Mark the current month as partial even on its
  final calendar day. Do not infer a reduction by comparing it to full months.
- Daily lines show cumulative recorded spending by day of month, ending today.
  Prior-month evidence is limited to the comparable elapsed window; days beyond
  a shorter prior month are null. Withhold either line without evidence in its
  corresponding window. Zero/flat portions mean no net recorded change, not
  verified no-spend days. Refunds can lower the line on their recorded dates.
- Weekday and weekday/weekend summaries describe recorded dates, not necessarily
  purchase dates. Show gross purchases, net spending, purchase counts and active
  purchase dates. Average gross purchases per active purchase date uses only dates
  with at least one purchase, not assumed missing/zero calendar days. This is a
  conditional statistic, not an allowance or a claim about personal habits.
- Withhold time-of-day patterns entirely until timestamp precision/provenance is
  reliable. Non-midnight values alone do not prove genuine purchase times.
- Use server-rendered, dependency-free SVG with exact-value table alternatives.
  Only plot coordinates are rounded; financial totals are not. Existing progressive
  updates preserve expanded details. No extra browser storage or external calls.

## Consequences and limits

No database migration or new configuration is required. Existing Firefly categories
and reviewed mappings remain the source for grouping; ungrouped expenses stay visible.
The six-month view includes five historical months plus the current partial month.
Missing imports can distort all patterns, including months that contain some records.
The report intentionally does not forecast spending, count upcoming salary, claim
statement completeness, or offer custom date ranges and hourly analysis.

Synthetic tests cover refunds, absent versus zero evidence, source/currency isolation,
calendar and timezone boundaries, leap years, unequal month lengths, conditional
averages, deterministic ordering, chart gaps/negative values and read-only rendering.
