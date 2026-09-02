"""Shared fail-closed HTTP behaviour for local upstream clients."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Mapping
from decimal import Decimal
from typing import Any
from urllib.parse import urlsplit

import httpx2

MAX_RESPONSE_BYTES = 10 * 1024 * 1024
RETRYABLE_STATUS_CODES = frozenset({429, 502, 503, 504})


class UpstreamError(RuntimeError):
    """Base error that never includes response bodies or credentials."""


class UpstreamAuthenticationError(UpstreamError):
    """Authentication or authorization was rejected."""


class UpstreamRateLimitError(UpstreamError):
    """The upstream rate limit remained exhausted after bounded retries."""


class UpstreamUnavailableError(UpstreamError):
    """The upstream could not be reached after bounded retries."""


class UpstreamResponseError(UpstreamError):
    """The upstream returned a non-success status."""

    def __init__(self, service: str, status_code: int) -> None:
        self.service = service
        self.status_code = status_code
        super().__init__(f"{service} returned HTTP {status_code}")


class UpstreamProtocolError(UpstreamError):
    """The upstream response violated the pinned response contract."""


def normalize_base_url(value: str) -> str:
    """Validate an HTTP base URL without allowing embedded credentials."""

    parts = urlsplit(value)
    if parts.scheme not in {"http", "https"} or parts.hostname is None:
        raise ValueError("base_url must be an absolute HTTP(S) URL")
    if parts.username is not None or parts.password is not None:
        raise ValueError("base_url cannot contain credentials")
    if parts.query or parts.fragment:
        raise ValueError("base_url cannot contain a query or fragment")
    return value.rstrip("/") + "/"


class ReadOnlyHttpClient:
    """Bounded JSON GET client with explicit retries and no redirects."""

    def __init__(
        self,
        *,
        service: str,
        base_url: str,
        headers: Mapping[str, str],
        max_attempts: int = 3,
        backoff_seconds: float = 0.1,
        transport: httpx2.AsyncBaseTransport | None = None,
    ) -> None:
        if not 1 <= max_attempts <= 5:
            raise ValueError("max_attempts must be between 1 and 5")
        if not 0 <= backoff_seconds <= 5:
            raise ValueError("backoff_seconds must be between 0 and 5")
        self._service = service
        self._max_attempts = max_attempts
        self._backoff_seconds = backoff_seconds
        self._client = httpx2.AsyncClient(
            base_url=normalize_base_url(base_url),
            headers=dict(headers),
            timeout=httpx2.Timeout(connect=2, read=10, write=10, pool=2),
            limits=httpx2.Limits(max_connections=10, max_keepalive_connections=5),
            follow_redirects=False,
            trust_env=False,
            transport=transport,
        )

    async def __aenter__(self) -> ReadOnlyHttpClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        """Release connection-pool resources."""

        await self._client.aclose()

    async def get_json(
        self,
        path: str,
        *,
        params: Mapping[str, str | int] | None = None,
    ) -> dict[str, Any] | list[Any]:
        """GET and parse a bounded JSON document without logging its body."""

        response: httpx2.Response | None = None
        transport_error: httpx2.TransportError | None = None
        for attempt in range(1, self._max_attempts + 1):
            try:
                response = await self._client.get(path, params=params)
                transport_error = None
            except httpx2.TransportError as error:
                transport_error = error
                response = None
            if (
                response is not None
                and response.status_code not in RETRYABLE_STATUS_CODES
            ):
                break
            if attempt < self._max_attempts:
                await asyncio.sleep(self._backoff_seconds * (2 ** (attempt - 1)))

        if response is None:
            raise UpstreamUnavailableError(
                f"{self._service} could not be reached after bounded retries"
            ) from transport_error
        if response.status_code in {401, 403}:
            raise UpstreamAuthenticationError(
                f"{self._service} rejected the configured credentials"
            )
        if response.status_code == 429:
            raise UpstreamRateLimitError(
                f"{self._service} remained rate limited after bounded retries"
            )
        if not 200 <= response.status_code < 300:
            raise UpstreamResponseError(self._service, response.status_code)
        if len(response.content) > MAX_RESPONSE_BYTES:
            raise UpstreamProtocolError(f"{self._service} response exceeded size limit")
        content_type = response.headers.get("content-type", "").lower()
        if "json" not in content_type:
            raise UpstreamProtocolError(f"{self._service} returned non-JSON content")
        try:
            payload = json.loads(response.content, parse_float=Decimal)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise UpstreamProtocolError(
                f"{self._service} returned invalid JSON"
            ) from error
        if not isinstance(payload, (dict, list)):
            raise UpstreamProtocolError(
                f"{self._service} returned an invalid JSON root"
            )
        return payload
