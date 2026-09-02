from datetime import UTC, datetime

import pytest

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.ingestion.reconciliation import (
    BalanceObservation,
    ReconciliationStatus,
    reconcile_accounts,
)

NOW = datetime(2026, 1, 2, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def source(external_id: str) -> SourceReference:
    return SourceReference(
        system=SourceSystem.FIREFLY_III,
        entity=SourceEntity.ACCOUNT,
        external_id=external_id,
        source_revision="synthetic-r1",
        source_updated_at=NOW,
    )


def account(external_id: str, amount: str) -> CanonicalAccount:
    return CanonicalAccount(
        account_id=f"firefly:{external_id}",
        name=f"Synthetic {external_id}",
        balance=Money(amount=amount, currency=EUR),
        canonical_owner=source(external_id),
    )


def observation(external_id: str, amount: str) -> BalanceObservation:
    return BalanceObservation(
        source=source(external_id),
        balance=Money(amount=amount, currency=EUR),
    )


def test_exact_balances_reconcile_without_rounding() -> None:
    report = reconcile_accounts(
        (account("one", "10.01"), account("two", "20.02")),
        (observation("one", "10.01"), observation("two", "20.02")),
    )

    assert report.matched
    assert {entry.status for entry in report.entries} == {ReconciliationStatus.MATCHED}


def test_report_explains_mismatch_missing_and_unexpected_accounts() -> None:
    report = reconcile_accounts(
        (account("different", "10.00"), account("unexpected", "1.00")),
        (observation("different", "10.01"), observation("missing", "5.00")),
    )

    assert not report.matched
    assert {entry.status for entry in report.entries} == {
        ReconciliationStatus.BALANCE_MISMATCH,
        ReconciliationStatus.MISSING,
        ReconciliationStatus.UNEXPECTED,
    }
    assert all(entry.explanation for entry in report.entries)


def test_duplicate_ownership_fails_closed() -> None:
    with pytest.raises(ValueError, match="duplicate ownership"):
        reconcile_accounts(
            (account("one", "1"), account("one", "1")),
            (observation("one", "1"),),
        )
