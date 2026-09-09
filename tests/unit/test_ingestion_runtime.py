import asyncio

import pytest
from pydantic import SecretStr

from finance_assistant.ingestion.reconciliation import ReconciliationReport
from finance_assistant.ingestion.runtime import (
    FireflyCategoryWriteSettings,
    FireflySyncSettings,
    run_firefly_sync_loop,
)
from finance_assistant.ingestion.service import IngestionResult


def test_firefly_sync_settings_are_disabled_by_default() -> None:
    assert FireflySyncSettings.from_environment({}) is None


def test_firefly_sync_settings_validate_required_values() -> None:
    with pytest.raises(ValueError, match="must be true or false"):
        FireflySyncSettings.from_environment({"PFA_FIREFLY_SYNC_ENABLED": "yes"})
    with pytest.raises(ValueError, match="API_TOKEN is required"):
        FireflySyncSettings.from_environment({"PFA_FIREFLY_SYNC_ENABLED": "true"})
    with pytest.raises(ValueError, match="BASE_URL is required"):
        FireflySyncSettings.from_environment(
            {
                "PFA_FIREFLY_SYNC_ENABLED": "true",
                "PFA_FIREFLY_API_TOKEN": "synthetic-token",
            }
        )
    with pytest.raises(ValueError, match="must be an integer"):
        FireflySyncSettings.from_environment(
            {
                "PFA_FIREFLY_SYNC_ENABLED": "true",
                "PFA_FIREFLY_API_TOKEN": "synthetic-token",
                "PFA_FIREFLY_BASE_URL": "http://firefly.test",
                "PFA_FIREFLY_SYNC_INTERVAL_SECONDS": "daily",
            }
        )
    with pytest.raises(ValueError, match="at least 300"):
        FireflySyncSettings.from_environment(
            {
                "PFA_FIREFLY_SYNC_ENABLED": "true",
                "PFA_FIREFLY_API_TOKEN": "synthetic-token",
                "PFA_FIREFLY_BASE_URL": "http://firefly.test",
                "PFA_FIREFLY_SYNC_INTERVAL_SECONDS": "60",
            }
        )


def test_firefly_sync_settings_keep_the_token_secret() -> None:
    settings = FireflySyncSettings.from_environment(
        {
            "PFA_FIREFLY_SYNC_ENABLED": "true",
            "PFA_FIREFLY_API_TOKEN": "synthetic-token",
            "PFA_FIREFLY_BASE_URL": "http://firefly.test/",
            "PFA_FIREFLY_SYNC_INTERVAL_SECONDS": "900",
        }
    )

    assert settings is not None
    assert settings.base_url == "http://firefly.test"
    assert settings.interval_seconds == 900
    assert settings.token == SecretStr("synthetic-token")
    assert "synthetic-token" not in repr(settings)


def test_category_writes_are_explicitly_opt_in_and_keep_token_secret() -> None:
    assert FireflyCategoryWriteSettings.from_environment({}) is None
    with pytest.raises(ValueError, match="API_TOKEN"):
        FireflyCategoryWriteSettings.from_environment(
            {"PFA_FIREFLY_CATEGORY_WRITES_ENABLED": "true"}
        )
    settings = FireflyCategoryWriteSettings.from_environment(
        {
            "PFA_FIREFLY_CATEGORY_WRITES_ENABLED": "true",
            "PFA_FIREFLY_API_TOKEN": "synthetic-token",
            "PFA_FIREFLY_BASE_URL": "http://firefly.test",
        }
    )
    assert settings is not None
    assert "synthetic-token" not in repr(settings)


@pytest.mark.anyio
async def test_sync_loop_runs_immediately_and_is_cancellable() -> None:
    called = asyncio.Event()

    async def sync_once() -> IngestionResult:
        called.set()
        return IngestionResult(
            accounts_seen=2,
            transactions_seen=3,
            goals_seen=1,
            goals_ineligible=0,
            projections_changed=6,
            reconciliation=ReconciliationReport(matched=True, entries=()),
        )

    task = asyncio.create_task(
        run_firefly_sync_loop(sync_once, interval_seconds=86_400)
    )
    await asyncio.wait_for(called.wait(), timeout=1)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task


@pytest.mark.anyio
async def test_sync_loop_contains_failures_and_retries(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    attempts = 0

    async def sync_once() -> IngestionResult:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise RuntimeError("synthetic failure")
        return IngestionResult(
            accounts_seen=1,
            transactions_seen=1,
            goals_seen=0,
            goals_ineligible=0,
            projections_changed=2,
            reconciliation=ReconciliationReport(matched=True, entries=()),
        )

    async def no_wait(_seconds: float) -> None:
        if attempts >= 2:
            raise asyncio.CancelledError

    monkeypatch.setattr("finance_assistant.ingestion.runtime.asyncio.sleep", no_wait)
    with pytest.raises(asyncio.CancelledError):
        await run_firefly_sync_loop(sync_once, interval_seconds=300)
    assert attempts == 2
