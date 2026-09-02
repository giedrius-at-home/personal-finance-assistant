from collections.abc import Iterator

import httpx2
import pytest

from finance_assistant.integrations._http import (
    MAX_RESPONSE_BYTES,
    ReadOnlyHttpClient,
    UpstreamAuthenticationError,
    UpstreamProtocolError,
    UpstreamResponseError,
    UpstreamUnavailableError,
    normalize_base_url,
)


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


@pytest.mark.parametrize(
    "base_url",
    [
        "firefly:8080/api/v1",
        "ftp://firefly/api/v1",
        "http://user:password@firefly/api/v1",
        "http://firefly/api/v1?token=secret",
        "http://firefly/api/v1#fragment",
    ],
)
def test_base_url_rejects_ambiguous_or_credentialed_values(base_url: str) -> None:
    with pytest.raises(ValueError, match="base_url"):
        normalize_base_url(base_url)


@pytest.mark.anyio
async def test_authentication_error_never_includes_body_or_token() -> None:
    token = "synthetic-secret-token"

    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.headers["authorization"] == f"Bearer {token}"
        return httpx2.Response(
            401,
            json={"message": token, "transaction": "private-description"},
        )

    client = ReadOnlyHttpClient(
        service="Synthetic",
        base_url="http://upstream.test/api/",
        headers={"Authorization": f"Bearer {token}"},
        transport=httpx2.MockTransport(handler),
    )
    with pytest.raises(UpstreamAuthenticationError) as captured:
        await client.get_json("accounts")
    await client.aclose()

    assert token not in str(captured.value)
    assert "private-description" not in str(captured.value)


@pytest.mark.anyio
async def test_retry_is_bounded_and_read_only() -> None:
    statuses: Iterator[int] = iter((503, 502, 200))
    requests: list[httpx2.Request] = []

    def handler(request: httpx2.Request) -> httpx2.Response:
        requests.append(request)
        return httpx2.Response(next(statuses), json={"ok": True})

    client = ReadOnlyHttpClient(
        service="Synthetic",
        base_url="http://upstream.test/api/",
        headers={},
        backoff_seconds=0,
        transport=httpx2.MockTransport(handler),
    )
    assert await client.get_json("accounts") == {"ok": True}
    await client.aclose()

    assert len(requests) == 3
    assert {request.method for request in requests} == {"GET"}


@pytest.mark.anyio
async def test_transport_failure_is_wrapped_without_network_detail() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        raise httpx2.ConnectTimeout("synthetic host detail", request=request)

    client = ReadOnlyHttpClient(
        service="Synthetic",
        base_url="http://upstream.test/api/",
        headers={},
        max_attempts=2,
        backoff_seconds=0,
        transport=httpx2.MockTransport(handler),
    )
    with pytest.raises(UpstreamUnavailableError, match="bounded retries") as captured:
        await client.get_json("accounts")
    await client.aclose()

    assert "synthetic host detail" not in str(captured.value)


@pytest.mark.anyio
@pytest.mark.parametrize(
    ("response", "error_type"),
    [
        (
            httpx2.Response(302, headers={"location": "https://elsewhere.test"}),
            UpstreamResponseError,
        ),
        (httpx2.Response(200, text="not json"), UpstreamProtocolError),
        (
            httpx2.Response(
                200,
                content=b"{}" + b" " * MAX_RESPONSE_BYTES,
                headers={"content-type": "application/json"},
            ),
            UpstreamProtocolError,
        ),
        (
            httpx2.Response(
                200,
                content=b"42",
                headers={"content-type": "application/json"},
            ),
            UpstreamProtocolError,
        ),
    ],
)
async def test_unexpected_responses_fail_closed(
    response: httpx2.Response,
    error_type: type[Exception],
) -> None:
    client = ReadOnlyHttpClient(
        service="Synthetic",
        base_url="http://upstream.test/api/",
        headers={},
        max_attempts=1,
        transport=httpx2.MockTransport(lambda _request: response),
    )
    with pytest.raises(error_type):
        await client.get_json("accounts")
    await client.aclose()
