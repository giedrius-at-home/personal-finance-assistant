import json
from datetime import date
from decimal import Decimal

import httpx2
import pytest

from finance_assistant.csv_import import ParsedCsvRow
from finance_assistant.integrations._http import UpstreamProtocolError
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyAccountAttributes,
    FireflyCategoryClient,
    FireflyCategoryConflictError,
    FireflyClient,
    FireflyCsvImportClient,
    FireflyCsvImportWriteError,
)


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
                    "include_net_worth": False,
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

    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.params["type"] == "asset"
        return httpx2.Response(200, json=payload)

    transport = httpx2.MockTransport(handler)

    async with FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=transport,
    ) as client:
        accounts = await client.list_accounts()

    assert accounts[0].attributes.current_balance == Decimal("123.45")
    assert accounts[0].attributes.include_net_worth is False


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


def transaction_document(category: str | None) -> dict[str, object]:
    return {
        "data": {
            "type": "transactions",
            "id": "journal-1",
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
                        "category_name": category,
                    }
                ]
            },
        }
    }


def test_firefly_normalizes_an_empty_category() -> None:
    document = transaction_document("")
    split = document["data"]["attributes"]["transactions"][0]  # type: ignore[index]
    from finance_assistant.integrations.firefly import FireflyTransactionSplit

    assert FireflyTransactionSplit.model_validate(split).category_name is None


@pytest.mark.anyio
async def test_reviewed_category_write_targets_only_the_exact_journal() -> None:
    methods: list[str] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        methods.append(request.method)
        assert request.url.path == "/api/v1/transactions/journal-1"
        assert request.headers["authorization"] == "Bearer synthetic-token"
        if request.method == "GET":
            return httpx2.Response(200, json=transaction_document(None))
        payload = __import__("json").loads(request.content)
        assert payload == {
            "apply_rules": False,
            "fire_webhooks": True,
            "transactions": [
                {
                    "transaction_journal_id": "journal-1",
                    "category_name": "Groceries",
                }
            ],
        }
        return httpx2.Response(200, json=transaction_document("Groceries"))

    async with FireflyCategoryClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(handler),
    ) as client:
        await client.set_category(
            transaction_id="journal-1",
            expected_category=None,
            category="Groceries",
        )

    assert methods == ["GET", "PUT"]


@pytest.mark.anyio
async def test_reviewed_category_write_refuses_stale_preview() -> None:
    client = FireflyCategoryClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(
            lambda _request: httpx2.Response(
                200, json=transaction_document("Manually changed")
            )
        ),
    )

    with pytest.raises(FireflyCategoryConflictError, match="changed"):
        await client.set_category(
            transaction_id="journal-1",
            expected_category=None,
            category="Groceries",
        )
    await client.aclose()


def csv_account(account_id: str, name: str, iban: str | None = None) -> FireflyAccount:
    return FireflyAccount(
        type="accounts",
        id=account_id,
        attributes=FireflyAccountAttributes(
            name=name,
            type="asset",
            currency_code="EUR",
            currency_decimal_places=2,
            current_balance="100.00",
            iban=iban,
        ),
    )


@pytest.mark.anyio
async def test_csv_writer_creates_transfer_and_applies_firefly_rules() -> None:
    source = csv_account("asset-1", "Synthetic one", "NL00TEST0000000001")
    destination = csv_account("asset-2", "Synthetic two", "NL00TEST0000000002")

    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.method == "POST"
        assert request.url.path == "/api/v1/transactions"
        payload = json.loads(request.content)
        assert payload["apply_rules"] is True
        assert payload["error_if_duplicate_hash"] is True
        split = payload["transactions"][0]
        assert split["type"] == "transfer"
        assert split["source_id"] == "asset-1"
        assert split["destination_id"] == "asset-2"
        assert split["external_id"].startswith("pfa-csv-v1-")
        assert "source_name" not in split
        return httpx2.Response(201, json={"data": {}})

    async with FireflyCsvImportClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(handler),
    ) as client:
        await client.create_csv_transaction(
            row=ParsedCsvRow(
                line_number=2,
                booked_on=date(2026, 9, 1),
                amount="-5.25",
                currency="EUR",
                description="Synthetic transfer",
                counterparty_iban="NL00 TEST 0000 0000 02",
            ),
            account=source,
            accounts=(source, destination),
            external_id="pfa-csv-v1-" + "a" * 64,
        )


@pytest.mark.anyio
async def test_csv_writer_reports_bounded_firefly_failure() -> None:
    selected = csv_account("asset-1", "Synthetic one")
    client = FireflyCsvImportClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(
            lambda _request: httpx2.Response(422, text="sensitive response")
        ),
    )
    with pytest.raises(FireflyCsvImportWriteError, match="HTTP 422") as caught:
        await client.create_csv_transaction(
            row=ParsedCsvRow(
                line_number=2,
                booked_on=date(2026, 9, 1),
                amount="10.00",
                currency="EUR",
                description="Synthetic income",
            ),
            account=selected,
            accounts=(selected,),
            external_id="pfa-csv-v1-" + "b" * 64,
        )
    assert "sensitive" not in str(caught.value)
    await client.aclose()


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


@pytest.mark.anyio
async def test_firefly_rejects_a_breaking_schema_change() -> None:
    payload = currency_page(1, 1)
    del payload["data"][0]["attributes"]["decimal_places"]  # type: ignore[index]
    client = FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(
            lambda _request: httpx2.Response(200, json=payload)
        ),
    )

    with pytest.raises(UpstreamProtocolError, match="pinned schema"):
        await client.list_currencies()
    await client.aclose()


@pytest.mark.anyio
async def test_firefly_piggy_bank_contract_and_pagination() -> None:
    requested_pages: list[int] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        page = int(request.url.params["page"])
        requested_pages.append(page)
        return httpx2.Response(
            200,
            json={
                "data": [
                    {
                        "type": "piggy-banks",
                        "id": f"synthetic-piggy-{page}",
                        "attributes": {
                            "name": "Synthetic emergency fund",
                            "start_date": "2026-01-01T00:00:00+00:00",
                            "target_date": "2026-12-31T00:00:00+00:00",
                            "order": page,
                            "active": True,
                            "currency_code": "EUR",
                            "currency_decimal_places": 2,
                            "target_amount": "1200.00",
                            "current_amount": "300.00",
                            "save_per_month": "100.00",
                            "accounts": [
                                {
                                    "account_id": "synthetic-account",
                                    "name": "Synthetic savings",
                                    "current_amount": "300.00",
                                }
                            ],
                        },
                    }
                ],
                "meta": {
                    "pagination": {
                        "total": 2,
                        "count": 1,
                        "per_page": 100,
                        "current_page": page,
                        "total_pages": 2,
                    }
                },
            },
        )

    async with FireflyClient(
        base_url="http://firefly.test/api/v1",
        token="synthetic-token",
        transport=httpx2.MockTransport(handler),
    ) as client:
        records = await client.list_piggy_banks()

    assert requested_pages == [1, 2]
    assert records[0].attributes.target_amount == Decimal("1200.00")
    assert records[0].attributes.accounts[0].account_id == "synthetic-account"


@pytest.mark.anyio
async def test_firefly_budget_limit_contract_uses_explicit_range() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.params["start"] == "2026-09-01"
        assert request.url.params["end"] == "2026-09-30"
        return httpx2.Response(
            200,
            json={
                "data": [
                    {
                        "type": "budget_limits",
                        "id": "synthetic-limit",
                        "attributes": {
                            "start": "2026-09-01T00:00:00+02:00",
                            "end": "2026-09-30T23:59:59+02:00",
                            "budget_id": "synthetic-budget",
                            "currency_code": "EUR",
                            "currency_decimal_places": 2,
                            "amount": "300.00",
                            "spent": [
                                {
                                    "currency_code": "EUR",
                                    "currency_decimal_places": 2,
                                    "sum": "-90.00",
                                }
                            ],
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
        records = await client.list_budget_limits(
            start=date(2026, 9, 1), end=date(2026, 9, 30)
        )

    assert records[0].attributes.spent[0].sum == Decimal("-90.00")
