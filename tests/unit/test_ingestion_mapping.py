from datetime import UTC, datetime
from decimal import Decimal

import pytest

from finance_assistant.domain.canonical import TransactionKind
from finance_assistant.domain.models import Currency
from finance_assistant.ingestion.mapping import (
    AccountLink,
    CurrencyRegistry,
    MappingError,
    build_canonical_accounts,
    build_canonical_transactions,
)
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyCurrency,
    FireflyTransaction,
)
from finance_assistant.integrations.ghostfolio import GhostfolioAccount

NOW = datetime(2026, 1, 2, tzinfo=UTC)


def firefly_currency(code: str = "EUR", places: int = 2) -> FireflyCurrency:
    return FireflyCurrency.model_validate(
        {
            "type": "currencies",
            "id": code,
            "attributes": {
                "code": code,
                "name": f"Synthetic {code}",
                "symbol": code,
                "decimal_places": places,
            },
        }
    )


def firefly_account(
    account_id: str, balance: str = "125.50", currency: str = "EUR"
) -> FireflyAccount:
    return FireflyAccount.model_validate(
        {
            "type": "accounts",
            "id": account_id,
            "attributes": {
                "name": f"Synthetic account {account_id}",
                "type": "asset",
                "currency_code": currency,
                "currency_decimal_places": 2,
                "current_balance": balance,
                "updated_at": NOW.isoformat(),
            },
        }
    )


def ghostfolio_account(
    account_id: str, balance: str = "125.50", currency: str | None = "EUR"
) -> GhostfolioAccount:
    return GhostfolioAccount.model_validate(
        {
            "id": account_id,
            "name": f"Synthetic brokerage {account_id}",
            "currency": currency,
            "balance": balance,
            "value": "200",
            "valueInBaseCurrency": "200",
            "createdAt": NOW.isoformat(),
            "updatedAt": NOW.isoformat(),
        }
    )


def transaction(
    kind: str, source: str | None, destination: str | None
) -> FireflyTransaction:
    return FireflyTransaction.model_validate(
        {
            "type": "transactions",
            "id": f"group-{kind}",
            "attributes": {
                "updated_at": NOW.isoformat(),
                "transactions": [
                    {
                        "transaction_journal_id": f"journal-{kind}",
                        "type": kind,
                        "date": NOW.isoformat(),
                        "currency_code": "EUR",
                        "currency_decimal_places": 2,
                        "amount": "10.25",
                        "description": f"Synthetic {kind}",
                        "source_id": source,
                        "destination_id": destination,
                    }
                ],
            },
        }
    )


def test_currency_registry_rejects_conflicting_or_unknown_metadata() -> None:
    registry = CurrencyRegistry.from_firefly((firefly_currency(),))

    assert registry.require("eur") == Currency(code="EUR", minor_unit=2)
    with pytest.raises(MappingError, match="conflicting precision"):
        registry.add(Currency(code="EUR", minor_unit=3))
    with pytest.raises(MappingError, match="unknown currency"):
        registry.require("USD")


def test_explicit_link_has_one_owner_and_prevents_double_counting() -> None:
    records = build_canonical_accounts(
        firefly_accounts=(firefly_account("cash"),),
        ghostfolio_accounts=(ghostfolio_account("broker"),),
        currencies=CurrencyRegistry.from_firefly((firefly_currency(),)),
        links=(AccountLink("cash", "broker"),),
    )

    assert len(records) == 1
    assert records[0].account_id == "firefly:cash"
    assert records[0].balance.amount == Decimal("125.50")
    assert records[0].linked_sources[0].external_id == "broker"


def test_ambiguous_links_and_currency_metadata_fail_closed() -> None:
    registry = CurrencyRegistry.from_firefly((firefly_currency(),))
    firefly_accounts = (firefly_account("cash"),)
    ghostfolio_accounts = (
        ghostfolio_account("broker-1"),
        ghostfolio_account("broker-2"),
    )
    with pytest.raises(MappingError, match="ambiguous"):
        build_canonical_accounts(
            firefly_accounts=firefly_accounts,
            ghostfolio_accounts=ghostfolio_accounts,
            currencies=registry,
            links=(
                AccountLink("cash", "broker-1"),
                AccountLink("cash", "broker-2"),
            ),
        )
    with pytest.raises(MappingError, match="no currency"):
        build_canonical_accounts(
            firefly_accounts=(),
            ghostfolio_accounts=(ghostfolio_account("broker", currency=None),),
            currencies=registry,
        )


def test_transactions_map_all_directions_including_transfer() -> None:
    registry = CurrencyRegistry.from_firefly((firefly_currency(),))
    accounts = build_canonical_accounts(
        firefly_accounts=(firefly_account("asset-1"), firefly_account("asset-2")),
        ghostfolio_accounts=(),
        currencies=registry,
    )
    records = build_canonical_transactions(
        (
            transaction("Withdrawal", "asset-1", "expense-1"),
            transaction("Deposit", "revenue-1", "asset-1"),
            transaction("Transfer", "asset-1", "asset-2"),
        ),
        currencies=registry,
        accounts=accounts,
    )

    assert [record.kind for record in records] == [
        TransactionKind.OUTFLOW,
        TransactionKind.INFLOW,
        TransactionKind.TRANSFER,
    ]
    assert records[2].counterparty_account_id == "firefly:asset-2"


def test_malformed_transaction_ownership_and_duplicates_fail_closed() -> None:
    registry = CurrencyRegistry.from_firefly((firefly_currency(),))
    accounts = build_canonical_accounts(
        firefly_accounts=(firefly_account("asset-1"),),
        ghostfolio_accounts=(),
        currencies=registry,
    )
    unresolved = transaction("Transfer", "asset-1", "missing")
    with pytest.raises(MappingError, match="counterparty ownership"):
        build_canonical_transactions(
            (unresolved,), currencies=registry, accounts=accounts
        )
    repeated = transaction("Withdrawal", "asset-1", "expense")
    with pytest.raises(MappingError, match="duplicate Firefly transaction"):
        build_canonical_transactions(
            (repeated, repeated), currencies=registry, accounts=accounts
        )
