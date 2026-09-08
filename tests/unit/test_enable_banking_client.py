from __future__ import annotations

from datetime import UTC, date, datetime
from pathlib import Path

import httpx2
import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import (
    Encoding,
    NoEncryption,
    PrivateFormat,
)

from finance_assistant.integrations._http import UpstreamProtocolError
from finance_assistant.integrations.enable_banking import EnableBankingClient


def private_key(tmp_path: Path) -> Path:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    path = tmp_path / "synthetic.pem"
    path.write_bytes(
        key.private_bytes(Encoding.PEM, PrivateFormat.PKCS8, NoEncryption())
    )
    return path


@pytest.mark.anyio
async def test_coverage_snapshot_paginates_without_returning_content(
    tmp_path: Path,
) -> None:
    requests: list[httpx2.Request] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        requests.append(request)
        token = request.headers["authorization"].removeprefix("Bearer ")
        header = jwt.get_unverified_header(token)
        assert header["kid"] == "synthetic-app"
        if request.url.path == "/sessions/synthetic-session":
            return httpx2.Response(
                200,
                headers={"content-type": "application/json"},
                json={
                    "accounts": ["account/one"],
                    "access": {"valid_until": "2026-12-01T00:00:00Z"},
                },
            )
        assert request.url.raw_path.startswith(b"/accounts/account%2Fone/transactions?")
        assert request.url.params["date_from"] == "2026-06-01"
        assert request.url.params["date_to"] == "2026-09-01"
        if "continuation_key" not in request.url.params:
            return httpx2.Response(
                200,
                headers={"content-type": "application/json"},
                json={
                    "transactions": [{"booking_date": "2026-06-02"}],
                    "continuation_key": "next-page",
                },
            )
        assert request.url.params["continuation_key"] == "next-page"
        return httpx2.Response(
            200,
            headers={"content-type": "application/json"},
            json={"transactions": [{"value_date": "2026-08-31"}]},
        )

    async with EnableBankingClient(
        app_id="synthetic-app",
        private_key_path=private_key(tmp_path),
        clock=lambda: datetime(2026, 9, 1, tzinfo=UTC),
        transport=httpx2.MockTransport(handler),
    ) as client:
        snapshot = await client.coverage_snapshot(
            session_id="synthetic-session",
            date_from=date(2026, 6, 1),
            date_to=date(2026, 9, 1),
        )

    assert len(requests) == 3
    assert snapshot.session_valid_until == datetime(2026, 12, 1, tzinfo=UTC)
    assert snapshot.accounts[0].transaction_count == 2
    assert snapshot.accounts[0].page_count == 2
    assert snapshot.accounts[0].earliest_date == date(2026, 6, 2)
    assert snapshot.accounts[0].latest_date == date(2026, 8, 31)
    assert "merchant" not in snapshot.model_dump_json()


@pytest.mark.anyio
async def test_coverage_snapshot_rejects_repeated_cursor(tmp_path: Path) -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        payload: dict[str, object]
        if request.url.path.startswith("/sessions/"):
            payload = {"accounts": ["account-one"]}
        else:
            payload = {"transactions": [], "continuation_key": "same"}
        return httpx2.Response(
            200, headers={"content-type": "application/json"}, json=payload
        )

    async with EnableBankingClient(
        app_id="synthetic-app",
        private_key_path=private_key(tmp_path),
        clock=lambda: datetime(2026, 9, 1, tzinfo=UTC),
        transport=httpx2.MockTransport(handler),
    ) as client:
        with pytest.raises(UpstreamProtocolError, match="continuation"):
            await client.coverage_snapshot(
                session_id="synthetic-session",
                date_from=date(2026, 6, 1),
                date_to=date(2026, 9, 1),
            )


def test_client_requires_a_regular_private_key(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="regular file"):
        EnableBankingClient(
            app_id="synthetic-app", private_key_path=tmp_path / "missing.pem"
        )


@pytest.mark.parametrize("max_pages", [0, 101])
def test_client_rejects_invalid_page_limit(tmp_path: Path, max_pages: int) -> None:
    with pytest.raises(ValueError, match="max_pages"):
        EnableBankingClient(
            app_id="synthetic-app",
            private_key_path=private_key(tmp_path),
            max_pages=max_pages,
        )


def test_client_rejects_empty_app_id(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="app_id"):
        EnableBankingClient(app_id="", private_key_path=private_key(tmp_path))


@pytest.mark.anyio
async def test_client_rejects_naive_clock(tmp_path: Path) -> None:
    client = EnableBankingClient(
        app_id="synthetic-app",
        private_key_path=private_key(tmp_path),
        clock=lambda: datetime(2026, 9, 1),
    )
    with pytest.raises(ValueError, match="timezone-aware"):
        await client.__aenter__()


@pytest.mark.anyio
async def test_client_rejects_invalid_key(tmp_path: Path) -> None:
    key_path = tmp_path / "invalid.pem"
    key_path.write_text("not a private key", encoding="utf-8")
    client = EnableBankingClient(app_id="synthetic-app", private_key_path=key_path)
    with pytest.raises(ValueError, match="could not sign"):
        await client.__aenter__()


@pytest.mark.anyio
async def test_client_validates_range_and_context(tmp_path: Path) -> None:
    client = EnableBankingClient(
        app_id="synthetic-app", private_key_path=private_key(tmp_path)
    )
    with pytest.raises(RuntimeError, match="async context"):
        await client._get("sessions/synthetic")
    async with client:
        with pytest.raises(ValueError, match="date_from"):
            await client.coverage_snapshot(
                session_id="synthetic",
                date_from=date(2026, 9, 2),
                date_to=date(2026, 9, 1),
            )
        with pytest.raises(ValueError, match="session_id"):
            await client.coverage_snapshot(
                session_id="",
                date_from=date(2026, 9, 1),
                date_to=date(2026, 9, 1),
            )


@pytest.mark.anyio
async def test_client_rejects_empty_account_id(tmp_path: Path) -> None:
    transport = httpx2.MockTransport(
        lambda _request: httpx2.Response(
            200,
            headers={"content-type": "application/json"},
            json={"accounts": [""]},
        )
    )
    async with EnableBankingClient(
        app_id="synthetic-app",
        private_key_path=private_key(tmp_path),
        transport=transport,
    ) as client:
        with pytest.raises(UpstreamProtocolError, match="empty account"):
            await client.coverage_snapshot(
                session_id="synthetic",
                date_from=date(2026, 9, 1),
                date_to=date(2026, 9, 1),
            )


@pytest.mark.anyio
async def test_client_rejects_excessive_pagination(tmp_path: Path) -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        payload = (
            {"accounts": ["account-one"]}
            if request.url.path.startswith("/sessions/")
            else {"transactions": [], "continuation_key": "next"}
        )
        return httpx2.Response(
            200, headers={"content-type": "application/json"}, json=payload
        )

    async with EnableBankingClient(
        app_id="synthetic-app",
        private_key_path=private_key(tmp_path),
        max_pages=1,
        transport=httpx2.MockTransport(handler),
    ) as client:
        with pytest.raises(UpstreamProtocolError, match="page limit"):
            await client.coverage_snapshot(
                session_id="synthetic",
                date_from=date(2026, 9, 1),
                date_to=date(2026, 9, 1),
            )
