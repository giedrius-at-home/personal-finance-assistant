"""Observed account history and calendar-based monthly payday cash flow."""

from calendar import monthrange
from collections import defaultdict
from datetime import date, datetime
from decimal import Decimal
from zoneinfo import ZoneInfo

from pydantic import AwareDatetime

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, StrictModel
from finance_assistant.domain.reporting import CurrencyWeek


class AccountHealth(StrictModel):
    account_id: str
    name: str
    source_modified_at: AwareDatetime | None
    first_transaction_on: date | None
    last_transaction_on: date | None
    transaction_count: int
    outflow_count: int
    uncategorised_outflows: int


class CashOverview(StrictModel):
    through_date: date
    period_start: date | None
    next_payday: date | None
    days_until_payday: int | None
    currencies: tuple[CurrencyWeek, ...]
    accounts: tuple[AccountHealth, ...]


def payday_window(today: date, day: int) -> tuple[date, date]:
    """Clamp monthly payday to month-end; payday itself starts a new period."""

    if not 1 <= day <= 31:
        raise ValueError("payday day must be between 1 and 31")

    def in_month(offset: int) -> date:
        year, month_index = divmod(today.year * 12 + today.month - 1 + offset, 12)
        month = month_index + 1
        return date(year, month, min(day, monthrange(year, month)[1]))

    if today >= in_month(0):
        return in_month(0), in_month(1)
    return in_month(-1), in_month(0)


def cash_overview(
    accounts: tuple[CanonicalAccount, ...],
    transactions: tuple[CanonicalTransaction, ...],
    *,
    as_of: datetime,
    timezone: str,
    payday_day: int | None,
) -> CashOverview:
    """Report available evidence without claiming bank completeness or reconciliation.

    Only Firefly's explicit transfer type removes a transaction from cash flow.
    Owner-name text is not evidence that a payment is an internal transfer.
    """

    if as_of.tzinfo is None:
        raise ValueError("as_of must be timezone-aware")
    zone = ZoneInfo(timezone)
    today = as_of.astimezone(zone).date()
    start, next_day = payday_window(today, payday_day) if payday_day else (None, None)
    selected = tuple(
        sorted(
            (
                a
                for a in accounts
                if a.canonical_owner.system is SourceSystem.FIREFLY_III
            ),
            key=lambda a: a.account_id,
        )
    )
    by_account: dict[str, list[CanonicalTransaction]] = {
        a.account_id: [] for a in selected
    }
    income: dict[Currency, Decimal] = defaultdict(Decimal)
    spending: dict[Currency, Decimal] = defaultdict(Decimal)
    transfers: dict[Currency, int] = defaultdict(int)
    for transaction in transactions:
        if transaction.source.system is not SourceSystem.FIREFLY_III:
            continue
        booked = transaction.occurred_at.astimezone(zone).date()
        if transaction.occurred_at > as_of:
            continue
        touched = {transaction.account_id}
        if transaction.kind is TransactionKind.TRANSFER:
            touched.add(transaction.counterparty_account_id or "")
        relevant = touched & by_account.keys()
        if not relevant:
            continue
        for key in relevant:
            by_account[key].append(transaction)
        if start is None or booked < start:
            continue
        currency = transaction.amount.currency
        if transaction.kind is TransactionKind.TRANSFER:
            transfers[currency] += 1
        elif transaction.kind is TransactionKind.INFLOW:
            income[currency] += transaction.amount.amount
        elif transaction.kind is TransactionKind.REFUND:
            spending[currency] -= transaction.amount.amount
        else:
            spending[currency] += transaction.amount.amount

    health: list[AccountHealth] = []
    for account in selected:
        records = by_account[account.account_id]
        dates = [t.occurred_at.astimezone(zone).date() for t in records]
        outflows = [t for t in records if t.kind is TransactionKind.OUTFLOW]
        health.append(
            AccountHealth(
                account_id=account.account_id,
                name=account.name,
                source_modified_at=account.canonical_owner.source_updated_at,
                first_transaction_on=min(dates) if dates else None,
                last_transaction_on=max(dates) if dates else None,
                transaction_count=len(records),
                outflow_count=len(outflows),
                uncategorised_outflows=sum(not t.category for t in outflows),
            )
        )
    currencies = sorted(
        set(income) | set(spending) | set(transfers), key=lambda c: c.code
    )
    return CashOverview(
        through_date=today,
        period_start=start,
        next_payday=next_day,
        days_until_payday=(next_day - today).days if next_day else None,
        accounts=tuple(health),
        currencies=tuple(
            CurrencyWeek(
                currency=c,
                income=income[c],
                spending=spending[c],
                net_cash_flow=income[c] - spending[c],
                transfer_count=transfers[c],
                categories=(),
            )
            for c in currencies
        ),
    )
