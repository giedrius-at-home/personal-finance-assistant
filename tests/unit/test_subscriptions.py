from datetime import UTC, date, datetime, timedelta
from pathlib import Path

import httpx2
import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError

from finance_assistant.api import create_app
from finance_assistant.integrations.firefly import FireflyBill, FireflyClient
from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.subscriptions import SubscriptionSnapshot, subscription_report

NOW = datetime(2026, 9, 9, 12, tzinfo=UTC)


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
        assert "No linked payment in the checked period" in page.text
        state.save_subscriptions(
            data.model_copy(update={"fetched_at": now - timedelta(days=3)})
        )
        page = client.get("/dashboard")
        assert "expected-cost totals are withheld" in page.text
        assert "expected at the configured upper limits" not in page.text
    state.close()


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
