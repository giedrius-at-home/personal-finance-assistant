# ADR 0005: Explicit currency precision and canonical source lineage

- Status: Accepted when merged
- Date: 2026-09-01

## Context

The first domain slice used Python `Decimal` but represented a currency with
only a three-letter string. That allowed arbitrary fractional precision and
could not distinguish a legitimate higher-precision currency from malformed
cash input. A fixed two-decimal rule would also be incorrect: ISO 4217 includes
different minor units, and Firefly III exposes a configurable
`decimal_places` value for transaction currencies.

Ghostfolio separately handles currency, asset quantity, unit price, and exchange
rate values. Treating all of those as cash would either lose investment
precision or falsely permit over-precise ledger amounts.

Canonical records also need stable identity across upstream updates. Firefly and
Ghostfolio may both represent the cash boundary around one investment account,
so a mapping must identify exactly one authoritative source rather than summing
both representations.

## Decision

Use `Decimal` for every financial calculation and reject binary `float` input at
trusted Python boundaries. Decimal JSON inputs are strings or integers.

Represent a currency as:

- an uppercase three-letter ASCII code; and
- an explicit `minor_unit` from zero through eighteen decimal places.

Adapters obtain this metadata from their pinned upstream contract and must fail
closed if the same currency code is observed with conflicting precision. A cash
`Money` amount may contain no non-zero fractional digits beyond its currency's
minor unit. Trailing zeroes are harmless, but the domain never rounds excess
precision implicitly. Explicit rounding belongs to a named presentation or
settlement boundary with a documented mode.

Asset quantities, unit prices, exchange rates, percentages, forecasts, and other
derived analytics are not cash minor units. They retain bounded decimal
precision in purpose-specific contracts introduced with their consuming
features.

Give each upstream record a `SourceReference` containing an immutable
`(system, entity, external_id)` identity plus optional mutable revision evidence.
A canonical account has one concrete authoritative account reference and zero
or more distinct linked account references. A canonical transaction has one
source identity, an aware timestamp, a positive cash amount, and an explicit
inflow, outflow, or transfer direction. Transfers name a distinct counterparty
canonical account.

## Consequences

- The pre-release HTTP contract now includes `{code, minor_unit}` wherever it
  previously emitted a bare currency code.
- API adapters must load and validate currency metadata before constructing
  canonical money values.
- JPY-style zero-decimal, ISO higher-precision, and Firefly custom currencies
  can be represented without a hard-coded two-decimal assumption.
- Investment quantity and price precision cannot accidentally leak into cash
  arithmetic.
- Source updates change revision evidence without changing the idempotency key.
- Explicit account ownership provides the basis for preventing Firefly and
  Ghostfolio double counting.
- Persistence must use exact decimal or lossless textual representations, never
  binary floating point.

## Alternatives rejected

- **Always use two decimal places:** invalid for zero- and higher-minor-unit
  currencies and custom upstream definitions.
- **Store every cash value as integer minor units:** exact for a known currency
  definition, but awkward at the API boundary and unsafe when upstream precision
  metadata changes or conflicts. It may still be used as a persistence detail
  after validating the explicit currency definition.
- **Allow arbitrary Decimal scale:** exact but fails to detect malformed cash
  input and conflates cash with quantities and rates.
- **Vendor a static ISO currency table:** adds update responsibility and does
  not cover upstream custom currencies. ISO metadata remains a validation source,
  not the sole runtime registry.

## References

- [Python decimal arithmetic](https://docs.python.org/3.14/library/decimal.html)
- [ISO 4217 maintenance agency data](https://www.six-group.com/en/products-services/financial-information/data-standards.html)
- [Firefly III 6.6.6 transaction-currency definitions](https://github.com/firefly-iii/firefly-iii/blob/v6.6.6/database/seeders/TransactionCurrencySeeder.php)
- [Ghostfolio 3.65.0 activity import contract](https://github.com/ghostfolio/ghostfolio/blob/3.65.0/apps/api/src/app/import/import.service.ts)
