from decimal import Decimal

import httpx2
import pytest

from finance_assistant.integrations._http import UpstreamProtocolError
from finance_assistant.integrations.ghostfolio import GhostfolioClient


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def activity(activity_id: str) -> dict[str, object]:
    return {
        "id": activity_id,
        "accountId": "account-1",
        "currency": "EUR",
        "date": "2026-01-02T12:00:00.000Z",
        "fee": 0.1,
        "quantity": 1.25,
        "type": "BUY",
        "unitPrice": 12.34,
        "updatedAt": "2026-01-02T12:01:00.000Z",
        "assetProfile": {
            "id": "asset-1",
            "symbol": "SYNTH",
            "dataSource": "MANUAL",
            "currency": "EUR",
            "updatedAt": "2026-01-02T12:01:00.000Z",
        },
    }


@pytest.mark.anyio
async def test_ghostfolio_accounts_use_api_key_and_exact_json_decimals() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.path == "/api/v1/account"
        assert request.headers["authorization"] == "Api-Key synthetic-key"
        return httpx2.Response(
            200,
            json={
                "accounts": [
                    {
                        "id": "account-1",
                        "name": "Synthetic brokerage",
                        "currency": "EUR",
                        "balance": 0.1,
                        "value": 12.34,
                        "valueInBaseCurrency": 12.34,
                        "createdAt": "2026-01-01T00:00:00.000Z",
                        "updatedAt": "2026-01-02T00:00:00.000Z",
                    }
                ],
                "additionalFutureField": True,
            },
        )

    async with GhostfolioClient(
        base_url="http://ghostfolio.test/api/v1/",
        api_key="synthetic-key",
        transport=httpx2.MockTransport(handler),
    ) as client:
        accounts = await client.list_accounts()

    assert accounts[0].balance == Decimal("0.1")
    assert accounts[0].value_in_base_currency == Decimal("12.34")


@pytest.mark.anyio
async def test_ghostfolio_can_use_a_bearer_session_token() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.headers["authorization"] == "Bearer synthetic-session"
        return httpx2.Response(200, json={"accounts": []})

    async with GhostfolioClient(
        base_url="http://ghostfolio.test/api/v1",
        bearer_token="synthetic-session",
        transport=httpx2.MockTransport(handler),
    ) as client:
        assert await client.list_accounts() == ()


def test_ghostfolio_requires_exactly_one_credential() -> None:
    with pytest.raises(ValueError, match="exactly one"):
        GhostfolioClient(base_url="http://ghostfolio.test/api/v1")
    with pytest.raises(ValueError, match="exactly one"):
        GhostfolioClient(
            base_url="http://ghostfolio.test/api/v1",
            api_key="synthetic-key",
            bearer_token="synthetic-session",
        )


@pytest.mark.anyio
async def test_ghostfolio_activity_pagination_and_precision() -> None:
    requested_skips: list[int] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        skip = int(request.url.params["skip"])
        requested_skips.append(skip)
        item = activity("activity-1" if skip == 0 else "activity-2")
        return httpx2.Response(200, json={"activities": [item], "count": 2})

    async with GhostfolioClient(
        base_url="http://ghostfolio.test/api/v1",
        api_key="synthetic-key",
        transport=httpx2.MockTransport(handler),
    ) as client:
        activities = await client.list_activities()

    assert requested_skips == [0, 1]
    assert [item.id for item in activities] == ["activity-1", "activity-2"]
    assert activities[0].fee == Decimal("0.1")
    assert activities[0].unit_price == Decimal("12.34")


@pytest.mark.anyio
async def test_ghostfolio_rejects_pagination_drift() -> None:
    calls = 0

    def handler(_request: httpx2.Request) -> httpx2.Response:
        nonlocal calls
        calls += 1
        return httpx2.Response(
            200,
            json={
                "activities": [activity(f"activity-{calls}")],
                "count": 2 if calls == 1 else 3,
            },
        )

    client = GhostfolioClient(
        base_url="http://ghostfolio.test/api/v1",
        api_key="synthetic-key",
        transport=httpx2.MockTransport(handler),
    )
    with pytest.raises(UpstreamProtocolError, match="count changed"):
        await client.list_activities()
    await client.aclose()
