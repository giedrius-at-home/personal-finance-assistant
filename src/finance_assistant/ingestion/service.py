"""Application service joining read-only sources to derived canonical state."""

from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime
from typing import Protocol

from pydantic import Field

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalSavingsGoal,
    CanonicalTransaction,
    SourceSystem,
)
from finance_assistant.domain.models import StrictModel
from finance_assistant.ingestion.goals import build_savings_goals
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
    FireflyPiggyBank,
    FireflyTransaction,
)
from finance_assistant.integrations.ghostfolio import GhostfolioAccount
from finance_assistant.subscriptions import SubscriptionSnapshot


class FireflyReader(Protocol):
    async def list_currencies(self) -> tuple[FireflyCurrency, ...]: ...

    async def list_accounts(self) -> tuple[FireflyAccount, ...]: ...

    async def list_transactions(self) -> tuple[FireflyTransaction, ...]: ...

    async def list_piggy_banks(self) -> tuple[FireflyPiggyBank, ...]: ...


class GhostfolioReader(Protocol):
    async def list_accounts(self) -> tuple[GhostfolioAccount, ...]: ...


class DerivedState(Protocol):
    def replace_source_snapshot(
        self,
        records: tuple[CanonicalAccount | CanonicalTransaction, ...],
        goals: tuple[CanonicalSavingsGoal, ...],
        *,
        source_systems: tuple[SourceSystem, ...],
        observed_at: datetime,
        complete_transactions: bool = True,
        subscriptions: SubscriptionSnapshot | None = None,
    ) -> int: ...

    def list_accounts(self) -> tuple[CanonicalAccount, ...]: ...


class IngestionResult(StrictModel):
    accounts_seen: int = Field(ge=0)
    transactions_seen: int = Field(ge=0)
    goals_seen: int = Field(ge=0)
    goals_ineligible: int = Field(ge=0)
    projections_changed: int = Field(ge=0)
    reconciliation: ReconciliationReport


class IngestionService:
    """Run one bounded, deterministic snapshot ingestion and reconciliation."""

    def __init__(
        self,
        *,
        firefly: FireflyReader,
        ghostfolio: GhostfolioReader | None = None,
        state: DerivedState,
        links: tuple[AccountLink, ...] = (),
        clock: Callable[[], datetime] | None = None,
        transactions: tuple[FireflyTransaction, ...] | None = None,
        complete_transactions: bool = True,
        subscriptions: SubscriptionSnapshot | None = None,
    ) -> None:
        self._firefly = firefly
        self._ghostfolio = ghostfolio
        self._state = state
        self._links = links
        self._clock = clock or (lambda: datetime.now(UTC))
        self._transactions = transactions
        self._complete_transactions = complete_transactions
        self._subscriptions = subscriptions

    async def ingest(self) -> IngestionResult:
        """Validate the full snapshot before changing any derived projection."""

        currencies = CurrencyRegistry.from_firefly(
            await self._firefly.list_currencies()
        )
        accounts = build_canonical_accounts(
            firefly_accounts=await self._firefly.list_accounts(),
            ghostfolio_accounts=(
                await self._ghostfolio.list_accounts() if self._ghostfolio else ()
            ),
            currencies=currencies,
            links=self._links,
        )
        transactions = build_canonical_transactions(
            (
                self._transactions
                if self._transactions is not None
                else await self._firefly.list_transactions()
            ),
            currencies=currencies,
            accounts=accounts,
        )
        goal_report = build_savings_goals(
            await self._firefly.list_piggy_banks(), currencies=currencies
        )
        observed_at = self._clock()
        snapshot: tuple[CanonicalAccount | CanonicalTransaction, ...] = (
            *accounts,
            *transactions,
        )
        changed = self._state.replace_source_snapshot(
            snapshot,
            goal_report.goals,
            source_systems=(
                (SourceSystem.FIREFLY_III, SourceSystem.GHOSTFOLIO)
                if self._ghostfolio is not None
                else (SourceSystem.FIREFLY_III,)
            ),
            observed_at=observed_at,
            complete_transactions=self._complete_transactions,
            subscriptions=self._subscriptions,
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
            goals_seen=len(goal_report.goals),
            goals_ineligible=len(goal_report.ineligible),
            projections_changed=changed,
            reconciliation=report,
        )
