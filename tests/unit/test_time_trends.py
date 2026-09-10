from datetime import UTC, date, datetime
from decimal import Decimal
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from finance_assistant.api import create_app
from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.domain.spending_groups import GroupMapping, SpendingGroup
from finance_assistant.domain.time_trends import time_trends
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.trend_charts import trend_chart

NOW = datetime(2026, 9, 10, 12, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def tx(
    key: str,
    amount: str = "10",
    *,
    when: datetime = NOW,
    kind: TransactionKind = TransactionKind.OUTFLOW,
    category: str | None = "Synthetic shop",
    currency: Currency = EUR,
) -> CanonicalTransaction:
    return CanonicalTransaction(
        transaction_id=key,
        account_id="synthetic-a",
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=key,
        ),
        occurred_at=when,
        kind=kind,
        amount=Money(amount=amount, currency=currency),
        counterparty_account_id="synthetic-b"
        if kind is TransactionKind.TRANSFER
        else None,
        category=category,
    )


def test_six_month_currency_separation_unknowns_refunds_and_confirmed_groups() -> None:
    rows = (
        tx("a", "30"),
        tx("b", "5", kind=TransactionKind.REFUND),
        tx("c", "7", category=None),
        tx("prior", "40", when=NOW.replace(month=8)),
        tx("old", "999", when=NOW.replace(month=3)),
        tx("future", "999", when=NOW.replace(hour=13)),
        tx("income", "999", kind=TransactionKind.INFLOW),
        tx("transfer", "999", kind=TransactionKind.TRANSFER),
        tx("usd", "4", currency=Currency(code="USD", minor_unit=2)),
    )
    report = time_trends(
        rows,
        (
            GroupMapping(
                category="Synthetic shop", group=SpendingGroup.GROCERIES, revision=1
            ),
        ),
        as_of=NOW,
        timezone="Europe/Amsterdam",
    )
    assert report.start == date(2026, 4, 1)
    assert report.time_of_day_available is False
    euro, usd = report.currencies
    assert [m.spending for m in euro.months] == [
        None,
        None,
        None,
        None,
        Decimal(40),
        Decimal(32),
    ]
    assert euro.months[-1].partial_month and not euro.months[-2].partial_month
    assert euro.months[-1].record_count == 3
    assert sum(g.values[-1] for g in euro.groups) == 32
    assert (
        next(g for g in euro.groups if g.group == SpendingGroup.GROCERIES).values[-1]
        == 25
    )
    assert usd.months[-1].spending == 4
    assert usd.months[-2].spending is None
    assert euro.daily[-1].current == 32 and euro.daily[-1].previous == 40


def test_zero_recorded_net_is_not_missing_and_daily_cumulative_can_fall() -> None:
    report = time_trends(
        (
            tx("a", "10", when=NOW.replace(day=1)),
            tx("b", "10", when=NOW.replace(day=3), kind=TransactionKind.REFUND),
        ),
        (),
        as_of=NOW,
        timezone="UTC",
    )
    item = report.currencies[0]
    assert item.months[-1].spending == 0 and item.months[-1].record_count == 2
    assert item.months[-2].spending is None
    assert [p.current for p in item.daily[:3]] == [10, 10, 0]
    assert all(p.previous is None for p in item.daily)
    assert time_trends((), (), as_of=NOW, timezone="UTC").currencies == ()
    with pytest.raises(ValueError, match="timezone-aware"):
        time_trends((), (), as_of=NOW.replace(tzinfo=None), timezone="UTC")


def test_leap_short_month_and_timezone_boundaries() -> None:
    now = datetime(2024, 3, 31, 12, tzinfo=UTC)
    report = time_trends(
        (
            tx("march", "3", when=datetime(2024, 2, 29, 23, 30, tzinfo=UTC)),
            tx("feb", "8", when=datetime(2024, 2, 29, 12, tzinfo=UTC)),
        ),
        (),
        as_of=now,
        timezone="Europe/Amsterdam",
    )
    assert report.start == date(2023, 10, 1)
    points = report.currencies[0].daily
    assert points[0].current == 3
    assert points[28].previous_on == date(2024, 2, 29) and points[28].previous == 8
    assert points[29].previous_on is None and points[29].previous is None
    assert points[30].current == 3


def test_weekday_patterns_use_active_purchase_dates_not_assumed_coverage() -> None:
    rows = (
        tx("a", "10", when=datetime(2026, 9, 5, 12, tzinfo=UTC)),
        tx("b", "20", when=datetime(2026, 9, 5, 13, tzinfo=UTC)),
        tx(
            "c",
            "4",
            when=datetime(2026, 9, 5, 14, tzinfo=UTC),
            kind=TransactionKind.REFUND,
        ),
        tx("d", "9", when=datetime(2026, 9, 6, 12, tzinfo=UTC)),
        tx("e", "7", when=datetime(2026, 9, 7, 12, tzinfo=UTC)),
    )
    item = time_trends(rows, (), as_of=NOW, timezone="UTC").currencies[0]
    saturday = item.weekdays[5]
    assert saturday.gross == 30 and saturday.spending == 26 and saturday.refunds == 4
    assert saturday.purchase_count == 2 and saturday.purchase_days == 1
    assert saturday.average_purchase_day == 30
    assert item.weekdays[1].spending is None
    weekdays, weekend = item.day_types
    assert weekdays.gross == 7 and weekend.gross == 39
    assert weekend.average_purchase_day == Decimal("19.5")
    assert sum(w.spending or 0 for w in item.weekdays) == item.months[-1].spending


def test_foreign_source_and_income_only_have_no_spending_trend() -> None:
    foreign = tx("external").model_copy(
        update={
            "source": SourceReference(
                system=SourceSystem.GHOSTFOLIO,
                entity=SourceEntity.ACTIVITY,
                external_id="x",
            )
        }
    )
    report = time_trends(
        (foreign, tx("income", kind=TransactionKind.INFLOW)),
        (),
        as_of=NOW,
        timezone="UTC",
    )
    assert report.currencies == ()


def test_chart_handles_negative_values_gaps_zero_and_singletons() -> None:
    chart = trend_chart(((Decimal(10), None, Decimal(-5), Decimal(0)),))
    assert chart.minimum == -5 and chart.maximum == 10
    assert len(chart.series[0].segments) == 2
    assert all(40 <= p.x <= 560 and 20 <= p.y <= 200 for p in chart.series[0].points)
    empty = trend_chart(((None, None),))
    assert empty.series[0].points == ()
    assert not empty.has_data and chart.has_data
    assert empty.maximum > empty.minimum
    assert len(trend_chart(((Decimal(0),),)).series[0].points) == 1
    shared = trend_chart(((Decimal(10),), (Decimal(20),)))
    assert shared.series[0].points[0].y > shared.series[1].points[0].y


def test_previous_later_records_do_not_create_false_zero_comparison() -> None:
    report = time_trends(
        (
            tx("old", when=NOW.replace(month=8, day=20)),
            tx("current", when=NOW),
        ),
        (),
        as_of=NOW,
        timezone="UTC",
    )
    assert report.currencies[0].months[-2].spending == 10
    assert all(p.previous is None for p in report.currencies[0].daily)


def test_empty_render_and_refund_only_weekday_averages() -> None:
    with TestClient(create_app()) as client:
        page = client.get("/dashboard")
        assert page.status_code == 200
        assert "No expense or refund records in the last six months" in page.text
    report = time_trends(
        (tx("refund", "3", kind=TransactionKind.REFUND),), (), as_of=NOW, timezone="UTC"
    )
    day = report.currencies[0].weekdays[NOW.weekday()]
    assert day.spending == -3 and day.gross == 0
    assert day.average_purchase_day is None


def test_reordered_input_and_missing_prior_category_preserve_group_totals() -> None:
    rows = (
        tx("a", "10", category="Synthetic new", when=NOW),
        tx("b", "7", category="Synthetic old", when=NOW.replace(month=8)),
    )
    mappings = (
        GroupMapping(category="Synthetic new", group=SpendingGroup.GROCERIES),
        GroupMapping(category="Synthetic old", group=SpendingGroup.TRANSPORT),
    )
    report = time_trends(rows, mappings, as_of=NOW, timezone="UTC")
    assert report == time_trends(
        tuple(reversed(rows)), mappings, as_of=NOW, timezone="UTC"
    )
    for i, month in enumerate(report.currencies[0].months):
        values = [group.values[i] for group in report.currencies[0].groups]
        if month.spending is None:
            assert all(value is None for value in values)
        else:
            assert sum(values) == month.spending


def test_dashboard_time_report_renders_without_source_writes(tmp_path: Path) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    record = tx("a", when=datetime.now(UTC))
    state.upsert(record, observed_at=NOW)
    state.close()
    with TestClient(create_app(database_path=path)) as client:
        page = client.get("/dashboard")
        assert page.status_code == 200
        for label in (
            "Spending over time",
            "Six-month trend",
            "Daily spending comparison",
            "Recorded weekday patterns",
            "Time-of-day analysis unavailable",
        ):
            assert label in page.text
        assert "<svg" in page.text
        assert (
            client.get("/v1/dashboard").json()["time_trends"]["time_of_day_available"]
            is False
        )
    state = SQLiteDerivedState(path)
    assert state.list_transactions() == (record,)
    state.close()
