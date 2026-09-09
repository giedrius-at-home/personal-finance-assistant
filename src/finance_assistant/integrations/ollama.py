"""Bounded, loopback-only client for local Ollama model generation."""

from __future__ import annotations

import asyncio
import json
import math
from typing import Any
from urllib.parse import urlsplit

import httpx2

MAX_OLLAMA_RESPONSE_BYTES = 64 * 1024
MAX_OLLAMA_EMBEDDING_RESPONSE_BYTES = 4 * 1024 * 1024
MAX_EMBEDDING_INPUTS = 512
MAX_EMBEDDING_DIMENSIONS = 4_096
RETRYABLE_STATUS_CODES = frozenset({429, 502, 503, 504})

# The documented local-only host gateway used by the container topology plus
# the literal loopback names. Anything else is refused before any request.
_LOCAL_HOSTS = frozenset({"127.0.0.1", "::1", "localhost", "host.docker.internal"})


class OllamaError(RuntimeError):
    """Base error that never includes prompt or response bodies."""


class OllamaUnavailableError(OllamaError):
    """The local model endpoint could not be reached after bounded retries."""


class OllamaResponseError(OllamaError):
    """The local model endpoint returned a non-success status."""


class OllamaProtocolError(OllamaError):
    """The local model response violated the pinned contract."""


def normalize_ollama_base_url(value: str) -> str:
    """Validate a local-only Ollama base URL without embedded credentials."""

    parts = urlsplit(value)
    if parts.scheme not in {"http", "https"} or parts.hostname is None:
        raise ValueError("ollama base_url must be an absolute HTTP(S) URL")
    if parts.username is not None or parts.password is not None:
        raise ValueError("ollama base_url cannot contain credentials")
    if parts.query or parts.fragment:
        raise ValueError("ollama base_url cannot contain a query or fragment")
    if parts.path not in {"", "/"}:
        raise ValueError("ollama base_url cannot contain a path")
    host = parts.hostname.lower()
    if host not in _LOCAL_HOSTS:
        raise ValueError("ollama base_url must use a local loopback host")
    return value.rstrip("/") + "/"


class OllamaClient:
    """Authenticated, size-bounded POST client for a pinned local model."""

    def __init__(
        self,
        *,
        base_url: str,
        model: str,
        transport: httpx2.AsyncBaseTransport | None = None,
        max_attempts: int = 3,
        backoff_seconds: float = 0.1,
        read_timeout_seconds: float = 120,
    ) -> None:
        if not model.strip() or len(model.strip()) > 128:
            raise ValueError("model must be a non-empty bounded identifier")
        if not 1 <= max_attempts <= 5:
            raise ValueError("max_attempts must be between 1 and 5")
        if not 0 <= backoff_seconds <= 5:
            raise ValueError("backoff_seconds must be between 0 and 5")
        if not 1 <= read_timeout_seconds <= 600:
            raise ValueError("read_timeout_seconds must be between 1 and 600")
        self._model = model.strip()
        self._max_attempts = max_attempts
        self._backoff_seconds = backoff_seconds
        self._client = httpx2.AsyncClient(
            base_url=normalize_ollama_base_url(base_url),
            timeout=httpx2.Timeout(
                connect=2, read=read_timeout_seconds, write=10, pool=2
            ),
            limits=httpx2.Limits(max_connections=5, max_keepalive_connections=2),
            follow_redirects=False,
            trust_env=False,
            transport=transport,
        )

    async def __aenter__(self) -> OllamaClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        """Release connection-pool resources."""

        await self._client.aclose()

    async def generate(self, prompt: str, *, system: str | None = None) -> str:
        """Run one bounded generation and return only the response text."""

        payload: dict[str, Any] = {
            "model": self._model,
            "prompt": prompt,
            "stream": False,
            "format": "json",
            "options": {"temperature": 0},
            "think": False,
        }
        if system is not None:
            payload["system"] = system
        document = await self._post_json(
            "api/generate", payload, max_bytes=MAX_OLLAMA_RESPONSE_BYTES
        )
        response_text = document.get("response")
        if not isinstance(response_text, str):
            raise OllamaProtocolError("local model response violated the pinned schema")
        return response_text

    async def embed(self, inputs: tuple[str, ...]) -> tuple[tuple[float, ...], ...]:
        """Embed bounded local text for ephemeral similarity retrieval."""

        if not 1 <= len(inputs) <= MAX_EMBEDDING_INPUTS:
            raise ValueError("embedding input count is outside the supported range")
        if any(not value or len(value) > 512 for value in inputs):
            raise ValueError("embedding inputs must be non-empty and bounded")
        document = await self._post_json(
            "api/embed",
            {"model": self._model, "input": list(inputs), "truncate": False},
            max_bytes=MAX_OLLAMA_EMBEDDING_RESPONSE_BYTES,
        )
        raw_embeddings = document.get("embeddings")
        if not isinstance(raw_embeddings, list) or len(raw_embeddings) != len(inputs):
            raise OllamaProtocolError("local embedding response violated the schema")
        embeddings: list[tuple[float, ...]] = []
        dimensions: int | None = None
        for raw_embedding in raw_embeddings:
            if not isinstance(raw_embedding, list):
                raise OllamaProtocolError(
                    "local embedding response violated the schema"
                )
            embedding = tuple(
                float(value)
                for value in raw_embedding
                if isinstance(value, int | float) and not isinstance(value, bool)
            )
            if len(embedding) != len(raw_embedding) or not embedding:
                raise OllamaProtocolError(
                    "local embedding response violated the schema"
                )
            if len(embedding) > MAX_EMBEDDING_DIMENSIONS or any(
                not math.isfinite(value) for value in embedding
            ):
                raise OllamaProtocolError(
                    "local embedding response violated the schema"
                )
            dimensions = dimensions or len(embedding)
            if len(embedding) != dimensions:
                raise OllamaProtocolError(
                    "local embedding dimensions were inconsistent"
                )
            embeddings.append(embedding)
        return tuple(embeddings)

    async def _post_json(
        self, endpoint: str, payload: dict[str, Any], *, max_bytes: int
    ) -> dict[str, Any]:
        response: httpx2.Response | None = None
        transport_error: httpx2.TransportError | None = None
        for attempt in range(1, self._max_attempts + 1):
            try:
                response = await self._client.post(endpoint, json=payload)
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
            raise OllamaUnavailableError(
                "local model could not be reached after bounded retries"
            ) from transport_error
        if not 200 <= response.status_code < 300:
            raise OllamaResponseError(
                f"local model returned HTTP {response.status_code}"
            )
        if len(response.content) > max_bytes:
            raise OllamaProtocolError("local model response exceeded the size limit")
        content_type = response.headers.get("content-type", "").lower()
        if "json" not in content_type:
            raise OllamaProtocolError("local model returned non-JSON content")
        try:
            document: Any = json.loads(response.content)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise OllamaProtocolError("local model returned invalid JSON") from error
        if not isinstance(document, dict):
            raise OllamaProtocolError("local model response violated the pinned schema")
        return document
