from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import httpx2
import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError

from finance_assistant.api import create_app
from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.integrations.firefly import FireflyBill, FireflyClient
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.subscriptions import SubscriptionSnapshot, subscription_report

NOW = datetime(2026, 9, 9, 12, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def payment(
    key: str,
    *,
    category: str | None = "Synthetic subscription",
    subscription_id: str | None = None,
    amount: str = "6.00",
    when: datetime = NOW - timedelta(days=1),
    kind: TransactionKind = TransactionKind.OUTFLOW,
    system: SourceSystem = SourceSystem.FIREFLY_III,
) -> CanonicalTransaction:
    return CanonicalTransaction(
        transaction_id=key,
        source=SourceReference(
            system=system,
            entity=SourceEntity.TRANSACTION,
            external_id=key,
        ),
        account_id="synthetic-account",
        occurred_at=when,
        kind=kind,
        amount=Money(amount=amount, currency=EUR),
        category=category,
        subscription_id=subscription_id,
    )


def bill() -> FireflyBill:
    return FireflyBill.model_validate(
        {
            "type": "bills",
            "id": "synthetic-bill",
            "attributes": {
                "name": "Synthetic subscription",
                "active": True,
                "currency_code": "EUR",
                "currency_decimal_places": 2,
                "amount_min": "5.00",
                "amount_max": "7.00",
                "repeat_freq": "monthly",
                "pay_dates": [
                    "2026-09-10T00:00:00+02:00",
                    "2026-09-10T00:00:00+02:00",
                    "2026-10-10T00:00:00+02:00",
                ],
                "paid_dates": [
                    {
                        "date": "2026-08-10T00:00:00+02:00",
                        "transaction_group_id": "private-synthetic-id",
                    }
                ],
                "notes": "private-synthetic-notes",
                "end_date": None,
            },
        }
    )


def snapshot(bills: tuple[FireflyBill, ...] | None = None) -> SubscriptionSnapshot:
    return SubscriptionSnapshot(
        fetched_at=NOW,
        start=date(2026, 8, 9),
        end=date(2026, 10, 14),
        bills=bills if bills is not None else (bill(),),
    )


def test_subscription_forecast_uses_schedule_and_does_not_add_actual_payments() -> None:
    report = subscription_report(
        snapshot(), now=NOW, timezone="Europe/Amsterdam", next_payday=date(2026, 9, 25)
    )
    assert not report.stale
    assert report.expected_costs[0].amount == 7
    assert report.rows[0].last_matched_on == date(2026, 8, 10)
    assert report.rows[0].expected_dates == (date(2026, 9, 10),)
    assert report.unmatched_count == 0
    assert report.evidence_start == date(2026, 4, 1)
    assert report.rows[0].recorded_spending == 0
    tomorrow = subscription_report(
        snapshot(), now=NOW, timezone="Europe/Amsterdam", next_payday=date(2026, 9, 10)
    )
    assert not tomorrow.expected_costs


def test_expired_inactive_unmatched_and_stale_inputs_are_explicit() -> None:
    b = bill()
    inactive = b.model_copy(
        update={"attributes": b.attributes.model_copy(update={"active": False})}
    )
    assert not subscription_report(
        snapshot((inactive,)), now=NOW, timezone="Europe/Amsterdam", next_payday=None
    ).rows
    expired = b.model_copy(
        update={
            "attributes": b.attributes.model_copy(
                update={"end_date": NOW - timedelta(days=1)}
            )
        }
    )
    assert not subscription_report(
        snapshot((expired,)), now=NOW, timezone="Europe/Amsterdam", next_payday=None
    ).rows
    unmatched = b.model_copy(
        update={"attributes": b.attributes.model_copy(update={"paid_dates": ()})}
    )
    report = subscription_report(
        snapshot((unmatched,)), now=NOW, timezone="Europe/Amsterdam", next_payday=None
    )
    assert report.unmatched_count == 1
    assert report.rows[0].last_matched_on is None
    assert subscription_report(
        snapshot(),
        now=NOW + timedelta(days=3),
        timezone="Europe/Amsterdam",
        next_payday=None,
    ).stale
    assert subscription_report(
        None, now=NOW, timezone="Europe/Amsterdam", next_payday=None
    ).stale
    with pytest.raises(ValueError):
        subscription_report(
            None,
            now=NOW.replace(tzinfo=None),
            timezone="Europe/Amsterdam",
            next_payday=None,
        )


def test_native_and_exact_unique_category_evidence_never_double_count() -> None:
    rows = (
        payment("native", subscription_id="synthetic-bill"),
        payment("category"),
        payment("native-and-category", subscription_id="synthetic-bill"),
        payment("income", kind=TransactionKind.INFLOW),
        payment("other-source", system=SourceSystem.GHOSTFOLIO),
        payment("future", when=NOW + timedelta(seconds=1)),
        payment("old", when=datetime(2026, 3, 31, tzinfo=UTC)),
    )
    report = subscription_report(
        snapshot(),
        rows,
        now=NOW,
        timezone="Europe/Amsterdam",
        next_payday=None,
    )
    row = report.rows[0]
    assert row.native_payment_count == 2
    assert row.category_payment_count == 1
    assert row.recorded_spending == 18
    assert row.last_recorded_on == date(2026, 9, 8)
    assert row.category_name_match
    assert report.category_matched_names == ("Synthetic subscription",)


def test_duplicate_bill_names_disable_category_name_matching() -> None:
    duplicate = bill().model_copy(update={"id": "second-synthetic-bill"})
    report = subscription_report(
        snapshot((bill(), duplicate)),
        (payment("category"),),
        now=NOW,
        timezone="Europe/Amsterdam",
        next_payday=None,
    )
    assert all(row.category_payment_count == 0 for row in report.rows)
    assert report.category_matched_names == ()


def test_subscription_evidence_requires_matching_currency() -> None:
    usd = Currency(code="USD", minor_unit=2)
    wrong_currency = payment("wrong").model_copy(
        update={"amount": Money(amount="6.00", currency=usd)}
    )
    report = subscription_report(
        snapshot(),
        (wrong_currency,),
        now=NOW,
        timezone="Europe/Amsterdam",
        next_payday=None,
    )
    assert report.rows[0].recorded_spending == 0


def test_snapshot_strips_unused_private_fields_and_replaces_deleted_bills(
    tmp_path: Path,
) -> None:
    data = snapshot()
    assert "private-synthetic" not in data.model_dump_json()
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    assert state.load_subscriptions() is None
    state.save_subscriptions(data)
    assert state.load_subscriptions() == data
    state.close()
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    assert state.load_subscriptions() == data
    state.save_subscriptions(snapshot(()))
    assert state.load_subscriptions() == snapshot(())
    state.close()
    with pytest.raises(ValidationError):
        snapshot((bill(), bill()))
    b = bill()
    with pytest.raises(ValidationError):
        snapshot(
            (
                b.model_copy(
                    update={
                        "attributes": b.attributes.model_copy(update={"amount_max": -1})
                    }
                ),
            )
        )


def test_dashboard_withholds_stale_subscription_totals(tmp_path: Path) -> None:
    path = tmp_path / "state.sqlite3"
    now = datetime.now(UTC)
    state = SQLiteDerivedState(path)
    state.migrate_up()
    b = bill()
    b = b.model_copy(
        update={
            "attributes": b.attributes.model_copy(
                update={"pay_dates": (now + timedelta(days=1),), "paid_dates": ()}
            )
        }
    )
    data = SubscriptionSnapshot(
        fetched_at=now,
        start=(now - timedelta(days=31)).date(),
        end=(now + timedelta(days=35)).date(),
        bills=(b,),
    )
    state.save_subscriptions(data)
    with TestClient(create_app(database_path=path)) as client:
        page = client.get("/dashboard")
        assert "Synthetic subscription" in page.text
        assert "expected at the configured upper limits" in page.text
        assert "No native Firefly link in the checked data" in page.text
        state.save_subscriptions(
            data.model_copy(update={"fetched_at": now - timedelta(days=3)})
        )
        page = client.get("/dashboard")
        assert "expected-cost totals are withheld" in page.text
        assert "expected at the configured upper limits" not in page.text
    state.close()


def test_dashboard_shows_category_evidence_and_recurring_group_without_mutation(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    data = snapshot()
    state.save_subscriptions(data)
    source = payment("category-evidence")
    state.upsert(source, observed_at=NOW)
    state.close()
    with TestClient(create_app(database_path=path)) as client:
        page = client.get("/dashboard")
        assert "1 exact category-name match" in page.text
        assert "Exact match to one saved Firefly subscription" in page.text
        assert "Subscriptions &amp; recurring" in page.text
        assert "already included in monthly spending and trends" in page.text
        report = client.get("/v1/dashboard").json()["subscriptions"]
        assert report["category_matched_names"] == ["Synthetic subscription"]
        assert report["rows"][0]["recorded_spending"] == "6.00"
    state = SQLiteDerivedState(path)
    assert state.list_transactions() == (source,)
    state.close()


def test_dashboard_shows_one_expected_cost_when_bill_limits_are_equal(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    b = bill()
    equal_cost = b.model_copy(
        update={
            "attributes": b.attributes.model_copy(
                update={"amount_max": b.attributes.amount_min}
            )
        }
    )
    state.save_subscriptions(snapshot((equal_cost,)))
    state.close()

    with TestClient(create_app(database_path=path)) as client:
        page = client.get("/dashboard")
        assert "5.00 EUR" in page.text
        assert "5.00 – 5.00 EUR" not in page.text


@pytest.mark.anyio
async def test_bill_client_passes_date_window_and_validates_response() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.path == "/api/v1/bills"
        assert request.url.params["start"] == "2026-08-09"
        assert request.url.params["end"] == "2026-10-14"
        return httpx2.Response(
            200,
            json={
                "data": [bill().model_dump(mode="json")],
                "meta": {
                    "pagination": {
                        "total": 1,
                        "count": 1,
                        "per_page": 100,
                        "current_page": 1,
                        "total_pages": 1,
                    }
                },
            },
        )

    async with FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic",
        transport=httpx2.MockTransport(handler),
    ) as client:
        assert await client.list_bills(
            start=date(2026, 8, 9), end=date(2026, 10, 14)
        ) == (bill(),)
        with pytest.raises(ValueError):
            await client.list_bills(start=date(2026, 10, 14), end=date(2026, 8, 9))
