"""Owner-facing forecasts from Firefly's own subscription schedule."""

from datetime import date, datetime, timedelta
from decimal import Decimal
from zoneinfo import ZoneInfo

from pydantic import AwareDatetime, Field, model_validator

from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money, StrictModel
from finance_assistant.integrations.firefly import FireflyBill


class SubscriptionSnapshot(StrictModel):
    fetched_at: AwareDatetime
    start: date
    end: date
    bills: tuple[FireflyBill, ...] = Field(max_length=10_000)

    @model_validator(mode="after")
    def validate_snapshot(self) -> SubscriptionSnapshot:
        if self.start > self.end or len({b.id for b in self.bills}) != len(self.bills):
            raise ValueError("invalid subscription snapshot")
        for bill in self.bills:
            a = bill.attributes
            currency = Currency(
                code=a.currency_code, minor_unit=a.currency_decimal_places
            )
            Money(amount=a.amount_min, currency=currency)
            Money(amount=a.amount_max, currency=currency)
            if a.amount_min < 0 or a.amount_min > a.amount_max:
                raise ValueError("invalid subscription amount range")
        return self


class SubscriptionRow(StrictModel):
    name: str
    currency: Currency
    amount_min: Decimal
    amount_max: Decimal
    frequency: str
    expected_dates: tuple[date, ...]
    last_matched_on: date | None
    native_payment_count: int
    category_payment_count: int
    recorded_spending: Decimal
    last_recorded_on: date | None
    category_name_match: bool


class SubscriptionReport(StrictModel):
    fetched_at: AwareDatetime | None
    through_date: date
    stale: bool
    rows: tuple[SubscriptionRow, ...]
    expected_costs: tuple[Money, ...]
    unmatched_count: int
    evidence_start: date
    category_matched_names: tuple[str, ...]


def subscription_report(
    snapshot: SubscriptionSnapshot | None,
    transactions: tuple[CanonicalTransaction, ...] = (),
    *,
    now: datetime,
    timezone: str,
    next_payday: date | None,
) -> SubscriptionReport:
    if now.tzinfo is None:
        raise ValueError("now must be timezone-aware")
    zone = ZoneInfo(timezone)
    today = now.astimezone(zone).date()
    evidence_start = _month_start(today, months_ago=5)
    end = next_payday - timedelta(days=1) if next_payday else today + timedelta(days=30)
    stale = (
        snapshot is None
        or now - snapshot.fetched_at > timedelta(days=2)
        or snapshot.start > today
        or snapshot.end < end
        or snapshot.fetched_at > now
    )
    rows: list[SubscriptionRow] = []
    totals: dict[Currency, Decimal] = {}
    bills = snapshot.bills if snapshot else ()
    unique_names: dict[str, str | None] = {}
    for bill in bills:
        key = bill.attributes.name.strip().casefold()
        unique_names[key] = bill.id if key not in unique_names else None
    evidence = tuple(
        transaction
        for transaction in transactions
        if transaction.source.system is SourceSystem.FIREFLY_III
        and transaction.kind is TransactionKind.OUTFLOW
        and transaction.occurred_at <= now
        and evidence_start <= transaction.occurred_at.astimezone(zone).date() <= today
    )
    category_matched_names: set[str] = set()
    for bill in bills:
        a = bill.attributes
        expires = a.end_date.astimezone(zone).date() if a.end_date else None
        if not a.active or (expires is not None and expires < today):
            continue
        currency = Currency(code=a.currency_code, minor_unit=a.currency_decimal_places)
        dates = tuple(
            sorted(
                {
                    d.astimezone(zone).date()
                    for d in a.pay_dates
                    if today <= d.astimezone(zone).date() <= end
                    and (expires is None or d.astimezone(zone).date() <= expires)
                }
            )
        )
        paid = [p.date.astimezone(zone).date() for p in a.paid_dates if p.date <= now]
        native = [
            t
            for t in evidence
            if t.subscription_id == bill.id and t.amount.currency == currency
        ]
        category = [
            t
            for t in evidence
            if t.subscription_id is None
            and t.amount.currency == currency
            and t.category is not None
            and unique_names.get(t.category.strip().casefold()) == bill.id
        ]
        if category:
            category_matched_names.update(t.category for t in category if t.category)
        recorded = (*native, *category)
        rows.append(
            SubscriptionRow(
                name=a.name,
                currency=currency,
                amount_min=a.amount_min,
                amount_max=a.amount_max,
                frequency=a.repeat_freq,
                expected_dates=dates,
                last_matched_on=max(paid) if paid else None,
                native_payment_count=len(native),
                category_payment_count=len(category),
                recorded_spending=sum((t.amount.amount for t in recorded), Decimal(0)),
                last_recorded_on=(
                    max(t.occurred_at.astimezone(zone).date() for t in recorded)
                    if recorded
                    else None
                ),
                category_name_match=bool(category),
            )
        )
        if dates:
            totals[currency] = (
                totals.get(currency, Decimal()) + len(dates) * a.amount_max
            )
    return SubscriptionReport(
        fetched_at=snapshot.fetched_at if snapshot else None,
        through_date=end,
        stale=stale,
        rows=tuple(
            sorted(rows, key=lambda r: (r.expected_dates or (date.max,), r.name))
        ),
        expected_costs=tuple(
            Money(currency=c, amount=v)
            for c, v in sorted(totals.items(), key=lambda x: x[0].code)
        ),
        unmatched_count=sum(r.last_matched_on is None for r in rows),
        evidence_start=evidence_start,
        category_matched_names=tuple(sorted(category_matched_names, key=str.casefold)),
    )


def _month_start(today: date, *, months_ago: int) -> date:
    ordinal = today.year * 12 + today.month - 1 - months_ago
    year, month = divmod(ordinal, 12)
    return date(year, month + 1, 1)
