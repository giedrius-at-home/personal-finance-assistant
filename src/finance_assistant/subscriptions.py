"""Owner-facing forecasts from Firefly's own subscription schedule."""

from datetime import date, datetime, timedelta
from decimal import Decimal
from zoneinfo import ZoneInfo

from pydantic import AwareDatetime, Field, model_validator

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


class SubscriptionReport(StrictModel):
    fetched_at: AwareDatetime | None
    through_date: date
    stale: bool
    rows: tuple[SubscriptionRow, ...]
    expected_costs: tuple[Money, ...]
    unmatched_count: int


def subscription_report(
    snapshot: SubscriptionSnapshot | None,
    *,
    now: datetime,
    timezone: str,
    next_payday: date | None,
) -> SubscriptionReport:
    if now.tzinfo is None:
        raise ValueError("now must be timezone-aware")
    zone = ZoneInfo(timezone)
    today = now.astimezone(zone).date()
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
    for bill in snapshot.bills if snapshot else ():
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
        rows.append(
            SubscriptionRow(
                name=a.name,
                currency=currency,
                amount_min=a.amount_min,
                amount_max=a.amount_max,
                frequency=a.repeat_freq,
                expected_dates=dates,
                last_matched_on=max(paid) if paid else None,
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
    )
