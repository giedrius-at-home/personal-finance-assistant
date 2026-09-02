"""Exact, explainable reconciliation of canonical account balances."""

from __future__ import annotations

from collections.abc import Iterable
from enum import StrEnum

from pydantic import Field

from finance_assistant.domain.canonical import CanonicalAccount, SourceReference
from finance_assistant.domain.models import Money, StrictModel


class ReconciliationStatus(StrEnum):
    MATCHED = "matched"
    MISSING = "missing"
    UNEXPECTED = "unexpected"
    BALANCE_MISMATCH = "balance_mismatch"
    CURRENCY_MISMATCH = "currency_mismatch"


class BalanceObservation(StrictModel):
    """Expected balance from one authoritative upstream account."""

    source: SourceReference
    balance: Money


class ReconciliationEntry(StrictModel):
    account_id: str | None = None
    source: SourceReference
    status: ReconciliationStatus
    expected: Money | None = None
    actual: Money | None = None
    explanation: str = Field(min_length=1, max_length=512)


class ReconciliationReport(StrictModel):
    matched: bool
    entries: tuple[ReconciliationEntry, ...]


def reconcile_accounts(
    accounts: Iterable[CanonicalAccount],
    observations: Iterable[BalanceObservation],
) -> ReconciliationReport:
    """Compare canonical balances to unique authoritative source observations."""

    account_records = tuple(accounts)
    canonical = {
        account.canonical_owner.identity: account for account in account_records
    }
    if len(canonical) != len(account_records):
        raise ValueError("canonical accounts contain duplicate ownership")
    expected = {}
    for observation in observations:
        if observation.source.identity in expected:
            raise ValueError("observations contain duplicate ownership")
        expected[observation.source.identity] = observation

    entries: list[ReconciliationEntry] = []
    for identity, observation in expected.items():
        account = canonical.pop(identity, None)
        if account is None:
            entries.append(
                ReconciliationEntry(
                    source=observation.source,
                    status=ReconciliationStatus.MISSING,
                    expected=observation.balance,
                    explanation=(
                        "authoritative source account is missing from canonical state"
                    ),
                )
            )
        elif account.currency != observation.balance.currency:
            entries.append(
                ReconciliationEntry(
                    account_id=account.account_id,
                    source=observation.source,
                    status=ReconciliationStatus.CURRENCY_MISMATCH,
                    expected=observation.balance,
                    actual=account.balance,
                    explanation="currency identity or precision differs",
                )
            )
        elif account.balance.amount != observation.balance.amount:
            entries.append(
                ReconciliationEntry(
                    account_id=account.account_id,
                    source=observation.source,
                    status=ReconciliationStatus.BALANCE_MISMATCH,
                    expected=observation.balance,
                    actual=account.balance,
                    explanation="exact decimal balances differ",
                )
            )
        else:
            entries.append(
                ReconciliationEntry(
                    account_id=account.account_id,
                    source=observation.source,
                    status=ReconciliationStatus.MATCHED,
                    expected=observation.balance,
                    actual=account.balance,
                    explanation=(
                        "canonical balance exactly matches its authoritative source"
                    ),
                )
            )
    for account in canonical.values():
        entries.append(
            ReconciliationEntry(
                account_id=account.account_id,
                source=account.canonical_owner,
                status=ReconciliationStatus.UNEXPECTED,
                actual=account.balance,
                explanation="canonical account has no authoritative source observation",
            )
        )
    return ReconciliationReport(
        matched=all(entry.status is ReconciliationStatus.MATCHED for entry in entries),
        entries=tuple(entries),
    )
