"""Read-only pytr evidence capture; cookies only, never saved login credentials.

Run with: uv run --with pytr==0.4.10 python scripts/trade_republic_capture.py login
The scheduler must not treat an evidence capture as a completed ledger import.
"""

from __future__ import annotations

import argparse
import asyncio
import fcntl
import getpass
import json
import logging
import os
import sys
import uuid
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

PRIVATE = (
    Path.home()
    / "Library/Application Support/Personal Finance Assistant"
    / "Trade Republic Sync"
)


class CaptureError(Exception):
    """Sanitized failure; upstream payloads must never be printed."""


def private_json(path: Path, value: Any) -> None:
    # Exclusive creation preserves the only evidence even on repeated runs.
    with os.fdopen(
        os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w"
    ) as f:
        json.dump(value, f)


async def response(tr: Any, subscription: str) -> Any:
    received, _, payload = await asyncio.wait_for(tr.recv(), timeout=45)
    if received != subscription:
        raise CaptureError("unexpected_subscription")
    await tr.unsubscribe(subscription)
    return payload


async def pages(tr: Any, kind: str) -> list[dict[str, Any]]:
    """Walk to exhaustion, rejecting loops, conflicting IDs and truncated feeds."""
    after = None
    cursors: set[str] = set()
    events: dict[str, dict[str, Any]] = {}
    for _ in range(200):
        payload = await response(tr, await tr.subscribe({"type": kind, "after": after}))
        if (
            not isinstance(payload, dict)
            or not isinstance(payload.get("items"), list)
            or not isinstance(payload.get("cursors"), dict)
        ):
            raise CaptureError("invalid_page")
        added = 0
        for item in payload["items"]:
            if not isinstance(item, dict) or not isinstance(item.get("id"), str):
                raise CaptureError("invalid_event")
            identity = item["id"]
            if not identity or (identity in events and events[identity] != item):
                raise CaptureError("conflicting_event")
            added += identity not in events
            events[identity] = item
        after = payload["cursors"].get("after")
        if after is None:
            return list(events.values())
        if not isinstance(after, str) or not after or after in cursors or not added:
            raise CaptureError("provider_pagination_error")
        cursors.add(after)
    raise CaptureError("provider_pagination_error")


def recent(events: list[dict[str, Any]], cutoff: datetime) -> list[dict[str, Any]]:
    result = []
    for event in events:
        stamp = datetime.fromisoformat(event["timestamp"])
        if stamp.tzinfo is None:
            raise CaptureError("missing_timezone")
        if stamp >= cutoff:
            result.append(event)
    return result


async def capture(tr: Any) -> dict[str, Any]:
    started = datetime.now(UTC)
    settings = tr.settings()
    cash = await response(tr, await tr.cash())
    streams = {}
    details = {}
    for kind in ("timelineTransactions", "timelineActivityLog"):
        streams[kind] = recent(await pages(tr, kind), started - timedelta(days=30))
        for event in streams[kind]:
            action = event.get("action") or {}
            if action.get("type") == "timelineDetail":
                if action.get("payload") != event["id"]:
                    raise CaptureError("detail_identity_mismatch")
                if event["id"] not in details:
                    details[event["id"]] = await response(
                        tr, await tr.timeline_detail_v2(event["id"])
                    )
    return {
        "version": 1,
        "pytr_version": "0.4.10",
        "started_at": started.isoformat(),
        "finished_at": datetime.now(UTC).isoformat(),
        "settings": settings,
        "cash": cash,
        "streams": streams,
        "details": details,
        "pagination_complete": True,
        "ledger_imported": False,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("login", "capture"))
    args = parser.parse_args()
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    PRIVATE.chmod(0o700)
    logging.disable(logging.CRITICAL)
    lock = (PRIVATE / "session.lock").open("a")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        lock.close()
        print('{"status": "capture_already_running"}')
        raise SystemExit(1) from None
    try:
        from importlib.metadata import version

        from pytr.api import TradeRepublicApi  # type: ignore[import-not-found]

        if version("pytr") != "0.4.10":
            raise CaptureError("unsupported_pytr_version")
        # Nonempty placeholders prevent pytr from reading its global credentials file.
        # Resume uses cookies; no login endpoint is called in capture mode.
        tr = TradeRepublicApi(
            phone_no="session-only",
            pin="unused",
            locale="en",
            save_cookies=True,
            cookies_file=PRIVATE / "cookies.txt",
            use_v2_login=True,
        )
        if not tr.resume_websession():
            if args.action != "login":
                raise CaptureError("reauthorization_required")
            if not sys.stdin.isatty():
                raise CaptureError("interactive_terminal_required")
            tr.phone_no = getpass.getpass(
                "Phone number (international format, hidden): "
            )
            tr.pin = getpass.getpass("Trade Republic PIN (hidden, never saved): ")
            tr.initiate_weblogin()
            code = (
                getpass.getpass("Authenticator code (hidden): ")
                if tr.weblogin_needs_authenticator
                else None
            )
            if code is None:
                print("Confirm this login in your Trade Republic app.", flush=True)
            tr.complete_weblogin(code)
            tr.phone_no, tr.pin = "session-only", "unused"
        (PRIVATE / "cookies.txt").chmod(0o600)
        print(
            "Session ready. Capturing read-only evidence; this can take a few minutes.",
            flush=True,
        )
        evidence = asyncio.run(asyncio.wait_for(capture(tr), timeout=900))
        private_json(PRIVATE / f"capture-{uuid.uuid4().hex}.json", evidence)
        tr.save_websession()
        print(
            json.dumps(
                {
                    "status": "captured",
                    "ledger_imported": False,
                    "events": {k: len(v) for k, v in evidence["streams"].items()},
                }
            )
        )
    except Exception as error:
        print(
            json.dumps(
                {
                    "status": str(error)
                    if isinstance(error, CaptureError)
                    else "capture_failed"
                }
            )
        )
        raise SystemExit(1) from None
    finally:
        lock.close()


if __name__ == "__main__":
    main()
