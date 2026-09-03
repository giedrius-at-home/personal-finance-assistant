"""Privacy-preserving contracts for explicitly public web research."""

from __future__ import annotations

import hashlib
import json
import re
import unicodedata
from enum import StrEnum

from pydantic import AwareDatetime, Field, field_validator, model_validator

from finance_assistant.domain.models import StrictModel

RESEARCH_POLICY_VERSION = "2026-09-v1"
MAX_PUBLIC_NAME_LENGTH = 80

_EMAIL = re.compile(r"[^\s@]+@[^\s@]+\.[^\s@]+")
_URL = re.compile(r"(?:https?://|www\.)", re.IGNORECASE)
_IBAN = re.compile(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b", re.IGNORECASE)
_LONG_NUMBER = re.compile(r"\d{4,}")
_PUBLIC_NAME = re.compile(r"[\w .,&'()\-/]+", re.UNICODE)


class ResearchPurpose(StrEnum):
    MERCHANT_IDENTITY = "merchant_identity"
    MARKET_COMPANY_PROFILE = "market_company_profile"
    MARKET_REGULATORY_FILINGS = "market_regulatory_filings"
    MARKET_RISK_DISCLOSURES = "market_risk_disclosures"


class ResearchProvider(StrEnum):
    WIKIPEDIA = "wikipedia"
    SEC_EDGAR = "sec_edgar"


class ResearchStatus(StrEnum):
    LIVE = "live"
    FRESH_CACHE = "fresh_cache"
    STALE_CACHE = "stale_cache"
    DISABLED = "disabled"
    UNAVAILABLE = "unavailable"
    ABSTAINED = "abstained"


class MarketIdentifierKind(StrEnum):
    TICKER = "ticker"
    CIK = "cik"
    ISIN = "isin"


class MarketQuestion(StrEnum):
    COMPANY_PROFILE = "company_profile"
    REGULATORY_FILINGS = "regulatory_filings"
    RISK_DISCLOSURES = "risk_disclosures"


class MerchantResearchRequest(StrictModel):
    public_name: str = Field(min_length=2, max_length=MAX_PUBLIC_NAME_LENGTH)
    country_code: str | None = Field(default=None, min_length=2, max_length=2)

    @field_validator("public_name")
    @classmethod
    def validate_public_name(cls, value: str) -> str:
        normalized = unicodedata.normalize("NFKC", value).strip()
        if any(
            unicodedata.category(character).startswith("C") for character in normalized
        ):
            raise ValueError("public merchant name contains control text")
        if (
            _EMAIL.search(normalized)
            or _URL.search(normalized)
            or _IBAN.search(normalized)
            or _LONG_NUMBER.search(normalized)
        ):
            raise ValueError("public merchant name contains private or query-like data")
        if not _PUBLIC_NAME.fullmatch(normalized) or normalized != value.strip():
            raise ValueError("public merchant name contains unsupported query syntax")
        return normalized

    @field_validator("country_code")
    @classmethod
    def normalize_country(cls, value: str | None) -> str | None:
        if value is None:
            return None
        normalized = value.upper()
        if not normalized.isascii() or not normalized.isalpha():
            raise ValueError("country_code must contain two ASCII letters")
        return normalized


class MarketResearchRequest(StrictModel):
    identifier: str = Field(min_length=1, max_length=12)
    identifier_kind: MarketIdentifierKind
    question: MarketQuestion

    @model_validator(mode="after")
    def validate_identifier(self) -> MarketResearchRequest:
        normalized = self.identifier.upper()
        valid = {
            MarketIdentifierKind.TICKER: bool(
                re.fullmatch(r"[A-Z][A-Z0-9.-]{0,11}", normalized)
            ),
            MarketIdentifierKind.CIK: bool(re.fullmatch(r"\d{1,10}", normalized)),
            MarketIdentifierKind.ISIN: bool(
                re.fullmatch(r"[A-Z]{2}[A-Z0-9]{9}\d", normalized)
            ),
        }[self.identifier_kind]
        if not valid:
            raise ValueError("invalid public market identifier")
        object.__setattr__(self, "identifier", normalized)
        return self


class ResearchQuery(StrictModel):
    provider: ResearchProvider
    purpose: ResearchPurpose
    public_subject: str
    country_code: str | None = None
    identifier_kind: MarketIdentifierKind | None = None

    @property
    def key(self) -> str:
        payload = json.dumps(
            self.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        )
        return hashlib.sha256(payload.encode()).hexdigest()


class PublicCitation(StrictModel):
    title: str = Field(min_length=1, max_length=300)
    publisher: str = Field(min_length=1, max_length=128)
    url: str = Field(min_length=1, max_length=2_048)
    retrieved_at: AwareDatetime
    expires_at: AwareDatetime
    content_hash: str = Field(pattern=r"^[0-9a-f]{64}$")


class UntrustedPublicDocument(StrictModel):
    trust_label: str = "untrusted_public_web"
    citation: PublicCitation
    excerpt: str = Field(min_length=1, max_length=2_000)


class ResearchResult(StrictModel):
    schema_version: str = "2026-09-v1"
    policy_version: str = RESEARCH_POLICY_VERSION
    query_key: str = Field(pattern=r"^[0-9a-f]{64}$")
    purpose: ResearchPurpose
    status: ResearchStatus
    documents: tuple[UntrustedPublicDocument, ...] = Field(max_length=3)
    error_code: str | None = Field(default=None, max_length=64)
    generated_at: AwareDatetime

    @model_validator(mode="after")
    def validate_status(self) -> ResearchResult:
        if self.status in {ResearchStatus.LIVE, ResearchStatus.FRESH_CACHE}:
            if not self.documents:
                raise ValueError("available research requires cited documents")
        if self.status is ResearchStatus.DISABLED and self.documents:
            raise ValueError("disabled research cannot expose cached documents")
        return self


class MerchantResearchResult(StrictModel):
    schema_version: str = "2026-09-v1"
    public_name: str
    country_code: str | None
    research: ResearchResult
    likely_business_type: str | None = None
    interpretation: str = "owner_or_local_model_review_required"
    category_change_allowed: bool = False


class MarketResearchResult(StrictModel):
    schema_version: str = "2026-09-v1"
    identifier: str
    identifier_kind: MarketIdentifierKind
    question: MarketQuestion
    research: ResearchResult
    personalized_advice: bool = False
    trade_action_allowed: bool = False


def merchant_query(request: MerchantResearchRequest) -> ResearchQuery:
    subject = request.public_name
    if request.country_code:
        subject = f"{subject} {request.country_code} company"
    return ResearchQuery(
        provider=ResearchProvider.WIKIPEDIA,
        purpose=ResearchPurpose.MERCHANT_IDENTITY,
        public_subject=subject,
        country_code=request.country_code,
    )


def market_query(request: MarketResearchRequest) -> ResearchQuery:
    purpose = {
        MarketQuestion.COMPANY_PROFILE: ResearchPurpose.MARKET_COMPANY_PROFILE,
        MarketQuestion.REGULATORY_FILINGS: ResearchPurpose.MARKET_REGULATORY_FILINGS,
        MarketQuestion.RISK_DISCLOSURES: ResearchPurpose.MARKET_RISK_DISCLOSURES,
    }[request.question]
    return ResearchQuery(
        provider=ResearchProvider.SEC_EDGAR,
        purpose=purpose,
        public_subject=request.identifier,
        identifier_kind=request.identifier_kind,
    )


def content_hash(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()
