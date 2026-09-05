from datetime import UTC, datetime, timedelta

import pytest

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.domain.reporting import (
    CategoryAggregate,
    net_worth_report,
    taxonomy_recommendations,
    weekly_report,
)

NOW = datetime(2026, 9, 2, 12, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)
USD = Currency(code="USD", minor_unit=2)


def source(external_id: str, *, updated_at: datetime = NOW) -> SourceReference:
    return SourceReference(
        system=SourceSystem.FIREFLY_III,
        entity=SourceEntity.TRANSACTION,
        external_id=external_id,
        source_updated_at=updated_at,
    )


def transaction(
    external_id: str, kind: TransactionKind, amount: str, currency: Currency = EUR
) -> CanonicalTransaction:
    return CanonicalTransaction(
        transaction_id=external_id,
        source=source(external_id),
        account_id="cash",
        counterparty_account_id="savings" if kind is TransactionKind.TRANSFER else None,
        occurred_at=NOW,
        kind=kind,
        amount=Money(amount=amount, currency=currency),
    )


def test_weekly_report_separates_currency_transfers_and_refunds() -> None:
    report = weekly_report(
        (
            transaction("income", TransactionKind.INFLOW, "100"),
            transaction("spend", TransactionKind.OUTFLOW, "30"),
            transaction("refund", TransactionKind.REFUND, "5"),
            transaction("transfer", TransactionKind.TRANSFER, "20"),
            transaction("usd", TransactionKind.OUTFLOW, "7", USD),
        ),
        as_of=NOW,
        timezone="Europe/Amsterdam",
        safe_to_spend=(
            Money(amount="60", currency=EUR),
            Money(amount="40", currency=EUR),
        ),
    )

    assert [(item.currency.code, item.net_cash_flow) for item in report.currencies] == [
        ("EUR", 75),
        ("USD", -7),
    ]
    assert report.currencies[0].transfer_count == 1
    assert report.safe_to_spend[0].amount == 40
    assert report == weekly_report(
        tuple(
            reversed(
                (
                    transaction("income", TransactionKind.INFLOW, "100"),
                    transaction("spend", TransactionKind.OUTFLOW, "30"),
                    transaction("refund", TransactionKind.REFUND, "5"),
                    transaction("transfer", TransactionKind.TRANSFER, "20"),
                    transaction("usd", TransactionKind.OUTFLOW, "7", USD),
                )
            )
        ),
        as_of=NOW,
        timezone="Europe/Amsterdam",
        safe_to_spend=(
            Money(amount="40", currency=EUR),
            Money(amount="60", currency=EUR),
        ),
    )


def test_net_worth_counts_mirrors_once_and_fails_closed_when_stale() -> None:
    account = CanonicalAccount(
        account_id="portfolio",
        name="Portfolio",
        balance=Money(amount="1000", currency=EUR),
        canonical_owner=source("owner", updated_at=NOW - timedelta(days=3)).model_copy(
            update={"entity": SourceEntity.ACCOUNT}
        ),
        linked_sources=(
            SourceReference(
                system=SourceSystem.GHOSTFOLIO,
                entity=SourceEntity.ACCOUNT,
                external_id="mirror",
                source_updated_at=NOW,
            ),
        ),
    )

    report = net_worth_report((account,), as_of=NOW)

    assert report.currency_totals[0].amount == 1000
    assert report.components[0].excluded_mirror_count == 1
    assert report.total is None
    assert report.reasons == ("stale valuations prevent a combined total",)


def test_net_worth_honors_source_exclusion() -> None:
    excluded = CanonicalAccount(
        account_id="excluded",
        name="Excluded synthetic account",
        balance=Money(amount="999", currency=EUR),
        canonical_owner=source("excluded").model_copy(
            update={"entity": SourceEntity.ACCOUNT}
        ),
        include_net_worth=False,
    )

    report = net_worth_report((excluded,), as_of=NOW)

    assert report.components == ()
    assert report.currency_totals == ()
    assert report.total is None
    assert report.reasons == ("source settings exclude 1 account from net worth",)


def test_taxonomy_signals_are_deterministic_and_reject_duplicate_input() -> None:
    aggregates = (
        CategoryAggregate(category="Dining out", transaction_count=20, amount="200"),
        CategoryAggregate(category="Dining-out", transaction_count=1, amount="10"),
    )

    signals = taxonomy_recommendations(aggregates)

    assert {item.signal.value for item in signals} == {
        "sparse",
        "duplicate_like",
        "overloaded",
    }
    assert signals == taxonomy_recommendations(tuple(reversed(aggregates)))
    with pytest.raises(ValueError, match="unique"):
        taxonomy_recommendations((aggregates[0], aggregates[0]))
