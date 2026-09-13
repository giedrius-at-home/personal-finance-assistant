import asyncio
from pathlib import Path
from unittest.mock import AsyncMock

import pytest
from pydantic import SecretStr

from finance_assistant.ingestion.reconciliation import ReconciliationReport
from finance_assistant.ingestion.runtime import (
    FireflyCategoryWriteSettings,
    FireflyCsvImportSettings,
    FireflySyncRunner,
    FireflySyncSettings,
    GhostfolioSyncSettings,
    run_firefly_sync_loop,
)
from finance_assistant.ingestion.service import IngestionResult
from finance_assistant.persistence.sqlite import SQLiteDerivedState


@pytest.mark.anyio
async def test_runner_records_success_then_failure_without_claiming_new_success(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    result = IngestionResult(
        accounts_seen=0,
        transactions_seen=0,
        goals_seen=0,
        goals_ineligible=0,
        projections_changed=0,
        reconciliation=ReconciliationReport(matched=True, entries=()),
    )
    ingest = AsyncMock(return_value=result)
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.IngestionService.ingest", ingest
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.FireflyClient.list_bills",
        AsyncMock(return_value=()),
    )
    list_transactions = AsyncMock(return_value=())
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.FireflyClient.list_transactions",
        list_transactions,
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.FireflyClient.list_budget_limits",
        AsyncMock(return_value=()),
    )
    runner = FireflySyncRunner(
        state=state,
        settings=FireflySyncSettings(
            base_url="http://firefly.test", token=SecretStr("synthetic-token")
        ),
    )
    assert await runner() == result
    success, _, failed = state.firefly_sync_health()
    assert success is not None and not failed
    assert state.last_full_firefly_sync() is not None
    list_transactions.reset_mock()
    ingest.side_effect = RuntimeError("synthetic failure")
    with pytest.raises(RuntimeError, match="synthetic failure"):
        await runner()
    list_transactions.assert_awaited_once()
    last_success, attempt, failed = state.firefly_sync_health()
    assert last_success == success and attempt is not None and failed
    state.close()


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
    with pytest.raises(ValueError, match="cash buffer must be a number"):
        FireflySyncSettings.from_environment(
            {
                "PFA_FIREFLY_SYNC_ENABLED": "true",
                "PFA_FIREFLY_API_TOKEN": "synthetic-token",
                "PFA_FIREFLY_BASE_URL": "http://firefly.test",
                "PFA_MINIMUM_CASH_BUFFER_EUR": "many",
            }
        )


@pytest.mark.anyio
async def test_optional_alert_failure_does_not_mark_valid_snapshot_failed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    result = IngestionResult(
        accounts_seen=0,
        transactions_seen=0,
        goals_seen=0,
        goals_ineligible=0,
        projections_changed=0,
        reconciliation=ReconciliationReport(matched=True, entries=()),
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.IngestionService.ingest",
        AsyncMock(return_value=result),
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.FireflyClient.list_bills",
        AsyncMock(return_value=()),
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime.FireflyClient.list_transactions",
        AsyncMock(return_value=()),
    )
    monkeypatch.setattr(
        "finance_assistant.ingestion.runtime._evaluate_alerts",
        AsyncMock(side_effect=RuntimeError("synthetic alert failure")),
    )
    runner = FireflySyncRunner(
        state=state,
        settings=FireflySyncSettings(
            base_url="http://firefly.test", token=SecretStr("synthetic-token")
        ),
    )

    assert await runner() == result
    success, _attempt, failed = state.firefly_sync_health()
    assert success is not None and not failed
    state.close()


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


def test_ghostfolio_sync_is_opt_in_and_requires_explicit_link_ownership() -> None:
    assert GhostfolioSyncSettings.from_environment({}) is None
    with pytest.raises(ValueError, match="BEARER_TOKEN"):
        GhostfolioSyncSettings.from_environment(
            {
                "PFA_GHOSTFOLIO_SYNC_ENABLED": "true",
                "PFA_GHOSTFOLIO_BASE_URL": "http://ghostfolio.test",
            }
        )
    settings = GhostfolioSyncSettings.from_environment(
        {
            "PFA_GHOSTFOLIO_SYNC_ENABLED": "true",
            "PFA_GHOSTFOLIO_BASE_URL": "http://ghostfolio.test/",
            "PFA_GHOSTFOLIO_BEARER_TOKEN": "synthetic-token",
            "PFA_GHOSTFOLIO_ACCOUNT_LINKS": ("firefly-1:ghostfolio-1:firefly_iii"),
        }
    )

    assert settings is not None
    assert settings.base_url == "http://ghostfolio.test"
    assert settings.links[0].firefly_account_id == "firefly-1"
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


def test_csv_import_writes_are_explicitly_opt_in_and_keep_token_secret() -> None:
    assert FireflyCsvImportSettings.from_environment({}) is None
    with pytest.raises(ValueError, match="must be true or false"):
        FireflyCsvImportSettings.from_environment(
            {"PFA_FIREFLY_CSV_IMPORTS_ENABLED": "yes"}
        )
    with pytest.raises(ValueError, match="API_TOKEN"):
        FireflyCsvImportSettings.from_environment(
            {"PFA_FIREFLY_CSV_IMPORTS_ENABLED": "true"}
        )
    with pytest.raises(ValueError, match="BASE_URL"):
        FireflyCsvImportSettings.from_environment(
            {
                "PFA_FIREFLY_CSV_IMPORTS_ENABLED": "true",
                "PFA_FIREFLY_API_TOKEN": "synthetic-token",
            }
        )
    settings = FireflyCsvImportSettings.from_environment(
        {
            "PFA_FIREFLY_CSV_IMPORTS_ENABLED": "true",
            "PFA_FIREFLY_API_TOKEN": "synthetic-token",
            "PFA_FIREFLY_BASE_URL": "http://firefly.test/",
        }
    )
    assert settings is not None
    assert settings.base_url == "http://firefly.test"
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

    delays: list[float] = []

    async def no_wait(seconds: float) -> None:
        delays.append(seconds)
        if attempts >= 2:
            raise asyncio.CancelledError

    monkeypatch.setattr("finance_assistant.ingestion.runtime.asyncio.sleep", no_wait)
    with pytest.raises(asyncio.CancelledError):
        await run_firefly_sync_loop(sync_once, interval_seconds=300)
    assert attempts == 2
    assert delays == [300, 300]
