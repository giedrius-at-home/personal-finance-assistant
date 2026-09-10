"""Owner-reviewed group labels over unchanged, confirmed Firefly categories."""

from __future__ import annotations

from calendar import monthrange
from collections import defaultdict
from datetime import date, datetime, timedelta
from decimal import Decimal
from enum import StrEnum
from zoneinfo import ZoneInfo

from pydantic import Field

from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, StrictModel


class SpendingGroup(StrEnum):
    SUBSCRIPTIONS = "Subscriptions & recurring"
    GROCERIES = "Groceries"
    DINING = "Eating out"
    HOME = "Home & household"
    HOUSING = "Housing & utilities"
    TRANSPORT = "Transport"
    TRAVEL = "Travel"
    ENTERTAINMENT = "Entertainment"
    SHOPPING = "Shopping"
    HEALTH = "Health & personal care"
    EDUCATION = "Education & work"
    FEES = "Fees & taxes"
    OTHER = "Other"


class GroupMapping(StrictModel):
    category: str = Field(min_length=1, max_length=128)
    group: SpendingGroup | None = None
    revision: int = Field(default=0, ge=0)


class StoreSpending(StrictModel):
    category: str | None
    spending: Decimal
    gross: Decimal
    refunds: Decimal
    transaction_count: int
    previous_spending: Decimal | None


class GroupSpending(StrictModel):
    group: SpendingGroup | None
    spending: Decimal
    share_percent: Decimal
    stores: tuple[StoreSpending, ...]


class CurrencySpending(StrictModel):
    currency: Currency
    spending: Decimal
    gross: Decimal
    refunds: Decimal
    comparison_spending: Decimal
    previous_spending: Decimal | None
    transfer_count: int
    groups: tuple[GroupSpending, ...]


class MonthlySpending(StrictModel):
    start: date
    through: date
    comparison_through: date
    previous_start: date
    previous_through: date
    currencies: tuple[CurrencySpending, ...]
    mappings: tuple[GroupMapping, ...]


def expense_categories(transactions: tuple[CanonicalTransaction, ...]) -> set[str]:
    return {
        t.category
        for t in transactions
        if t.category is not None
        and t.source.system is SourceSystem.FIREFLY_III
        and t.kind in (TransactionKind.OUTFLOW, TransactionKind.REFUND)
    }


def _net(rows: list[CanonicalTransaction]) -> Decimal:
    return sum(
        (
            t.amount.amount if t.kind is TransactionKind.OUTFLOW else -t.amount.amount
            for t in rows
        ),
        Decimal(0),
    )


def monthly_spending(
    transactions: tuple[CanonicalTransaction, ...],
    mappings: tuple[GroupMapping, ...],
    *,
    as_of: datetime,
    timezone: str,
) -> MonthlySpending:
    if as_of.tzinfo is None:
        raise ValueError("as_of must be timezone-aware")
    zone = ZoneInfo(timezone)
    today = as_of.astimezone(zone).date()
    start = today.replace(day=1)
    previous_start = (start - timedelta(days=1)).replace(day=1)
    day = min(today.day, monthrange(previous_start.year, previous_start.month)[1])
    comparison_through = today.replace(day=day)
    previous_through = previous_start.replace(day=day)
    reviewed = {m.category: m for m in mappings}
    source_rows = tuple(
        t
        for t in transactions
        if t.source.system is SourceSystem.FIREFLY_III and t.occurred_at <= as_of
    )
    current: dict[Currency, list[CanonicalTransaction]] = defaultdict(list)
    previous: dict[Currency, list[CanonicalTransaction]] = defaultdict(list)
    transfers: dict[Currency, int] = defaultdict(int)
    for t in source_rows:
        on = t.occurred_at.astimezone(zone).date()
        if start <= on <= today and t.kind is TransactionKind.TRANSFER:
            transfers[t.amount.currency] += 1
        if t.kind not in (TransactionKind.OUTFLOW, TransactionKind.REFUND):
            continue
        if start <= on <= today:
            current[t.amount.currency].append(t)
        elif previous_start <= on <= previous_through:
            previous[t.amount.currency].append(t)
    currencies = []
    for currency in sorted(
        current.keys() | previous.keys() | transfers.keys(),
        key=lambda c: (c.code, c.minor_unit),
    ):
        rows, prior = current[currency], previous[currency]
        gross = sum(
            (t.amount.amount for t in rows if t.kind is TransactionKind.OUTFLOW),
            Decimal(0),
        )
        refunds = gross - _net(rows)
        stores: dict[str | None, list[CanonicalTransaction]] = defaultdict(list)
        prior_stores: dict[str | None, list[CanonicalTransaction]] = defaultdict(list)
        for t in rows:
            stores[t.category].append(t)
        for t in prior:
            prior_stores[t.category].append(t)
        groups: dict[SpendingGroup | None, list[StoreSpending]] = defaultdict(list)
        for category in stores.keys() | prior_stores.keys():
            items = stores[category]
            store_gross = sum(
                (t.amount.amount for t in items if t.kind is TransactionKind.OUTFLOW),
                Decimal(0),
            )
            group = reviewed[category].group if category in reviewed else None
            groups[group].append(
                StoreSpending(
                    category=category,
                    spending=_net(items),
                    gross=store_gross,
                    refunds=store_gross - _net(items),
                    transaction_count=len(items),
                    previous_spending=_net(prior_stores[category]) if prior else None,
                )
            )
        group_rows = tuple(
            sorted(
                (
                    GroupSpending(
                        group=group,
                        spending=sum((s.spending for s in items), Decimal(0)),
                        share_percent=(
                            sum((s.gross for s in items), Decimal(0)) * 100 / gross
                            if gross
                            else Decimal(0)
                        ),
                        stores=tuple(
                            sorted(items, key=lambda s: (-s.gross, s.category or ""))
                        ),
                    )
                    for group, items in groups.items()
                ),
                key=lambda g: (-g.share_percent, g.group or ""),
            )
        )
        currencies.append(
            CurrencySpending(
                currency=currency,
                spending=_net(rows),
                gross=gross,
                refunds=refunds,
                comparison_spending=_net(
                    [
                        t
                        for t in rows
                        if t.occurred_at.astimezone(zone).date() <= comparison_through
                    ]
                ),
                previous_spending=_net(prior) if prior else None,
                transfer_count=transfers[currency],
                groups=group_rows,
            )
        )
    return MonthlySpending(
        start=start,
        through=today,
        comparison_through=comparison_through,
        previous_start=previous_start,
        previous_through=previous_through,
        currencies=tuple(currencies),
        mappings=tuple(
            reviewed.get(category, GroupMapping(category=category))
            for category in sorted(
                expense_categories(source_rows), key=lambda c: (c.casefold(), c)
            )
        ),
    )
