from datetime import UTC, date, datetime

import pytest

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.cash_overview import cash_overview, payday_window
from finance_assistant.domain.models import Currency, Money

EUR = Currency(code="EUR", minor_unit=2)
NOW = datetime(2026, 9, 9, 12, tzinfo=UTC)


def account(key: str) -> CanonicalAccount:
    return CanonicalAccount(
        account_id=key,
        name=f"Synthetic {key}",
        balance=Money(amount="100", currency=EUR),
        canonical_owner=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.ACCOUNT,
            external_id=key,
        ),
    )


def row(
    key: str,
    kind: TransactionKind,
    amount: str,
    *,
    day: int = 9,
    category: str | None = None,
    currency: Currency = EUR,
) -> CanonicalTransaction:
    return CanonicalTransaction(
        transaction_id=key,
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=key,
        ),
        account_id="a",
        counterparty_account_id="b" if kind is TransactionKind.TRANSFER else None,
        occurred_at=NOW.replace(day=day),
        kind=kind,
        amount=Money(amount=amount, currency=currency),
        category=category,
        description="Synthetic Owner shop",
    )


@pytest.mark.parametrize(
    ("today", "day", "start", "next_day"),
    [
        (date(2026, 9, 9), 25, date(2026, 8, 25), date(2026, 9, 25)),
        (date(2026, 9, 25), 25, date(2026, 9, 25), date(2026, 10, 25)),
        (date(2026, 3, 1), 31, date(2026, 2, 28), date(2026, 3, 31)),
        (date(2024, 2, 29), 31, date(2024, 2, 29), date(2024, 3, 31)),
        (date(2026, 1, 1), 25, date(2025, 12, 25), date(2026, 1, 25)),
    ],
)
def test_monthly_payday_boundaries(
    today: date, day: int, start: date, next_day: date
) -> None:
    assert payday_window(today, day) == (start, next_day)


def test_health_and_cycle_keep_real_inflows_and_separate_transfers() -> None:
    rows = (
        row("salary", TransactionKind.INFLOW, "1000"),
        row("spend", TransactionKind.OUTFLOW, "30", category="Groceries"),
        row("unknown", TransactionKind.OUTFLOW, "10"),
        row("refund", TransactionKind.REFUND, "5"),
        row("transfer", TransactionKind.TRANSFER, "100"),
        row("future", TransactionKind.OUTFLOW, "999", day=10),
    )
    report = cash_overview(
        (account("a"), account("b")),
        rows,
        as_of=NOW,
        timezone="Europe/Amsterdam",
        payday_day=25,
    )
    assert report.period_start == date(2026, 8, 25)
    assert report.days_until_payday == 16
    assert report.currencies[0].income == 1000
    assert report.currencies[0].spending == 35
    assert report.currencies[0].net_cash_flow == 965
    assert report.currencies[0].transfer_count == 1
    assert report.accounts[0].uncategorised_outflows == 1
    assert report.accounts[0].outflow_count == 2
    assert report.accounts[1].transaction_count == 1
    assert report.accounts[1].last_transaction_on == date(2026, 9, 9)
    assert (
        cash_overview(
            (account("b"), account("a")),
            tuple(reversed(rows)),
            as_of=NOW,
            timezone="Europe/Amsterdam",
            payday_day=25,
        )
        == report
    )


def test_unknown_payday_empty_accounts_and_foreign_currency() -> None:
    empty = cash_overview(
        (account("a"),), (), as_of=NOW, timezone="Europe/Amsterdam", payday_day=None
    )
    assert empty.next_payday is None and empty.currencies == ()
    assert empty.accounts[0].first_transaction_on is None
    usd = Currency(code="USD", minor_unit=2)
    report = cash_overview(
        (account("a"),),
        (
            row("e", TransactionKind.OUTFLOW, "10"),
            row("u", TransactionKind.OUTFLOW, "7", currency=usd),
        ),
        as_of=NOW,
        timezone="Europe/Amsterdam",
        payday_day=1,
    )
    assert [(c.currency.code, c.spending) for c in report.currencies] == [
        ("EUR", 10),
        ("USD", 7),
    ]


def test_timezone_and_invalid_payday() -> None:
    with pytest.raises(ValueError):
        payday_window(date(2026, 9, 1), 0)
    with pytest.raises(ValueError):
        cash_overview(
            (),
            (),
            as_of=datetime(2026, 9, 1),
            timezone="Europe/Amsterdam",
            payday_day=1,
        )
    result = cash_overview(
        (),
        (),
        as_of=datetime(2026, 8, 31, 23, tzinfo=UTC),
        timezone="Europe/Amsterdam",
        payday_day=1,
    )
    assert result.period_start == date(2026, 9, 1)
