"""Typed read-only client for the pinned Ghostfolio API."""

from __future__ import annotations

from decimal import Decimal
from typing import Annotated

import httpx2
from pydantic import (
    AwareDatetime,
    BaseModel,
    ConfigDict,
    Field,
    SecretStr,
    ValidationError,
)

from finance_assistant.integrations._http import (
    ReadOnlyHttpClient,
    UpstreamProtocolError,
)


class GhostfolioModel(BaseModel):
    """Validate consumed fields while tolerating additive upstream fields."""

    model_config = ConfigDict(extra="ignore", frozen=True, populate_by_name=True)


FiniteDecimal = Annotated[Decimal, Field(allow_inf_nan=False, max_digits=38)]


class GhostfolioAccount(GhostfolioModel):
    id: str = Field(min_length=1, max_length=128)
    name: str | None = Field(default=None, max_length=256)
    currency: str | None = Field(default=None, min_length=3, max_length=3)
    balance: FiniteDecimal
    value: FiniteDecimal
    value_in_base_currency: FiniteDecimal = Field(alias="valueInBaseCurrency")
    created_at: AwareDatetime = Field(alias="createdAt")
    updated_at: AwareDatetime = Field(alias="updatedAt")


class GhostfolioAccountsResponse(GhostfolioModel):
    accounts: tuple[GhostfolioAccount, ...]


class GhostfolioAssetProfile(GhostfolioModel):
    id: str = Field(min_length=1, max_length=128)
    symbol: str = Field(min_length=1, max_length=128)
    data_source: str = Field(alias="dataSource", min_length=1, max_length=64)
    currency: str | None = Field(default=None, min_length=3, max_length=3)
    updated_at: AwareDatetime = Field(alias="updatedAt")


class GhostfolioActivity(GhostfolioModel):
    id: str = Field(min_length=1, max_length=128)
    account_id: str | None = Field(
        default=None, alias="accountId", min_length=1, max_length=128
    )
    currency: str | None = Field(default=None, min_length=3, max_length=3)
    date: AwareDatetime
    fee: FiniteDecimal
    quantity: FiniteDecimal
    type: str = Field(min_length=1, max_length=64)
    unit_price: FiniteDecimal = Field(alias="unitPrice")
    updated_at: AwareDatetime = Field(alias="updatedAt")
    asset_profile: GhostfolioAssetProfile = Field(alias="assetProfile")


class GhostfolioActivitiesResponse(GhostfolioModel):
    activities: tuple[GhostfolioActivity, ...]
    count: int = Field(ge=0)


class GhostfolioClient:
    """API-key client exposing only read operations required for ingestion."""

    def __init__(
        self,
        *,
        base_url: str,
        api_key: SecretStr | str | None = None,
        bearer_token: SecretStr | str | None = None,
        max_items: int = 10_000,
        transport: httpx2.AsyncBaseTransport | None = None,
        max_attempts: int = 3,
        backoff_seconds: float = 0.1,
    ) -> None:
        if not 1 <= max_items <= 100_000:
            raise ValueError("max_items must be between 1 and 100000")
        if (api_key is None) == (bearer_token is None):
            raise ValueError("configure exactly one Ghostfolio credential")
        configured = api_key if api_key is not None else bearer_token
        secret = (
            configured
            if isinstance(configured, SecretStr)
            else SecretStr(configured or "")
        )
        if not secret.get_secret_value():
            raise ValueError("Ghostfolio credential cannot be empty")
        authorization = (
            f"Api-Key {secret.get_secret_value()}"
            if api_key is not None
            else f"Bearer {secret.get_secret_value()}"
        )
        self._max_items = max_items
        self._http = ReadOnlyHttpClient(
            service="Ghostfolio",
            base_url=base_url,
            headers={
                "Accept": "application/json",
                "Authorization": authorization,
                "User-Agent": "personal-finance-assistant/0.1",
            },
            max_attempts=max_attempts,
            backoff_seconds=backoff_seconds,
            transport=transport,
        )

    async def __aenter__(self) -> GhostfolioClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._http.aclose()

    async def list_accounts(self) -> tuple[GhostfolioAccount, ...]:
        payload = await self._http.get_json("account")
        try:
            response = GhostfolioAccountsResponse.model_validate(payload)
        except ValidationError as error:
            raise UpstreamProtocolError(
                "Ghostfolio account response violated the pinned schema"
            ) from error
        return response.accounts

    async def list_activities(self) -> tuple[GhostfolioActivity, ...]:
        activities: list[GhostfolioActivity] = []
        total: int | None = None
        while total is None or len(activities) < total:
            if len(activities) >= self._max_items:
                raise UpstreamProtocolError(
                    "Ghostfolio activities exceeded the item limit"
                )
            take = min(100, self._max_items - len(activities))
            payload = await self._http.get_json(
                "activities",
                params={"skip": len(activities), "take": take},
            )
            try:
                response = GhostfolioActivitiesResponse.model_validate(payload)
            except ValidationError as error:
                raise UpstreamProtocolError(
                    "Ghostfolio activity response violated the pinned schema"
                ) from error
            if total is not None and response.count != total:
                raise UpstreamProtocolError(
                    "Ghostfolio activity count changed during pagination"
                )
            total = response.count
            if not response.activities and len(activities) < total:
                raise UpstreamProtocolError("Ghostfolio pagination did not advance")
            activities.extend(response.activities)
            if len(activities) > total or len(activities) > self._max_items:
                raise UpstreamProtocolError(
                    "Ghostfolio activities exceeded the declared bounds"
                )
        return tuple(activities)
