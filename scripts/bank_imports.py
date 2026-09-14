"""Private, serialized macOS scheduling for the upstream bank API importer."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import plistlib
import re
import shutil
import signal
import subprocess  # nosec B404
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
BANKS = ("abnamro", "revolut", "swedbank", "traderepublic")
LABEL = "local.personal-finance-assistant.bank-imports"
DEPLOY = (
    Path.home()
    / "Library"
    / "Application Support"
    / "Personal Finance Assistant"
    / "Bank Imports"
)
PRIVATE = DEPLOY / "data"
STATUSES = frozenset(
    {
        "verified",
        "waiting_for_completed_day",
        "success",
        "failed",
        "import_failed",
        "reauthorization_required",
        "mapping_mismatch",
        "invalid_configuration",
        "invalid_mapping",
        "rate_limited",
        "upstream_unavailable",
        "runtime_unavailable",
        "missing_config",
        "provider_pagination_error",
        "unsupported_importer_version",
        "event_review_required",
        "transfer_mapping_required",
        "legacy_overlap_after_cutoff",
        "existing_event_changed",
        "ledger_operation_failed",
    }
)


def write_private(path: Path, content: bytes) -> None:
    """Publish private configuration/status atomically."""
    temporary = path.with_suffix(path.suffix + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(content)
    os.chmod(temporary, 0o600)
    temporary.replace(path)


def is_due(previous: dict[str, Any], now: datetime) -> bool:
    """Daily per-bank success cadence, hourly retries, catch up after sleep."""
    last = previous.get("last_attempt")
    if not isinstance(last, str):
        return True
    if previous.get("status") == "waiting_for_completed_day":
        return (
            now.astimezone(UTC).date()
            > datetime.fromisoformat(last).astimezone(UTC).date()
        )
    delay = (
        timedelta(days=1) if previous.get("status") == "success" else timedelta(hours=1)
    )
    return now >= datetime.fromisoformat(last) + delay


def run_bank(configuration: dict[str, Any], *, check_only: bool) -> dict[str, Any]:
    """Send secrets on stdin; never emit raw importer output or exception text."""
    if configuration.get("pfa_provider") == "pytr":
        return run_native_bank(configuration, check_only=check_only)
    docker = shutil.which("docker")
    if docker is None:
        return {"status": "runtime_unavailable"}
    worker = (
        Path(__file__)
        .with_name("bank_import_worker.php")
        .read_text()
        .removeprefix("<?php")
    )
    try:
        discovered = subprocess.run(  # noqa: S603  # nosec B603
            [
                docker,
                "ps",
                "-q",
                "--filter",
                "label=com.docker.compose.project=personal-finance-assistant",
                "--filter",
                "label=com.docker.compose.service=importer",
            ],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        container = discovered.stdout.strip()
        if (
            discovered.returncode != 0
            or re.fullmatch(r"[a-f0-9]{12,64}", container) is None
        ):
            return {"status": "runtime_unavailable"}
    except OSError, subprocess.TimeoutExpired:
        return {"status": "runtime_unavailable"}
    command = [
        docker,
        "exec",
        "-i",
        "-e",
        "LOG_CHANNEL=null",
        "-e",
        "LOG_LEVEL=emergency",
        "-e",
        "IMPORT_DIR_ALLOWLIST=/tmp",
        "-e",
        "ENABLE_MAIL_REPORT=false",
        container,
        "timeout",
        "--kill-after=10",
        "900",
        "php",
        "-r",
        worker,
    ]
    try:
        # Fixed command and adapter; all financial configuration travels over stdin.
        result = subprocess.run(  # noqa: S603  # nosec B603
            command,
            text=True,
            capture_output=True,
            timeout=930,
            input=json.dumps(
                {"configuration": configuration, "check_only": check_only}
            ),
            check=False,
        )
        if result.returncode != 0:
            return {"status": "runtime_unavailable"}
        decoded = json.loads(result.stdout)
        status = decoded.get("status")
        if status not in STATUSES:
            return {"status": "failed"}
        safe: dict[str, Any] = {"status": status}
        if decoded.get("phase") in {"preflight", "import", "summary"}:
            safe["phase"] = decoded["phase"]
        if decoded.get("exception_type") in {
            "Error",
            "TypeError",
            "ValueError",
            "RuntimeException",
            "ReflectionException",
            "JsonException",
            "InvalidArgumentException",
            "UnexpectedValueException",
            "ErrorException",
            "ImporterErrorException",
            "ImporterHttpException",
            "ValidationException",
            "CommandNotFoundException",
            "BindingResolutionException",
        }:
            safe["exception_type"] = decoded["exception_type"]
        if isinstance(decoded.get("exit_code"), int):
            safe["exit_code"] = decoded["exit_code"]
        for field in (
            "converted",
            "conversion_errors",
            "conversion_warnings",
            "submission_errors",
            "submission_messages",
        ):
            if isinstance(decoded.get(field), int):
                safe[field] = decoded[field]
        return safe
    except OSError, subprocess.TimeoutExpired, ValueError, AttributeError, TypeError:
        return {"status": "runtime_unavailable"}


def run_native_bank(
    configuration: dict[str, Any], *, check_only: bool
) -> dict[str, Any]:
    """Run pinned native adapter with private stdin and bounded lifetime."""
    uv = shutil.which("uv")
    if uv is None:
        return {"status": "runtime_unavailable"}
    try:
        command = [
            uv,
            "run",
            "--no-project",
            "--with",
            "pytr==0.4.10",
            "python",
            str(Path(__file__).with_name("trade_republic_sync.py")),
        ]
        with subprocess.Popen(  # noqa: S603  # nosec B603
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        ) as process:
            try:
                stdout, _ = process.communicate(
                    json.dumps(
                        {"configuration": configuration, "check_only": check_only}
                    ),
                    timeout=900,
                )
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.communicate(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.communicate()
                return {"status": "runtime_unavailable"}
            returncode = process.returncode
        decoded = json.loads(stdout)
        if returncode != 0 or decoded.get("status") not in STATUSES:
            return {"status": "failed"}
        safe = {"status": decoded["status"]}
        for field in ("converted", "created", "historical_preserved"):
            if type(decoded.get(field)) is int:
                safe[field] = decoded[field]
        if type(decoded.get("bank_balance_equals_ledger")) is bool:
            safe["bank_balance_equals_ledger"] = decoded["bank_balance_equals_ledger"]
        return safe
    except OSError, ValueError, AttributeError, subprocess.TimeoutExpired:
        return {"status": "runtime_unavailable"}


def run_cycle(
    *, force: bool = False, check_only: bool = False, selected_bank: str | None = None
) -> None:
    PRIVATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    PRIVATE.chmod(0o700)
    with (PRIVATE / "run.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return
        status_path = PRIVATE / "status.json"
        statuses = json.loads(status_path.read_text()) if status_path.exists() else {}
        for bank in BANKS:
            if selected_bank is not None and selected_bank != bank:
                continue
            previous = statuses.get(bank, {})
            now = datetime.now(UTC)
            if not (force or check_only or is_due(previous, now)):
                continue
            path = PRIVATE / (bank + ".json")
            try:
                c = json.loads(path.read_text())
                if c.get("pfa_provider") == "pytr":
                    c["pfa_pytr_last_success"] = previous.get("last_success")
                result = run_bank(c, check_only=check_only)
            except OSError, ValueError:
                result = {"status": "missing_config"}
            print(json.dumps({"bank": bank, **result}), flush=True)
            if check_only:
                continue
            result["last_attempt"] = now.isoformat()
            result["last_success"] = (
                datetime.now(UTC).isoformat()
                if result["status"] == "success"
                else previous.get("last_success")
            )
            statuses[bank] = result
            write_private(status_path, json.dumps(statuses, indent=2).encode())


def install(downloads: Path) -> Path:
    """Stage the four exact exports and install a user-owned launch agent."""
    PRIVATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    PRIVATE.chmod(0o700)
    DEPLOY.chmod(0o700)
    for filename in (
        "bank_imports.py",
        "bank_import_worker.php",
        "trade_republic_capture.py",
        "trade_republic_sync.py",
        "trade_republic_worker.php",
    ):
        write_private(
            DEPLOY / filename, Path(__file__).with_name(filename).read_bytes()
        )
    for bank in BANKS:
        source = downloads / (bank + ".json")
        c = json.loads(source.read_text())
        if c.get("flow") != "eb" or not c.get("accounts"):
            raise ValueError("expected a saved bank API configuration")
        write_private(PRIVATE / (bank + ".json"), source.read_bytes())
    agent = Path.home() / "Library" / "LaunchAgents" / (LABEL + ".plist")
    agent.parent.mkdir(parents=True, exist_ok=True)
    body = {
        "Label": LABEL,
        "ProgramArguments": [
            str(Path(sys.executable).resolve()),
            str(DEPLOY / "bank_imports.py"),
            "run",
        ],
        "WorkingDirectory": str(DEPLOY),
        "StartInterval": 900,
        "RunAtLoad": True,
        "ProcessType": "Background",
        "EnvironmentVariables": {
            "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin")
        },
        "StandardOutPath": str(PRIVATE / "scheduler.log"),
        "StandardErrorPath": str(PRIVATE / "scheduler-error.log"),
    }
    write_private(agent, plistlib.dumps(body))
    return agent


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("install", "run", "check", "status"))
    parser.add_argument("--downloads", type=Path, default=Path.home() / "Downloads")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--bank", choices=BANKS)
    args = parser.parse_args()
    try:
        if args.action == "install":
            print(install(args.downloads))
        elif args.action == "status":
            print((PRIVATE / "status.json").read_text())
        else:
            run_cycle(
                force=args.force,
                check_only=args.action == "check",
                selected_bank=args.bank,
            )
    except OSError, ValueError, TypeError:
        print("Bank scheduler could not read its local configuration.", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
