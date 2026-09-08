"""Minimal read-only Enable Banking client for coverage comparisons."""

from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any
from urllib.parse import quote

import httpx2
import jwt
from jwt.exceptions import InvalidKeyError
from pydantic import BaseModel, ConfigDict, Field, SecretStr, ValidationError

from finance_assistant.integrations._http import (
    ReadOnlyHttpClient,
    UpstreamProtocolError,
)

ENABLE_BANKING_API = "https://api.enablebanking.com"
MAX_ACCOUNTS = 50
MAX_PAGES = 100


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="ignore", frozen=True)


class _Access(_StrictModel):
    valid_until: datetime | None = None


class _Session(_StrictModel):
    accounts: tuple[str, ...] = Field(max_length=MAX_ACCOUNTS)
    access: _Access | None = None


class _Transaction(_StrictModel):
    booking_date: date | None = None
    value_date: date | None = None


class _TransactionPage(_StrictModel):
    transactions: tuple[_Transaction, ...]
    continuation_key: str | None = Field(default=None, max_length=4096)


class EnableBankingCoverage(BaseModel):
    """Content-free evidence about one account's available transaction history."""

    model_config = ConfigDict(frozen=True)

    transaction_count: int = Field(ge=0)
    page_count: int = Field(ge=1)
    earliest_date: date | None = None
    latest_date: date | None = None


class EnableBankingSnapshot(BaseModel):
    """Privacy-safe coverage summary for every account in one bank session."""

    model_config = ConfigDict(frozen=True)

    session_valid_until: datetime | None = None
    accounts: tuple[EnableBankingCoverage, ...]


class EnableBankingClient:
    """Query Enable Banking without returning transaction content or balances."""

    def __init__(
        self,
        *,
        app_id: SecretStr | str,
        private_key_path: Path | str,
        base_url: str = ENABLE_BANKING_API,
        clock: Callable[[], datetime] | None = None,
        max_pages: int = MAX_PAGES,
        transport: httpx2.AsyncBaseTransport | None = None,
    ) -> None:
        secret = app_id if isinstance(app_id, SecretStr) else SecretStr(app_id)
        if not secret.get_secret_value():
            raise ValueError("app_id cannot be empty")
        key_path = Path(private_key_path)
        if not key_path.is_file():
            raise ValueError("private_key_path must reference a regular file")
        if not 1 <= max_pages <= MAX_PAGES:
            raise ValueError(f"max_pages must be between 1 and {MAX_PAGES}")
        self._app_id = secret
        self._private_key_path = key_path
        self._clock = clock or (lambda: datetime.now(UTC))
        self._max_pages = max_pages
        self._transport = transport
        self._base_url = base_url
        self._http: ReadOnlyHttpClient | None = None

    async def __aenter__(self) -> EnableBankingClient:
        now = self._clock()
        if now.tzinfo is None:
            raise ValueError("clock must return a timezone-aware datetime")
        timestamp = int(now.timestamp())
        key = self._private_key_path.read_bytes()
        try:
            token = jwt.encode(
                {
                    "iss": "enablebanking.com",
                    "aud": "api.enablebanking.com",
                    "iat": timestamp,
                    "exp": timestamp + 3600,
                },
                key,
                algorithm="RS256",
                headers={
                    "alg": "RS256",
                    "typ": "JWT",
                    "kid": self._app_id.get_secret_value(),
                },
            )
        except (InvalidKeyError, TypeError, ValueError) as error:
            raise ValueError(
                "private key could not sign an Enable Banking token"
            ) from error
        self._http = ReadOnlyHttpClient(
            service="Enable Banking",
            base_url=self._base_url,
            headers={"Authorization": f"Bearer {token}", "Accept": "application/json"},
            transport=self._transport,
        )
        await self._http.__aenter__()
        return self

    async def __aexit__(self, *_args: object) -> None:
        if self._http is not None:
            await self._http.aclose()
            self._http = None

    async def coverage_snapshot(
        self,
        *,
        session_id: SecretStr | str,
        date_from: date,
        date_to: date,
    ) -> EnableBankingSnapshot:
        """Return counts and date coverage, never transaction bodies or identifiers."""

        if date_from > date_to:
            raise ValueError("date_from cannot be after date_to")
        secret = (
            session_id if isinstance(session_id, SecretStr) else SecretStr(session_id)
        )
        opaque_session = secret.get_secret_value()
        if not opaque_session:
            raise ValueError("session_id cannot be empty")
        session = self._parse(
            _Session, await self._get(f"sessions/{quote(opaque_session, safe='')}")
        )
        accounts = tuple(
            [
                await self._account_coverage(
                    account_id=account_id,
                    date_from=date_from,
                    date_to=date_to,
                )
                for account_id in session.accounts
            ]
        )
        return EnableBankingSnapshot(
            session_valid_until=session.access.valid_until if session.access else None,
            accounts=accounts,
        )

    async def _account_coverage(
        self, *, account_id: str, date_from: date, date_to: date
    ) -> EnableBankingCoverage:
        if not account_id:
            raise UpstreamProtocolError("Enable Banking returned an empty account ID")
        continuation: str | None = None
        seen: set[str] = set()
        dates: list[date] = []
        count = 0
        for page_number in range(1, self._max_pages + 1):
            params = {
                "date_from": date_from.isoformat(),
                "date_to": date_to.isoformat(),
            }
            if continuation is not None:
                params["continuation_key"] = continuation
            payload = await self._get(
                f"accounts/{quote(account_id, safe='')}/transactions", params=params
            )
            page = self._parse(_TransactionPage, payload)
            count += len(page.transactions)
            dates.extend(
                value
                for transaction in page.transactions
                if (value := transaction.booking_date or transaction.value_date)
                is not None
            )
            continuation = page.continuation_key
            if continuation is None:
                return EnableBankingCoverage(
                    transaction_count=count,
                    page_count=page_number,
                    earliest_date=min(dates, default=None),
                    latest_date=max(dates, default=None),
                )
            if continuation in seen:
                raise UpstreamProtocolError(
                    "Enable Banking repeated a transaction continuation key"
                )
            seen.add(continuation)
        raise UpstreamProtocolError("Enable Banking pagination exceeded the page limit")

    async def _get(
        self, path: str, *, params: dict[str, str] | None = None
    ) -> dict[str, Any] | list[Any]:
        if self._http is None:
            raise RuntimeError("EnableBankingClient must be used as an async context")
        return await self._http.get_json(path, params=params)

    @staticmethod
    def _parse[ModelT: BaseModel](model: type[ModelT], payload: object) -> ModelT:
        try:
            return model.model_validate(payload)
        except ValidationError as error:
            raise UpstreamProtocolError(
                "Enable Banking returned an unexpected response shape"
            ) from error
