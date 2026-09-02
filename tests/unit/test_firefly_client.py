from datetime import date
from decimal import Decimal

import httpx2
import pytest

from finance_assistant.integrations._http import UpstreamProtocolError
from finance_assistant.integrations.firefly import FireflyClient


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def currency_page(page: int, total_pages: int) -> dict[str, object]:
    return {
        "data": [
            {
                "type": "currencies",
                "id": str(page),
                "attributes": {
                    "code": "EUR" if page == 1 else "JPY",
                    "name": "Euro" if page == 1 else "Japanese yen",
                    "symbol": "€" if page == 1 else "¥",
                    "decimal_places": 2 if page == 1 else 0,
                    "enabled": True,
                    "primary": page == 1,
                    "additive_future_field": "ignored",
                },
            }
        ],
        "meta": {
            "pagination": {
                "total": total_pages,
                "count": 1,
                "per_page": 100,
                "current_page": page,
                "total_pages": total_pages,
            }
        },
        "links": {},
    }


@pytest.mark.anyio
async def test_firefly_authentication_and_pagination() -> None:
    requested_pages: list[int] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.path == "/api/v1/currencies"
        assert request.headers["authorization"] == "Bearer synthetic-token"
        page = int(request.url.params["page"])
        requested_pages.append(page)
        return httpx2.Response(
            200,
            json=currency_page(page, 2),
            headers={"content-type": "application/vnd.api+json"},
        )

    async with FireflyClient(
        base_url="http://firefly.test/api/v1/",
        token="synthetic-token",
        transport=httpx2.MockTransport(handler),
    ) as client:
        currencies = await client.list_currencies()

    assert requested_pages == [1, 2]
    assert [currency.attributes.code for currency in currencies] == ["EUR", "JPY"]
    assert [currency.attributes.decimal_places for currency in currencies] == [2, 0]


@pytest.mark.anyio
async def test_firefly_accounts_preserve_exact_decimal_strings() -> None:
    payload = {
        "data": [
            {
                "type": "accounts",
                "id": "account-1",
                "attributes": {
                    "name": "Synthetic account",
                    "type": "asset",
                    "currency_code": "EUR",
                    "currency_decimal_places": 2,
                    "current_balance": "123.45",
                },
            }
        ],
        "meta": {
            "pagination": {
                "total": 1,
                "count": 1,
                "per_page": 100,
                "current_page": 1,
                "total_pages": 1,
            }
        },
    }
    transport = httpx2.MockTransport(
        lambda _request: httpx2.Response(200, json=payload)
    )

    async with FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=transport,
    ) as client:
        accounts = await client.list_accounts()

    assert accounts[0].attributes.current_balance == Decimal("123.45")


@pytest.mark.anyio
async def test_firefly_transaction_range_and_schema() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.params["start"] == "2026-01-01"
        assert request.url.params["end"] == "2026-01-31"
        return httpx2.Response(
            200,
            json={
                "data": [
                    {
                        "type": "transactions",
                        "id": "group-1",
                        "attributes": {
                            "transactions": [
                                {
                                    "transaction_journal_id": "journal-1",
                                    "type": "Withdrawal",
                                    "date": "2026-01-02T12:00:00+00:00",
                                    "currency_code": "EUR",
                                    "currency_decimal_places": 2,
                                    "amount": "12.34",
                                    "description": "Synthetic merchant",
                                    "source_id": "asset-1",
                                    "destination_id": "expense-1",
                                }
                            ]
                        },
                    }
                ],
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
        token="synthetic-token",
        transport=httpx2.MockTransport(handler),
    ) as client:
        transactions = await client.list_transactions(
            start=date(2026, 1, 1),
            end=date(2026, 1, 31),
        )

    split = transactions[0].attributes.transactions[0]
    assert split.amount == Decimal("12.34")
    assert split.transaction_journal_id == "journal-1"


@pytest.mark.anyio
async def test_firefly_rejects_invalid_range_and_pagination() -> None:
    client = FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        max_pages=1,
        transport=httpx2.MockTransport(
            lambda _request: httpx2.Response(200, json=currency_page(1, 2))
        ),
    )
    with pytest.raises(ValueError, match="start cannot be after end"):
        await client.list_transactions(
            start=date(2026, 2, 1),
            end=date(2026, 1, 1),
        )
    with pytest.raises(UpstreamProtocolError, match="page limit"):
        await client.list_currencies()
    await client.aclose()
