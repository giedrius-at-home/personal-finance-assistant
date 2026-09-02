"""Application service joining read-only sources to derived canonical state."""

from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime
from typing import Protocol

from pydantic import Field

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
)
from finance_assistant.domain.models import StrictModel
from finance_assistant.ingestion.mapping import (
    AccountLink,
    CurrencyRegistry,
    build_canonical_accounts,
    build_canonical_transactions,
)
from finance_assistant.ingestion.reconciliation import (
    BalanceObservation,
    ReconciliationReport,
    reconcile_accounts,
)
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyCurrency,
    FireflyTransaction,
)
from finance_assistant.integrations.ghostfolio import GhostfolioAccount


class FireflyReader(Protocol):
    async def list_currencies(self) -> tuple[FireflyCurrency, ...]: ...

    async def list_accounts(self) -> tuple[FireflyAccount, ...]: ...

    async def list_transactions(self) -> tuple[FireflyTransaction, ...]: ...


class GhostfolioReader(Protocol):
    async def list_accounts(self) -> tuple[GhostfolioAccount, ...]: ...


class DerivedState(Protocol):
    def upsert_many(
        self,
        records: tuple[CanonicalAccount | CanonicalTransaction, ...],
        *,
        observed_at: datetime,
    ) -> int: ...

    def list_accounts(self) -> tuple[CanonicalAccount, ...]: ...


class IngestionResult(StrictModel):
    accounts_seen: int = Field(ge=0)
    transactions_seen: int = Field(ge=0)
    projections_changed: int = Field(ge=0)
    reconciliation: ReconciliationReport


class IngestionService:
    """Run one bounded, deterministic snapshot ingestion and reconciliation."""

    def __init__(
        self,
        *,
        firefly: FireflyReader,
        ghostfolio: GhostfolioReader,
        state: DerivedState,
        links: tuple[AccountLink, ...] = (),
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        self._firefly = firefly
        self._ghostfolio = ghostfolio
        self._state = state
        self._links = links
        self._clock = clock or (lambda: datetime.now(UTC))

    async def ingest(self) -> IngestionResult:
        """Validate the full snapshot before changing any derived projection."""

        currencies = CurrencyRegistry.from_firefly(
            await self._firefly.list_currencies()
        )
        accounts = build_canonical_accounts(
            firefly_accounts=await self._firefly.list_accounts(),
            ghostfolio_accounts=await self._ghostfolio.list_accounts(),
            currencies=currencies,
            links=self._links,
        )
        transactions = build_canonical_transactions(
            await self._firefly.list_transactions(),
            currencies=currencies,
            accounts=accounts,
        )
        observed_at = self._clock()
        changed = self._state.upsert_many(
            (*accounts, *transactions), observed_at=observed_at
        )
        observations = tuple(
            BalanceObservation(
                source=account.canonical_owner,
                balance=account.balance,
            )
            for account in accounts
        )
        report = reconcile_accounts(self._state.list_accounts(), observations)
        return IngestionResult(
            accounts_seen=len(accounts),
            transactions_seen=len(transactions),
            projections_changed=changed,
            reconciliation=report,
        )
