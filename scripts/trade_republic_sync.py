"""Prospective daily native cash imports; older reconciled history is immutable."""

from __future__ import annotations

import asyncio
import fcntl
import hashlib
import json
import logging
import os
import re
import shutil
import subprocess  # nosec B404
import sys
import uuid
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from pathlib import Path
from typing import Any

try:
    from scripts.trade_republic_capture import (
        PRIVATE,
        CaptureError,
        bound_http_requests,
        capture,
        private_json,
    )
except ModuleNotFoundError:
    from trade_republic_capture import (  # type: ignore[import-not-found,no-redef]
        PRIVATE,
        CaptureError,
        bound_http_requests,
        capture,
        private_json,
    )


def reference(account_iban: str, event_id: str) -> str:
    return (
        "pfa-pytr:"
        + hashlib.sha256((account_iban + "\0" + event_id).encode()).hexdigest()
    )


def normalized_iban(value: str) -> str:
    return re.sub(r"\s+", "", value).upper()


def build_plan(
    evidence: dict[str, Any], ledger: dict[str, Any], cutoff: datetime
) -> dict[str, Any]:
    account = ledger["account"]
    attrs = account["attributes"]
    target = int(account["id"])
    cash = evidence["cash"]
    source = normalized_iban(evidence["settings"]["cashAccount"]["iban"])
    if (
        cutoff.tzinfo is None
        or evidence.get("pagination_complete") is not True
        or ledger.get("ledger_complete") is not True
        or len(cash) != 1
        or not source
        or source != normalized_iban(attrs["iban"])
        or attrs["type"] != "asset"
        or attrs["active"] is not True
        or cash[0]["currencyId"] != attrs["currency_code"]
    ):
        raise CaptureError("mapping_mismatch")
    existing = [
        t for g in ledger["transactions"] for t in g["attributes"]["transactions"]
    ]
    by_id = {t.get("external_id"): t for t in existing if t.get("external_id")}
    posted = []
    excluded = 0
    historical = 0
    seen: dict[str, dict[str, Any]] = {}
    for events in evidence["streams"].values():
        for event in events:
            stamp = datetime.fromisoformat(event["timestamp"])
            if stamp.tzinfo is None:
                raise CaptureError("missing_timezone")
            if stamp < cutoff:
                historical += 1
                continue
            identity = event["id"]
            if identity in seen:
                if seen[identity] != event:
                    raise CaptureError("conflicting_event")
                continue
            seen[identity] = event
            detail = evidence["details"].get(identity)
            if not detail or detail.get("id") != identity:
                raise CaptureError("detail_identity_mismatch")
            status = event["status"]
            headers = [
                s["data"].get("status")
                for s in detail["sections"]
                if s.get("type") == "header"
            ]
            if headers != [status.lower()]:
                raise CaptureError("conflicting_status")
            if event.get("cashAccountNumber") not in (None, cash[0]["accountNumber"]):
                raise CaptureError("mapping_mismatch")
            value = Decimal(str(event["amount"]["value"]))
            currency = event["amount"]["currency"]
            if not value.is_finite() or currency != cash[0]["currencyId"]:
                raise CaptureError("currency_mismatch")
            if event.get("deleted") or event.get("hidden"):
                raise CaptureError("event_review_required")
            if status == "CANCELED":
                if reference(source, identity) in by_id:
                    raise CaptureError("existing_event_changed")
                excluded += 1
                continue
            if status == "PENDING":
                excluded += 1
                continue
            if status != "EXECUTED":
                raise CaptureError("event_review_required")
            if stamp > datetime.fromisoformat(evidence["finished_at"]):
                raise CaptureError("event_review_required")
            kind = event["eventType"]
            if kind == "CARD_VERIFICATION" and value == 0:
                excluded += 1
                continue
            if value == 0:
                raise CaptureError("event_review_required")
            t = {
                "date": stamp.isoformat(),
                "amount": str(abs(value)),
                "currency_code": currency,
                "description": event["title"],
                "external_id": reference(source, identity),
                "notes": (
                    "Native Trade Republic executed timestamp; "
                    "imported after the owner-approved reconciliation cutoff."
                ),
            }
            if kind == "CARD_TRANSACTION" and value < 0:
                # Bank top-ups and ATM movements must not be silently called spending.
                body = json.dumps(detail, ensure_ascii=False).casefold()
                if any(
                    word in body
                    for word in ("cash withdrawal", "geldopname", "atm withdrawal")
                ):
                    raise CaptureError("transfer_mapping_required")
                if any(
                    word in event["title"].casefold()
                    for word in ("revolut", "swedbank", "abn amro", "trade republic")
                ):
                    raise CaptureError("transfer_mapping_required")
                t.update(
                    type="withdrawal", source_id=target, destination_name=event["title"]
                )
            elif kind in ("INTEREST_PAYOUT", "INTEREST_PAYOUT_CREATED") and value > 0:
                t.update(
                    type="deposit",
                    destination_id=target,
                    source_name="Trade Republic interest",
                )
            elif kind in ("BANK_TRANSACTION_INCOMING", "BANK_TRANSACTION_OUTGOING"):
                text = normalized_iban(json.dumps(detail, ensure_ascii=False))
                own = [
                    a
                    for a in ledger["assets"]
                    if int(a["id"]) != target
                    and a["attributes"]["active"]
                    and normalized_iban(a["attributes"].get("iban") or "")
                    and normalized_iban(a["attributes"]["iban"]) in text
                ]
                if len(own) != 1:
                    raise CaptureError("transfer_mapping_required")
                t.update(
                    type="transfer",
                    source_id=int(own[0]["id"]) if value > 0 else target,
                    destination_id=target if value > 0 else int(own[0]["id"]),
                )
            else:
                raise CaptureError("event_review_required")
            previous = by_id.get(t["external_id"])
            if previous:
                if (
                    Decimal(previous["amount"]) != abs(value)
                    or previous["currency_code"] != currency
                    or previous["type"] != t["type"]
                ):
                    raise CaptureError("existing_event_changed")
                continue
            # Never guess cross-provider overlap for a post-cutoff event.
            for old in existing:
                if datetime.fromisoformat(old["date"]) < cutoff or (
                    old.get("external_id") or ""
                ).startswith("pfa-pytr:"):
                    continue
                signed = Decimal(old["amount"]) * (
                    1 if int(old["destination_id"]) == target else -1
                )
                if (
                    signed == value
                    and old["currency_code"] == currency
                    and abs(
                        (datetime.fromisoformat(old["date"]).date() - stamp.date()).days
                    )
                    <= 3
                ):
                    raise CaptureError("legacy_overlap_after_cutoff")
            posted.append(t)
    return {
        "transactions": posted,
        "excluded": excluded,
        "historical_preserved": historical,
        "iban_sha256": hashlib.sha256(source.encode()).hexdigest(),
        "bank_balance_equals_ledger": Decimal(str(cash[0]["amount"]))
        == Decimal(attrs["current_balance"]),
    }


def ledger_request(container: str, payload: dict[str, Any]) -> dict[str, Any]:
    worker = (
        Path(__file__)
        .with_name("trade_republic_worker.php")
        .read_text()
        .removeprefix("<?php")
    )
    result = subprocess.run(  # noqa: S603  # nosec B603
        [
            shutil.which("docker") or "docker",
            "exec",
            "-i",
            "-e",
            "LOG_CHANNEL=null",
            container,
            "php",
            "-r",
            worker,
        ],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        timeout=180,
    )
    if result.returncode:
        raise CaptureError("ledger_operation_failed")
    value = json.loads(result.stdout)
    if not isinstance(value, dict):
        raise CaptureError("ledger_operation_failed")
    return value


def main() -> None:
    os.umask(0o077)
    logging.disable(logging.CRITICAL)
    PRIVATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    phase = "setup"
    try:
        from importlib.metadata import version

        from pytr.api import TradeRepublicApi  # type: ignore[import-not-found]

        if version("pytr") != "0.4.10":
            raise CaptureError("unsupported_pytr_version")
        args = json.load(sys.stdin)
        configuration = args["configuration"]
        cutoff = datetime.fromisoformat(configuration["pfa_pytr_cutoff"])
        target = list(configuration["accounts"].values())
        if len(target) != 1 or type(target[0]) is not int:
            raise CaptureError("mapping_mismatch")
        discovered = subprocess.run(  # noqa: S603  # nosec B603
            [
                shutil.which("docker") or "docker",
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
        )
        container = discovered.stdout.strip()
        if not re.fullmatch("[a-f0-9]{12,64}", container):
            raise CaptureError("runtime_unavailable")
        with (PRIVATE / "session.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            tr = TradeRepublicApi(
                phone_no="session-only",
                pin="unused",
                locale="en",
                save_cookies=True,
                cookies_file=PRIVATE / "cookies.txt",
                use_v2_login=True,
            )
            bound_http_requests(tr)
            phase = "authentication"
            if not tr.resume_websession():
                raise CaptureError("reauthorization_required")
            phase = "capture"
            last_success = datetime.fromisoformat(
                configuration.get("pfa_pytr_last_success") or cutoff.isoformat()
            )
            scan_since = max(
                cutoff, min(datetime.now(UTC) - timedelta(days=30), last_success)
            )
            evidence = asyncio.run(
                asyncio.wait_for(capture(tr, not_before=scan_since), timeout=600)
            )
            tr.save_websession()
            private_json(PRIVATE / f"capture-{uuid.uuid4().hex}.json", evidence)
        phase = "ledger"
        ledger = ledger_request(
            container, {"operation": "snapshot", "target": target[0]}
        )
        phase = "plan"
        plan = build_plan(evidence, ledger, cutoff)
        private_json(PRIVATE / f"prospective-plan-{uuid.uuid4().hex}.json", plan)
        created = 0
        if not args.get("check_only", True):
            for transaction in plan["transactions"]:
                result = ledger_request(
                    container,
                    {
                        "operation": "post",
                        "target": target[0],
                        "iban_sha256": plan["iban_sha256"],
                        "transaction": transaction,
                    },
                )
                if result.get("status") not in ("created", "existing"):
                    raise CaptureError("ledger_operation_failed")
                created += result["status"] == "created"
        print(
            json.dumps(
                {
                    "status": "verified" if args.get("check_only", True) else "success",
                    "converted": len(plan["transactions"]),
                    "created": created,
                    "historical_preserved": plan["historical_preserved"],
                    "bank_balance_equals_ledger": plan["bank_balance_equals_ledger"],
                }
            )
        )
    except Exception as error:
        print(
            json.dumps(
                {
                    "status": str(error)
                    if isinstance(error, CaptureError)
                    else "failed",
                    "phase": phase,
                    "failure_type": type(error).__name__,
                }
            )
        )


if __name__ == "__main__":
    main()
