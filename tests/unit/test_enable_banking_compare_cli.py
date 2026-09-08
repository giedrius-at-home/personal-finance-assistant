from __future__ import annotations

from datetime import UTC, date, datetime
from pathlib import Path
from types import TracebackType
from typing import Self

import pytest

from finance_assistant import enable_banking_compare_cli as cli
from finance_assistant.integrations.enable_banking import (
    EnableBankingCoverage,
    EnableBankingSnapshot,
)


class FakeClient:
    def __init__(self, **_kwargs: object) -> None:
        pass

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        pass

    async def coverage_snapshot(self, **_kwargs: object) -> EnableBankingSnapshot:
        return EnableBankingSnapshot(
            session_valid_until=datetime(2026, 12, 1, tzinfo=UTC),
            accounts=(
                EnableBankingCoverage(
                    transaction_count=12,
                    page_count=2,
                    earliest_date=date(2026, 6, 1),
                    latest_date=date(2026, 9, 1),
                ),
            ),
        )


def test_read_secret_rejects_missing_empty_and_large_files(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="not configured"):
        cli._read_secret(tmp_path / "missing")
    empty = tmp_path / "empty"
    empty.write_text("", encoding="utf-8")
    with pytest.raises(ValueError, match="empty"):
        cli._read_secret(empty)
    large = tmp_path / "large"
    large.write_text("x" * (cli.MAX_SECRET_BYTES + 1), encoding="utf-8")
    with pytest.raises(ValueError, match="size limit"):
        cli._read_secret(large)


@pytest.mark.anyio
async def test_run_prints_only_privacy_safe_coverage(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    session = tmp_path / "session"
    session.write_text("synthetic-session\n", encoding="utf-8")
    key = tmp_path / "key"
    key.write_text("synthetic-key", encoding="utf-8")
    monkeypatch.setenv("PFA_ENABLE_BANKING_APP_ID", "synthetic-app")
    monkeypatch.setenv("PFA_ENABLE_BANKING_PRIVATE_KEY_FILE", str(key))
    monkeypatch.setenv("PFA_ENABLE_BANKING_SESSION_ID_FILE", str(session))
    monkeypatch.setattr(cli, "EnableBankingClient", FakeClient)

    await cli._run(date(2026, 6, 1), date(2026, 9, 1))

    output = capsys.readouterr().out
    assert "accounts=1" in output
    assert "transactions=12" in output
    assert "pages=2" in output
    assert "2026-06-01" in output
    assert "2026-09-01" in output
    assert "synthetic-session" not in output
    assert "synthetic-app" not in output
