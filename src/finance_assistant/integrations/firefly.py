"""Typed clients for the pinned Firefly III v1 API."""

from __future__ import annotations

from datetime import date
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
    field_validator,
)

from finance_assistant.integrations._http import (
    ReadOnlyHttpClient,
    UpstreamProtocolError,
)


class FireflyModel(BaseModel):
    """Validate consumed fields while tolerating additive upstream fields."""

    model_config = ConfigDict(extra="ignore", frozen=True)


FiniteDecimal = Annotated[Decimal, Field(allow_inf_nan=False, max_digits=38)]


class FireflyPagination(FireflyModel):
    total: int = Field(ge=0)
    count: int = Field(ge=0)
    per_page: int = Field(gt=0)
    current_page: int = Field(gt=0)
    total_pages: int = Field(ge=0)


class FireflyMeta(FireflyModel):
    pagination: FireflyPagination


class FireflyPage[RecordT: FireflyModel](FireflyModel):
    data: tuple[RecordT, ...]
    meta: FireflyMeta


class FireflyCurrencyAttributes(FireflyModel):
    code: str = Field(min_length=3, max_length=3)
    name: str = Field(min_length=1, max_length=256)
    symbol: str = Field(min_length=1, max_length=32)
    decimal_places: int = Field(ge=0, le=16)
    enabled: bool = True
    primary: bool = False
    updated_at: AwareDatetime | None = None


class FireflyCurrency(FireflyModel):
    type: str = Field(pattern="^currencies$")
    id: str = Field(min_length=1, max_length=128)
    attributes: FireflyCurrencyAttributes


class FireflyAccountAttributes(FireflyModel):
    name: str = Field(min_length=1, max_length=256)
    type: str = Field(min_length=1, max_length=64)
    currency_code: str = Field(min_length=3, max_length=3)
    currency_decimal_places: int = Field(ge=0, le=16)
    current_balance: FiniteDecimal
    active: bool = True
    include_net_worth: bool = True
    updated_at: AwareDatetime | None = None


class FireflyAccount(FireflyModel):
    type: str = Field(pattern="^accounts$")
    id: str = Field(min_length=1, max_length=128)
    attributes: FireflyAccountAttributes


class FireflyTransactionSplit(FireflyModel):
    transaction_journal_id: str = Field(min_length=1, max_length=128)
    type: str = Field(min_length=1, max_length=64)
    date: AwareDatetime
    currency_code: str = Field(min_length=3, max_length=3)
    currency_decimal_places: int = Field(ge=0, le=16)
    amount: FiniteDecimal
    description: str = Field(min_length=1, max_length=1024)
    source_id: str | None = Field(default=None, min_length=1, max_length=128)
    destination_id: str | None = Field(default=None, min_length=1, max_length=128)
    category_name: str | None = Field(default=None, min_length=1, max_length=128)

    @field_validator("category_name", mode="before")
    @classmethod
    def normalize_empty_category(cls, value: object) -> object:
        return None if value == "" else value


class FireflyTransactionAttributes(FireflyModel):
    transactions: tuple[FireflyTransactionSplit, ...] = Field(min_length=1)
    updated_at: AwareDatetime | None = None


class FireflyTransaction(FireflyModel):
    type: str = Field(pattern="^transactions$")
    id: str = Field(min_length=1, max_length=128)
    attributes: FireflyTransactionAttributes


class FireflyTransactionDocument(FireflyModel):
    data: FireflyTransaction


class FireflyPiggyBankAccount(FireflyModel):
    account_id: str = Field(min_length=1, max_length=128)
    name: str = Field(min_length=1, max_length=256)
    current_amount: FiniteDecimal


class FireflyPiggyBankAttributes(FireflyModel):
    name: str = Field(min_length=1, max_length=256)
    start_date: AwareDatetime | None = None
    target_date: AwareDatetime | None = None
    order: int | None = Field(default=None, ge=1)
    active: bool = True
    accounts: tuple[FireflyPiggyBankAccount, ...] = ()
    currency_code: str = Field(min_length=3, max_length=3)
    currency_decimal_places: int = Field(ge=0, le=16)
    target_amount: FiniteDecimal | None = None
    current_amount: FiniteDecimal
    save_per_month: FiniteDecimal | None = None
    updated_at: AwareDatetime | None = None


class FireflyPiggyBank(FireflyModel):
    type: str = Field(pattern="^piggy-banks$")
    id: str = Field(min_length=1, max_length=128)
    attributes: FireflyPiggyBankAttributes


class FireflyCurrencySum(FireflyModel):
    currency_code: str = Field(min_length=3, max_length=3)
    currency_decimal_places: int = Field(ge=0, le=16)
    sum: FiniteDecimal


class FireflyBudgetLimitAttributes(FireflyModel):
    start: AwareDatetime
    end: AwareDatetime
    budget_id: str = Field(min_length=1, max_length=128)
    currency_code: str = Field(min_length=3, max_length=3)
    currency_decimal_places: int = Field(ge=0, le=16)
    amount: FiniteDecimal
    spent: tuple[FireflyCurrencySum, ...] = ()
    updated_at: AwareDatetime | None = None


class FireflyBudgetLimit(FireflyModel):
    type: str = Field(pattern="^budget_limits$")
    id: str = Field(min_length=1, max_length=128)
    attributes: FireflyBudgetLimitAttributes


class FireflyClient:
    """Authenticated client exposing only read operations required for ingestion."""

    def __init__(
        self,
        *,
        base_url: str,
        token: SecretStr | str,
        max_pages: int = 100,
        transport: httpx2.AsyncBaseTransport | None = None,
        max_attempts: int = 3,
        backoff_seconds: float = 0.1,
    ) -> None:
        if not 1 <= max_pages <= 1_000:
            raise ValueError("max_pages must be between 1 and 1000")
        secret = token if isinstance(token, SecretStr) else SecretStr(token)
        if not secret.get_secret_value():
            raise ValueError("token cannot be empty")
        self._max_pages = max_pages
        self._http = ReadOnlyHttpClient(
            service="Firefly III",
            base_url=base_url,
            headers={
                "Accept": "application/vnd.api+json",
                "Authorization": f"Bearer {secret.get_secret_value()}",
                "User-Agent": "personal-finance-assistant/0.1",
            },
            max_attempts=max_attempts,
            backoff_seconds=backoff_seconds,
            transport=transport,
        )

    async def __aenter__(self) -> FireflyClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._http.aclose()

    async def list_currencies(self) -> tuple[FireflyCurrency, ...]:
        return await self._paginate("currencies", FireflyCurrency)

    async def list_accounts(self) -> tuple[FireflyAccount, ...]:
        return await self._paginate(
            "accounts", FireflyAccount, params={"type": "asset"}
        )

    async def list_transactions(
        self,
        *,
        start: date | None = None,
        end: date | None = None,
    ) -> tuple[FireflyTransaction, ...]:
        if start is not None and end is not None and start > end:
            raise ValueError("start cannot be after end")
        params: dict[str, str] = {}
        if start is not None:
            params["start"] = start.isoformat()
        if end is not None:
            params["end"] = end.isoformat()
        return await self._paginate("transactions", FireflyTransaction, params=params)

    async def list_piggy_banks(self) -> tuple[FireflyPiggyBank, ...]:
        return await self._paginate("piggy-banks", FireflyPiggyBank)

    async def list_budget_limits(
        self, *, start: date, end: date
    ) -> tuple[FireflyBudgetLimit, ...]:
        if start > end:
            raise ValueError("start cannot be after end")
        return await self._paginate(
            "budget-limits",
            FireflyBudgetLimit,
            params={"start": start.isoformat(), "end": end.isoformat()},
        )

    async def _paginate[RecordT: FireflyModel](
        self,
        path: str,
        record_type: type[RecordT],
        *,
        params: dict[str, str] | None = None,
    ) -> tuple[RecordT, ...]:
        records: list[RecordT] = []
        requested_page = 1
        page_type = FireflyPage[record_type]  # type: ignore[valid-type]
        while requested_page <= self._max_pages:
            query: dict[str, str | int] = {"limit": 100, "page": requested_page}
            if params:
                query.update(params)
            payload = await self._http.get_json(path, params=query)
            try:
                page = page_type.model_validate(payload)
            except ValidationError as error:
                raise UpstreamProtocolError(
                    "Firefly III response violated the pinned schema"
                ) from error
            pagination = page.meta.pagination
            if pagination.current_page != requested_page:
                raise UpstreamProtocolError("Firefly III pagination did not advance")
            records.extend(page.data)
            if requested_page >= pagination.total_pages:
                return tuple(records)
            requested_page += 1
        raise UpstreamProtocolError("Firefly III pagination exceeded the page limit")


class FireflyCategoryConflictError(RuntimeError):
    """The reviewed transaction no longer matches the Firefly preview."""


class FireflyCategoryWriteError(RuntimeError):
    """A category-only Firefly update failed without exposing response content."""


class FireflyCategoryClient:
    """Narrow client for owner-approved, category-only Firefly updates."""

    def __init__(
        self,
        *,
        base_url: str,
        token: SecretStr | str,
        transport: httpx2.AsyncBaseTransport | None = None,
    ) -> None:
        secret = token if isinstance(token, SecretStr) else SecretStr(token)
        if not secret.get_secret_value():
            raise ValueError("token cannot be empty")
        self._client = httpx2.AsyncClient(
            base_url=base_url.rstrip("/") + "/",
            headers={
                "Accept": "application/vnd.api+json",
                "Authorization": f"Bearer {secret.get_secret_value()}",
                "User-Agent": "personal-finance-assistant/0.1",
            },
            timeout=httpx2.Timeout(connect=2, read=15, write=15, pool=2),
            limits=httpx2.Limits(max_connections=2, max_keepalive_connections=1),
            follow_redirects=False,
            trust_env=False,
            transport=transport,
        )

    async def __aenter__(self) -> FireflyCategoryClient:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._client.aclose()

    async def set_category(
        self,
        *,
        transaction_id: str,
        expected_category: str | None,
        category: str | None,
    ) -> None:
        """Apply one reviewed category if Firefly still matches the preview."""

        current = await self._get_transaction(transaction_id)
        split = self._exact_split(current, transaction_id)
        if split.category_name == category:
            return
        if split.category_name != expected_category:
            raise FireflyCategoryConflictError(
                "Firefly category changed after this suggestion was created"
            )
        response = await self._client.put(
            f"transactions/{transaction_id}",
            json={
                "apply_rules": False,
                "fire_webhooks": True,
                "transactions": [
                    {
                        "transaction_journal_id": transaction_id,
                        "category_name": category,
                    }
                ],
            },
        )
        if not 200 <= response.status_code < 300:
            raise FireflyCategoryWriteError(
                f"Firefly category update returned HTTP {response.status_code}"
            )
        updated = self._parse_document(response)
        if self._exact_split(updated, transaction_id).category_name != category:
            raise FireflyCategoryWriteError(
                "Firefly did not confirm the requested category"
            )

    async def _get_transaction(self, transaction_id: str) -> FireflyTransaction:
        response = await self._client.get(f"transactions/{transaction_id}")
        if not 200 <= response.status_code < 300:
            raise FireflyCategoryWriteError(
                f"Firefly transaction lookup returned HTTP {response.status_code}"
            )
        return self._parse_document(response)

    @staticmethod
    def _parse_document(response: httpx2.Response) -> FireflyTransaction:
        if len(response.content) > 256 * 1024:
            raise FireflyCategoryWriteError("Firefly response exceeded the size limit")
        try:
            return FireflyTransactionDocument.model_validate(response.json()).data
        except (ValueError, ValidationError) as error:
            raise FireflyCategoryWriteError(
                "Firefly response violated the pinned schema"
            ) from error

    @staticmethod
    def _exact_split(
        transaction: FireflyTransaction, transaction_id: str
    ) -> FireflyTransactionSplit:
        matches = tuple(
            split
            for split in transaction.attributes.transactions
            if split.transaction_journal_id == transaction_id
        )
        if len(matches) != 1:
            raise FireflyCategoryWriteError(
                "Firefly transaction identity did not match the reviewed item"
            )
        return matches[0]
