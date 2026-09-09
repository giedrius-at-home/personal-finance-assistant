"""Single-process, privacy-safe Firefly polling for derived dashboard state."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime

from pydantic import SecretStr

from finance_assistant.ingestion.service import IngestionResult, IngestionService
from finance_assistant.integrations.firefly import FireflyClient
from finance_assistant.persistence.sqlite import SQLiteDerivedState

# Uvicorn installs a dedicated handler and does not configure arbitrary module
# loggers. Reuse its error logger so content-free poll outcomes are observable.
LOGGER = logging.getLogger("uvicorn.error")
DEFAULT_SYNC_INTERVAL_SECONDS = 86_400
MIN_SYNC_INTERVAL_SECONDS = 300


@dataclass(frozen=True, slots=True)
class FireflySyncSettings:
    """Validated non-secret settings plus an opaque Firefly credential."""

    base_url: str
    token: SecretStr
    interval_seconds: int = DEFAULT_SYNC_INTERVAL_SECONDS

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflySyncSettings | None:
        enabled = environment.get("PFA_FIREFLY_SYNC_ENABLED", "false").casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_FIREFLY_SYNC_ENABLED must be true or false")
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError("PFA_FIREFLY_API_TOKEN is required when sync is enabled")
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError("PFA_FIREFLY_BASE_URL is required when sync is enabled")
        try:
            interval = int(
                environment.get(
                    "PFA_FIREFLY_SYNC_INTERVAL_SECONDS",
                    str(DEFAULT_SYNC_INTERVAL_SECONDS),
                )
            )
        except ValueError as error:
            raise ValueError(
                "PFA_FIREFLY_SYNC_INTERVAL_SECONDS must be an integer"
            ) from error
        if interval < MIN_SYNC_INTERVAL_SECONDS:
            raise ValueError(
                f"PFA_FIREFLY_SYNC_INTERVAL_SECONDS must be at least "
                f"{MIN_SYNC_INTERVAL_SECONDS}"
            )
        return cls(base_url=base_url, token=token, interval_seconds=interval)


@dataclass(frozen=True, slots=True)
class FireflyCategoryWriteSettings:
    """Opt-in credentials for reviewed category-only writes."""

    base_url: str
    token: SecretStr

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflyCategoryWriteSettings | None:
        enabled = environment.get(
            "PFA_FIREFLY_CATEGORY_WRITES_ENABLED", "false"
        ).casefold()
        if enabled not in {"true", "false"}:
            raise ValueError(
                "PFA_FIREFLY_CATEGORY_WRITES_ENABLED must be true or false"
            )
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError(
                "PFA_FIREFLY_API_TOKEN is required when category writes are enabled"
            )
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError(
                "PFA_FIREFLY_BASE_URL is required when category writes are enabled"
            )
        return cls(base_url=base_url, token=token)


@dataclass(frozen=True, slots=True)
class FireflyCsvImportSettings:
    """Opt-in credentials for previewed, owner-approved CSV imports."""

    base_url: str
    token: SecretStr

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> FireflyCsvImportSettings | None:
        enabled = environment.get("PFA_FIREFLY_CSV_IMPORTS_ENABLED", "false").casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_FIREFLY_CSV_IMPORTS_ENABLED must be true or false")
        if enabled == "false":
            return None
        token = SecretStr(environment.get("PFA_FIREFLY_API_TOKEN", ""))
        if not token.get_secret_value():
            raise ValueError(
                "PFA_FIREFLY_API_TOKEN is required when CSV imports are enabled"
            )
        base_url = environment.get("PFA_FIREFLY_BASE_URL", "").rstrip("/")
        if not base_url:
            raise ValueError(
                "PFA_FIREFLY_BASE_URL is required when CSV imports are enabled"
            )
        return cls(base_url=base_url, token=token)


class FireflySyncRunner:
    """Fetch one read-only snapshot into the gateway-owned SQLite connection."""

    def __init__(
        self, *, state: SQLiteDerivedState, settings: FireflySyncSettings
    ) -> None:
        self._state = state
        self._settings = settings

    async def __call__(self) -> IngestionResult:
        try:
            async with FireflyClient(
                base_url=f"{self._settings.base_url}/api/v1",
                token=self._settings.token,
            ) as firefly:
                result = await IngestionService(
                    firefly=firefly, state=self._state
                ).ingest()
        except Exception:
            self._state.record_firefly_sync(success=False, now=datetime.now(UTC))
            raise
        self._state.record_firefly_sync(success=True, now=datetime.now(UTC))
        return result


async def run_firefly_sync_loop(
    sync_once: Callable[[], Awaitable[IngestionResult]],
    *,
    interval_seconds: int,
) -> None:
    """Run immediately and daily thereafter without logging financial content."""

    while True:
        try:
            result = await sync_once()
        except asyncio.CancelledError:
            raise
        except Exception as error:  # noqa: BLE001 - isolate an optional poller
            LOGGER.error("Firefly sync failed error_type=%s", type(error).__name__)
        else:
            LOGGER.info(
                "Firefly sync completed accounts=%d transactions=%d "
                "goals=%d changed=%d",
                result.accounts_seen,
                result.transactions_seen,
                result.goals_seen,
                result.projections_changed,
            )
        await asyncio.sleep(interval_seconds)
