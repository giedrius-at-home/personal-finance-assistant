"""Loopback-only Ollama client tests using a mock transport."""

import json

import httpx2
import pytest

from finance_assistant.integrations.ollama import (
    OllamaClient,
    OllamaProtocolError,
    OllamaResponseError,
    OllamaUnavailableError,
    normalize_ollama_base_url,
)


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def test_normalize_ollama_base_url_accepts_only_local_hosts() -> None:
    assert (
        normalize_ollama_base_url("http://127.0.0.1:11434") == "http://127.0.0.1:11434/"
    )
    assert (
        normalize_ollama_base_url("http://host.docker.internal:11434")
        == "http://host.docker.internal:11434/"
    )
    with pytest.raises(ValueError):
        normalize_ollama_base_url("https://evil.example.com")
    with pytest.raises(ValueError):
        normalize_ollama_base_url("http://user:pass@127.0.0.1:11434")
    with pytest.raises(ValueError):
        normalize_ollama_base_url("http://127.0.0.1:11434?x=1")
    with pytest.raises(ValueError):
        normalize_ollama_base_url("ftp://127.0.0.1:11434")
    with pytest.raises(ValueError):
        normalize_ollama_base_url("http://127.0.0.1:11434#fragment")


def test_ollama_client_rejects_invalid_configuration() -> None:
    with pytest.raises(ValueError):
        OllamaClient(base_url="http://127.0.0.1:11434", model="   ")
    with pytest.raises(ValueError):
        OllamaClient(
            base_url="http://127.0.0.1:11434", model="qwen3:8b", max_attempts=6
        )
    with pytest.raises(ValueError):
        OllamaClient(
            base_url="http://127.0.0.1:11434", model="qwen3:8b", read_timeout_seconds=0
        )


@pytest.mark.anyio
async def test_generate_posts_pinned_model_and_returns_response_text() -> None:
    def handler(request: httpx2.Request) -> httpx2.Response:
        assert request.url.path == "/api/generate"
        body = json.loads(request.content)
        assert body["model"] == "qwen3:8b"
        assert body["stream"] is False
        assert "untrusted" in body["prompt"]
        return httpx2.Response(
            200,
            json={"model": "qwen3:8b", "response": '{"category": "dining"}'},
            headers={"content-type": "application/json"},
        )

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(handler),
    ) as client:
        response = await client.generate("untrusted merchant")

    assert response == '{"category": "dining"}'


@pytest.mark.anyio
async def test_generate_rejects_non_json_and_missing_response() -> None:
    def missing_response(request: httpx2.Request) -> httpx2.Response:
        return httpx2.Response(
            200,
            json={"model": "qwen3:8b"},
            headers={"content-type": "application/json"},
        )

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(missing_response),
    ) as client:
        with pytest.raises(OllamaProtocolError):
            await client.generate("merchant")

    def non_json(request: httpx2.Request) -> httpx2.Response:
        return httpx2.Response(
            200, text="not json", headers={"content-type": "text/plain"}
        )

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(non_json),
    ) as client:
        with pytest.raises(OllamaProtocolError):
            await client.generate("merchant")

    def oversized(request: httpx2.Request) -> httpx2.Response:
        return httpx2.Response(
            200,
            json={"response": "x" * 70_000},
            headers={"content-type": "application/json"},
        )

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(oversized),
    ) as client:
        with pytest.raises(OllamaProtocolError):
            await client.generate("merchant")


@pytest.mark.anyio
async def test_generate_raises_on_non_success_and_transport_error() -> None:
    def server_error(request: httpx2.Request) -> httpx2.Response:
        return httpx2.Response(500)

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(server_error),
    ) as client:
        with pytest.raises(OllamaResponseError):
            await client.generate("merchant")

    def transport_error(request: httpx2.Request) -> httpx2.Response:
        raise httpx2.ConnectError("down")

    async with OllamaClient(
        base_url="http://127.0.0.1:11434",
        model="qwen3:8b",
        transport=httpx2.MockTransport(transport_error),
        max_attempts=1,
    ) as client:
        with pytest.raises(OllamaUnavailableError):
            await client.generate("merchant")
