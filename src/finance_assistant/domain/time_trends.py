"""Read-only calendar trends from confirmed Firefly spending, never predictions."""

from __future__ import annotations

from calendar import monthrange
from collections import defaultdict
from datetime import date, datetime
from decimal import Decimal
from typing import Literal
from zoneinfo import ZoneInfo

from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, StrictModel
from finance_assistant.domain.spending_groups import GroupMapping, SpendingGroup

WEEKDAYS = (
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
)


class MonthSpending(StrictModel):
    start: date
    through: date
    partial_month: bool
    spending: Decimal | None
    record_count: int


class GroupTrend(StrictModel):
    group: SpendingGroup | None
    values: tuple[Decimal | None, ...]


class DailySpending(StrictModel):
    day: int
    current_on: date
    previous_on: date | None
    current: Decimal | None
    previous: Decimal | None


class WeekdaySpending(StrictModel):
    label: str
    spending: Decimal | None
    gross: Decimal
    refunds: Decimal
    purchase_count: int
    purchase_days: int
    average_purchase_day: Decimal | None


class CurrencyTrends(StrictModel):
    currency: Currency
    months: tuple[MonthSpending, ...]
    groups: tuple[GroupTrend, ...]
    daily: tuple[DailySpending, ...]
    weekdays: tuple[WeekdaySpending, ...]
    day_types: tuple[WeekdaySpending, ...]


class TimeTrends(StrictModel):
    start: date
    through: date
    timezone: str
    currencies: tuple[CurrencyTrends, ...]
    time_of_day_available: Literal[False] = False


def _month(start: date, offset: int) -> date:
    ordinal = start.year * 12 + start.month - 1 + offset
    year, month = divmod(ordinal, 12)
    return date(year, month + 1, 1)


def _net(rows: list[CanonicalTransaction]) -> Decimal:
    return sum(
        (
            r.amount.amount if r.kind is TransactionKind.OUTFLOW else -r.amount.amount
            for r in rows
        ),
        Decimal(0),
    )


def _weekday(
    label: str, rows: list[CanonicalTransaction], zone: ZoneInfo
) -> WeekdaySpending:
    purchases = [r for r in rows if r.kind is TransactionKind.OUTFLOW]
    days = len({r.occurred_at.astimezone(zone).date() for r in purchases})
    gross = sum((r.amount.amount for r in purchases), Decimal(0))
    return WeekdaySpending(
        label=label,
        spending=_net(rows) if rows else None,
        gross=gross,
        refunds=gross - _net(rows),
        purchase_count=len(purchases),
        purchase_days=days,
        average_purchase_day=gross / days if days else None,
    )


def _daily(
    current: list[CanonicalTransaction],
    previous: list[CanonicalTransaction],
    *,
    today: date,
    zone: ZoneInfo,
) -> tuple[DailySpending, ...]:
    prior_start = _month(today, -1)
    prior_days = monthrange(prior_start.year, prior_start.month)[1]
    previous = [
        r
        for r in previous
        if r.occurred_at.astimezone(zone).day <= min(today.day, prior_days)
    ]
    totals: list[dict[int, Decimal]] = [defaultdict(Decimal), defaultdict(Decimal)]
    for rows, amounts in zip((current, previous), totals, strict=True):
        for r in rows:
            day = r.occurred_at.astimezone(zone).day
            amounts[day] += (
                r.amount.amount
                if r.kind is TransactionKind.OUTFLOW
                else -r.amount.amount
            )
    current_sum = previous_sum = Decimal(0)
    points = []
    for day in range(1, today.day + 1):
        current_sum += totals[0][day]
        previous_sum += totals[1][day]
        points.append(
            DailySpending(
                day=day,
                current_on=today.replace(day=day),
                previous_on=prior_start.replace(day=day) if day <= prior_days else None,
                current=current_sum if current else None,
                previous=previous_sum if previous and day <= prior_days else None,
            )
        )
    return tuple(points)


def time_trends(
    transactions: tuple[CanonicalTransaction, ...],
    mappings: tuple[GroupMapping, ...],
    *,
    as_of: datetime,
    timezone: str,
) -> TimeTrends:
    if as_of.tzinfo is None:
        raise ValueError("as_of must be timezone-aware")
    zone = ZoneInfo(timezone)
    today = as_of.astimezone(zone).date()
    starts = tuple(_month(today, offset) for offset in range(-5, 1))
    grouped: dict[Currency, list[CanonicalTransaction]] = defaultdict(list)
    reviewed = {m.category: m.group for m in mappings}
    for r in transactions:
        if (
            r.source.system is SourceSystem.FIREFLY_III
            and r.kind in (TransactionKind.OUTFLOW, TransactionKind.REFUND)
            and r.occurred_at <= as_of
            and starts[0] <= r.occurred_at.astimezone(zone).date() <= today
        ):
            grouped[r.amount.currency].append(r)
    results = []
    for currency, rows in sorted(
        grouped.items(), key=lambda pair: (pair[0].code, pair[0].minor_unit)
    ):
        by_month: dict[date, list[CanonicalTransaction]] = defaultdict(list)
        by_group: dict[SpendingGroup | None, dict[date, list[CanonicalTransaction]]] = (
            defaultdict(lambda: defaultdict(list))
        )
        by_weekday: dict[int, list[CanonicalTransaction]] = defaultdict(list)
        for r in rows:
            on = r.occurred_at.astimezone(zone).date()
            start = on.replace(day=1)
            by_month[start].append(r)
            by_group[reviewed.get(r.category) if r.category is not None else None][
                start
            ].append(r)
            by_weekday[on.weekday()].append(r)
        months = tuple(
            MonthSpending(
                start=start,
                through=today
                if start == starts[-1]
                else start.replace(day=monthrange(start.year, start.month)[1]),
                partial_month=start == starts[-1],
                spending=_net(by_month[start]) if by_month[start] else None,
                record_count=len(by_month[start]),
            )
            for start in starts
        )
        groups = tuple(
            GroupTrend(
                group=group,
                values=tuple(
                    _net(history[start]) if by_month[start] else None
                    for start in starts
                ),
            )
            for group, history in sorted(
                by_group.items(), key=lambda pair: pair[0] or ""
            )
        )
        results.append(
            CurrencyTrends(
                currency=currency,
                months=months,
                groups=groups,
                daily=_daily(
                    by_month[starts[-1]], by_month[starts[-2]], today=today, zone=zone
                ),
                weekdays=tuple(
                    _weekday(label, by_weekday[i], zone)
                    for i, label in enumerate(WEEKDAYS)
                ),
                day_types=(
                    _weekday(
                        "Weekdays", [r for i in range(5) for r in by_weekday[i]], zone
                    ),
                    _weekday(
                        "Weekend", [r for i in (5, 6) for r in by_weekday[i]], zone
                    ),
                ),
            )
        )
    return TimeTrends(
        start=starts[0], through=today, timezone=timezone, currencies=tuple(results)
    )
