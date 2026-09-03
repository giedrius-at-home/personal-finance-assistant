"""Restricted, cache-backed public-web research application boundary."""

from __future__ import annotations

import asyncio
import ipaddress
import json
import socket
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from html.parser import HTMLParser
from urllib.parse import urlencode, urljoin, urlsplit

import httpx2
from defusedxml import ElementTree

from finance_assistant.domain.research import (
    MarketIdentifierKind,
    MarketResearchRequest,
    MarketResearchResult,
    MerchantResearchRequest,
    MerchantResearchResult,
    PublicCitation,
    ResearchProvider,
    ResearchQuery,
    ResearchResult,
    ResearchStatus,
    UntrustedPublicDocument,
    content_hash,
    market_query,
    merchant_query,
)
from finance_assistant.research_cache import ResearchCache

MAX_RESEARCH_BYTES = 256 * 1024
MAX_REDIRECTS = 2
ALLOWED_HOSTS = {
    ResearchProvider.WIKIPEDIA: frozenset({"en.wikipedia.org"}),
    ResearchProvider.SEC_EDGAR: frozenset({"www.sec.gov"}),
}

Resolver = Callable[[str, int], tuple[str, ...]]


class ResearchBoundaryError(RuntimeError):
    """A content-free rejection at the public research boundary."""

    def __init__(self, code: str) -> None:
        self.code = code
        super().__init__(code)


def _system_resolver(host: str, port: int) -> tuple[str, ...]:
    return tuple(
        sorted(
            {
                str(item[4][0])
                for item in socket.getaddrinfo(
                    host, port, type=socket.SOCK_STREAM, proto=socket.IPPROTO_TCP
                )
            }
        )
    )


class RestrictedResearchGateway:
    """Fetch only internally constructed HTTPS URLs from exact public hosts."""

    def __init__(
        self,
        *,
        enabled: bool,
        cache: ResearchCache | None = None,
        transport: httpx2.AsyncBaseTransport | None = None,
        resolver: Resolver = _system_resolver,
        clock: Callable[[], datetime] | None = None,
        ttl: timedelta = timedelta(hours=24),
        user_agent: str = (
            "personal-finance-assistant/0.1 "
            "(https://github.com/giedrius-at-home/personal-finance-assistant)"
        ),
    ) -> None:
        if not timedelta(minutes=5) <= ttl <= timedelta(days=30):
            raise ValueError("research cache TTL must be between 5 minutes and 30 days")
        if not user_agent.strip() or len(user_agent) > 200 or "\n" in user_agent:
            raise ValueError("research user agent must be a bounded single line")
        self._enabled = enabled
        self._cache = cache
        self._resolver = resolver
        self._clock = clock or (lambda: datetime.now(UTC))
        self._ttl = ttl
        self._client = httpx2.AsyncClient(
            timeout=httpx2.Timeout(connect=3, read=10, write=5, pool=3),
            limits=httpx2.Limits(max_connections=3, max_keepalive_connections=1),
            follow_redirects=False,
            trust_env=False,
            headers={
                "Accept": "application/json, application/atom+xml, text/html;q=0.5",
                "User-Agent": user_agent,
            },
            transport=transport,
        )

    async def __aenter__(self) -> RestrictedResearchGateway:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._client.aclose()

    async def merchant(
        self, request: MerchantResearchRequest
    ) -> MerchantResearchResult:
        result = await self.retrieve(merchant_query(request))
        if result.documents:
            expected = " ".join(request.public_name.casefold().split())
            exact = tuple(
                document
                for document in result.documents
                if " ".join(document.citation.title.casefold().split()) == expected
            )
            if len(exact) == 1:
                result = result.model_copy(update={"documents": exact})
            else:
                result = result.model_copy(
                    update={
                        "status": ResearchStatus.ABSTAINED,
                        "documents": (),
                        "error_code": "ambiguous_public_identity",
                    }
                )
        return MerchantResearchResult(
            public_name=request.public_name,
            country_code=request.country_code,
            research=result,
        )

    async def market(self, request: MarketResearchRequest) -> MarketResearchResult:
        result = await self.retrieve(market_query(request))
        return MarketResearchResult(
            identifier=request.identifier,
            identifier_kind=request.identifier_kind,
            question=request.question,
            research=result,
        )

    async def retrieve(self, query: ResearchQuery) -> ResearchResult:
        now = self._clock()
        if now.tzinfo is None:
            raise ValueError("research clock must be timezone-aware")
        if not self._enabled:
            return _empty_result(
                query, ResearchStatus.DISABLED, now, "research_disabled"
            )
        cached = self._cache.get(query.key) if self._cache else None
        if cached is not None and cached[1] > now:
            return cached[0].model_copy(
                update={"status": ResearchStatus.FRESH_CACHE, "generated_at": now}
            )
        try:
            documents = await self._fetch(query, now=now)
        except ResearchBoundaryError as error:
            code = error.code
            if cached is not None:
                return cached[0].model_copy(
                    update={
                        "status": ResearchStatus.STALE_CACHE,
                        "generated_at": now,
                        "error_code": code,
                    }
                )
            return _empty_result(query, ResearchStatus.UNAVAILABLE, now, code)
        except httpx2.TransportError as error:
            code = (
                "upstream_protocol"
                if isinstance(error, httpx2.RemoteProtocolError)
                else "network_unavailable"
            )
            if cached is not None:
                return cached[0].model_copy(
                    update={
                        "status": ResearchStatus.STALE_CACHE,
                        "generated_at": now,
                        "error_code": code,
                    }
                )
            return _empty_result(query, ResearchStatus.UNAVAILABLE, now, code)
        if not documents:
            return _empty_result(
                query, ResearchStatus.ABSTAINED, now, "no_cited_evidence"
            )
        result = ResearchResult(
            query_key=query.key,
            purpose=query.purpose,
            status=ResearchStatus.LIVE,
            documents=documents,
            generated_at=now,
        )
        if self._cache:
            self._cache.put(result)
        return result

    async def _fetch(
        self, query: ResearchQuery, *, now: datetime
    ) -> tuple[UntrustedPublicDocument, ...]:
        url = _query_url(query)
        for redirect_count in range(MAX_REDIRECTS + 1):
            await self._validate_destination(url, query.provider)
            request = self._client.build_request("GET", url)
            response = await self._client.send(request, stream=True)
            try:
                if response.status_code in {301, 302, 303, 307, 308}:
                    if redirect_count == MAX_REDIRECTS:
                        raise ResearchBoundaryError("redirect_limit")
                    location = response.headers.get("location")
                    if not location:
                        raise ResearchBoundaryError("invalid_redirect")
                    url = urljoin(url, location)
                    continue
                if response.status_code != 200:
                    raise ResearchBoundaryError("upstream_status")
                declared = response.headers.get("content-length")
                if declared:
                    try:
                        if int(declared) > MAX_RESEARCH_BYTES:
                            raise ResearchBoundaryError("response_too_large")
                    except ValueError as error:
                        raise ResearchBoundaryError("invalid_content_length") from error
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(body) + len(chunk) > MAX_RESEARCH_BYTES:
                        raise ResearchBoundaryError("response_too_large")
                    body.extend(chunk)
                return _parse_response(
                    query,
                    response_url=str(response.url),
                    content_type=response.headers.get("content-type", ""),
                    body=bytes(body),
                    now=now,
                    ttl=self._ttl,
                )
            finally:
                await response.aclose()
        raise ResearchBoundaryError("redirect_limit")  # pragma: no cover

    async def _validate_destination(self, url: str, provider: ResearchProvider) -> None:
        parts = urlsplit(url)
        host = (parts.hostname or "").lower()
        try:
            port = parts.port
        except ValueError as error:
            raise ResearchBoundaryError("destination_rejected") from error
        if (
            parts.scheme != "https"
            or host not in ALLOWED_HOSTS[provider]
            or port not in {None, 443}
            or parts.username is not None
            or parts.password is not None
            or parts.fragment
        ):
            raise ResearchBoundaryError("destination_rejected")
        addresses = await asyncio.to_thread(self._resolver, host, 443)
        if not addresses:
            raise ResearchBoundaryError("dns_unavailable")
        try:
            if any(
                not ipaddress.ip_address(address).is_global for address in addresses
            ):
                raise ResearchBoundaryError("non_public_destination")
        except ValueError as error:
            raise ResearchBoundaryError("invalid_dns_answer") from error


def _query_url(query: ResearchQuery) -> str:
    if query.provider is ResearchProvider.WIKIPEDIA:
        params: tuple[tuple[str, str], ...] = (
            ("action", "query"),
            ("exintro", "1"),
            ("explaintext", "1"),
            ("format", "json"),
            ("formatversion", "2"),
            ("generator", "search"),
            ("gsrlimit", "3"),
            ("gsrsearch", query.public_subject),
            ("inprop", "url"),
            ("maxlag", "5"),
            ("prop", "extracts|info"),
        )
        return f"https://en.wikipedia.org/w/api.php?{urlencode(params)}"
    if query.identifier_kind is MarketIdentifierKind.ISIN:
        raise ResearchBoundaryError("unsupported_identifier_for_provider")
    params = (
        ("CIK", query.public_subject),
        ("action", "getcompany"),
        ("count", "10"),
        ("output", "atom"),
    )
    return f"https://www.sec.gov/cgi-bin/browse-edgar?{urlencode(params)}"


def _parse_response(
    query: ResearchQuery,
    *,
    response_url: str,
    content_type: str,
    body: bytes,
    now: datetime,
    ttl: timedelta,
) -> tuple[UntrustedPublicDocument, ...]:
    media_type = content_type.partition(";")[0].strip().casefold()
    if query.provider is ResearchProvider.WIKIPEDIA:
        if media_type != "application/json":
            raise ResearchBoundaryError("unsupported_content_type")
        return _parse_wikipedia(query, body=body, now=now, ttl=ttl)
    if media_type not in {
        "application/atom+xml",
        "application/xml",
        "text/html",
        "text/xml",
    }:
        raise ResearchBoundaryError("unsupported_content_type")
    if media_type == "text/html":
        text = _html_text(body)
        if not text:
            return ()
        return (
            _document(
                title="SEC EDGAR company search",
                publisher="U.S. Securities and Exchange Commission",
                url=response_url,
                excerpt=text,
                now=now,
                ttl=ttl,
            ),
        )
    return _parse_sec(body=body, fallback_url=response_url, now=now, ttl=ttl)


def _parse_wikipedia(
    query: ResearchQuery, *, body: bytes, now: datetime, ttl: timedelta
) -> tuple[UntrustedPublicDocument, ...]:
    try:
        payload = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ResearchBoundaryError("invalid_json") from error
    if not isinstance(payload, dict):
        raise ResearchBoundaryError("invalid_schema")
    query_payload = payload.get("query", {})
    pages = query_payload.get("pages", []) if isinstance(query_payload, dict) else []
    if not isinstance(pages, list):
        raise ResearchBoundaryError("invalid_schema")
    documents: list[UntrustedPublicDocument] = []
    for page in pages[:3]:
        if not isinstance(page, dict):
            raise ResearchBoundaryError("invalid_schema")
        title, excerpt, url = (
            page.get("title"),
            page.get("extract"),
            page.get("fullurl"),
        )
        if (
            not isinstance(title, str)
            or not title.strip()
            or not isinstance(excerpt, str)
            or not excerpt.strip()
            or not isinstance(url, str)
            or not url.strip()
        ):
            continue
        _validate_citation_url(url, query.provider)
        documents.append(
            _document(
                title=title,
                publisher="Wikipedia",
                url=url,
                excerpt=excerpt,
                now=now,
                ttl=ttl,
            )
        )
    return tuple(documents)


def _parse_sec(
    *, body: bytes, fallback_url: str, now: datetime, ttl: timedelta
) -> tuple[UntrustedPublicDocument, ...]:
    try:
        root = ElementTree.fromstring(body)
    except ElementTree.ParseError as error:
        raise ResearchBoundaryError("invalid_xml") from error
    documents: list[UntrustedPublicDocument] = []
    for entry in root.findall("{http://www.w3.org/2005/Atom}entry")[:3]:
        title = entry.findtext("{http://www.w3.org/2005/Atom}title", "").strip()
        summary = entry.findtext("{http://www.w3.org/2005/Atom}summary", "").strip()
        link = entry.find("{http://www.w3.org/2005/Atom}link")
        url = link.get("href", "") if link is not None else fallback_url
        if not title or not summary:
            continue
        _validate_citation_url(url, ResearchProvider.SEC_EDGAR)
        documents.append(
            _document(
                title=title,
                publisher="U.S. Securities and Exchange Commission",
                url=url,
                excerpt=summary,
                now=now,
                ttl=ttl,
            )
        )
    return tuple(documents)


def _validate_citation_url(url: str, provider: ResearchProvider) -> None:
    parts = urlsplit(url)
    try:
        port = parts.port
    except ValueError as error:
        raise ResearchBoundaryError("citation_rejected") from error
    if (
        parts.scheme != "https"
        or (parts.hostname or "").lower() not in ALLOWED_HOSTS[provider]
        or port not in {None, 443}
        or parts.username is not None
        or parts.password is not None
        or parts.fragment
    ):
        raise ResearchBoundaryError("citation_rejected")


def _document(
    *,
    title: str,
    publisher: str,
    url: str,
    excerpt: str,
    now: datetime,
    ttl: timedelta,
) -> UntrustedPublicDocument:
    clean_title = " ".join(title.split())[:300]
    clean_excerpt = " ".join(excerpt.split())[:2_000]
    if not clean_title or not clean_excerpt:
        raise ResearchBoundaryError("empty_document")
    return UntrustedPublicDocument(
        citation=PublicCitation(
            title=clean_title,
            publisher=publisher,
            url=url,
            retrieved_at=now,
            expires_at=now + ttl,
            content_hash=content_hash(clean_excerpt),
        ),
        excerpt=clean_excerpt,
    )


class _TextExtractor(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.parts: list[str] = []

    def handle_data(self, data: str) -> None:
        self.parts.append(data)


def _html_text(body: bytes) -> str:
    try:
        decoded = body.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ResearchBoundaryError("unsupported_encoding") from error
    parser = _TextExtractor()
    parser.feed(decoded)
    return " ".join(" ".join(parser.parts).split())[:2_000]


def _empty_result(
    query: ResearchQuery,
    status: ResearchStatus,
    now: datetime,
    error_code: str,
) -> ResearchResult:
    return ResearchResult(
        query_key=query.key,
        purpose=query.purpose,
        status=status,
        documents=(),
        error_code=error_code,
        generated_at=now,
    )
