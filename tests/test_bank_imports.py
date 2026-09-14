"""Synthetic scheduler tests; no live bank or Docker access."""

import json
import plistlib
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from scripts import bank_imports as banks


def test_due_catches_up_and_retries_failed_bank_independently() -> None:
    now = datetime(2026, 9, 13, tzinfo=UTC)
    previous = {
        "status": "success",
        "last_attempt": (now - timedelta(hours=2)).isoformat(),
    }
    assert not banks.is_due(previous, now)
    assert banks.is_due({**previous, "status": "failed"}, now)
    assert banks.is_due(previous, now + timedelta(days=1))
    assert banks.is_due({}, now)


def test_worker_keeps_secrets_off_command_and_discards_extra_fields(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(banks.shutil, "which", lambda _: "/usr/bin/docker")

    def execute(
        command: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        if command[1] == "ps":
            return subprocess.CompletedProcess(command, 0, "a" * 12, "")
        assert "synthetic-private-token" not in " ".join(command)
        assert "synthetic-private-token" in str(kwargs["input"])
        return subprocess.CompletedProcess(
            command, 0, json.dumps({"status": "success", "raw": "private response"}), ""
        )

    monkeypatch.setattr(banks.subprocess, "run", execute)
    assert banks.run_bank(
        {"access_token": "synthetic-private-token"}, check_only=False
    ) == {"status": "success"}


@pytest.mark.parametrize(
    "output", ["raw upstream error", '{"status":"private error"}', "[]"]
)
def test_worker_rejects_untrusted_output(
    monkeypatch: pytest.MonkeyPatch, output: str
) -> None:
    monkeypatch.setattr(banks.shutil, "which", lambda _: "/usr/bin/docker")
    monkeypatch.setattr(
        banks.subprocess,
        "run",
        lambda *a, **k: subprocess.CompletedProcess([], 0, output, "secret"),
    )
    result = banks.run_bank({}, check_only=True)
    assert result["status"] in {"failed", "runtime_unavailable"}
    assert output not in str(result)


def test_one_bank_failure_does_not_block_others_and_check_does_not_record_success(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(banks, "PRIVATE", tmp_path)
    for bank in banks.BANKS:
        (tmp_path / (bank + ".json")).write_text(json.dumps({"bank": bank}))
    monkeypatch.setattr(
        banks,
        "run_bank",
        lambda c, **k: {
            "status": "reauthorization_required"
            if c["bank"] == "swedbank"
            else "success"
        },
    )
    banks.run_cycle(check_only=True)
    assert not (tmp_path / "status.json").exists()
    banks.run_cycle()
    status = json.loads((tmp_path / "status.json").read_text())
    assert status["swedbank"]["last_success"] is None
    assert status["revolut"]["last_success"] is not None
    assert status["traderepublic"]["last_success"] is not None
    assert (tmp_path / "status.json").stat().st_mode & 0o777 == 0o600


def test_install_stages_private_standalone_agent_without_credentials_in_plist(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    deploy = tmp_path / "app-support"
    monkeypatch.setattr(banks, "DEPLOY", deploy)
    monkeypatch.setattr(banks, "PRIVATE", deploy / "data")
    downloads = tmp_path / "Downloads"
    downloads.mkdir()
    for bank in banks.BANKS:
        (downloads / (bank + ".json")).write_text(
            json.dumps(
                {
                    "flow": "eb",
                    "accounts": {"synthetic": 42},
                    "access_token": "synthetic-private-token",
                }
            )
        )
    agent = banks.install(downloads)
    config = plistlib.loads(agent.read_bytes())
    assert "synthetic-private-token" not in agent.read_text()
    assert config["WorkingDirectory"] == str(deploy)
    assert Path(config["ProgramArguments"][1]).parent == deploy
    assert config["StartInterval"] == 900
    assert (deploy / "bank_import_worker.php").is_file()
    assert (deploy / "data" / "abnamro.json").stat().st_mode & 0o777 == 0o600


def test_multiple_importer_containers_fail_closed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = []
    monkeypatch.setattr(banks.shutil, "which", lambda _: "/usr/bin/docker")

    def execute(
        command: list[str], **kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0, "a" * 12 + "\n" + "b" * 12, "")

    monkeypatch.setattr(banks.subprocess, "run", execute)
    assert banks.run_bank({}, check_only=False) == {"status": "runtime_unavailable"}
    assert len(calls) == 1


def test_cutover_waits_until_next_utc_day():
    previous = {
        "status": "waiting_for_completed_day",
        "last_attempt": "2026-09-14T05:00:00+00:00",
    }
    assert not banks.is_due(previous, datetime(2026, 9, 14, 23, tzinfo=UTC))
    assert banks.is_due(previous, datetime(2026, 9, 15, tzinfo=UTC))


def test_native_runner_uses_private_stdin_and_sanitizes_output(monkeypatch):
    class Process:
        returncode = 0

        def __init__(self, command, **kwargs):
            assert "synthetic-private-token" not in " ".join(command)
            assert kwargs["start_new_session"] is True

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def communicate(self, content, timeout):
            assert "synthetic-private-token" in content
            assert timeout == 900
            return json.dumps({"status": "success", "created": 0, "raw": "private"}), ""

    monkeypatch.setattr(banks.shutil, "which", lambda _: "/synthetic/uv")
    monkeypatch.setattr(banks.subprocess, "Popen", Process)
    assert banks.run_native_bank(
        {"token": "synthetic-private-token"}, check_only=True
    ) == {"status": "success", "created": 0}
