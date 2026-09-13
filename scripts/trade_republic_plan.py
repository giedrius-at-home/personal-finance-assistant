"""Fail-closed reconciliation of native Trade Republic events with Firefly.

This produces a private review plan, never ledger writes. A reviewed migration
mapping is required where the two providers do not expose a shared identifier.
"""

from __future__ import annotations

import json
import re
import uuid
from collections import Counter
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from typing import Any

from scripts.trade_republic_capture import PRIVATE, CaptureError, private_json


def normalized(value: str) -> str:
    return re.sub(r"\s+", " ", value.strip()).casefold()


def iban(value: str) -> str:
    return re.sub(r"\s", "", value).upper()


def signed_amount(transaction: dict[str, Any], target: str) -> Decimal:
    if str(transaction["source_id"]) == target:
        return -Decimal(transaction["amount"])
    if str(transaction["destination_id"]) == target:
        return Decimal(transaction["amount"])
    raise CaptureError("ledger_account_mismatch")


def reconcile(evidence: dict[str, Any], ledger: dict[str, Any]) -> dict[str, Any]:
    """Identify evidence-supported overlap; never guess from amount alone."""
    account = ledger["account"]
    attributes = account["attributes"]
    target = str(account["id"])
    cash = evidence["cash"]
    if (
        evidence.get("pagination_complete") is not True
        or len(cash) != 1
        or attributes["type"] != "asset"
        or attributes["active"] is not True
        or not iban(attributes["iban"])
        or iban(attributes["iban"]) != iban(evidence["settings"]["cashAccount"]["iban"])
        or cash[0]["currencyId"] != attributes["currency_code"]
    ):
        raise CaptureError("mapping_mismatch")
    transactions = [
        t
        for group in ledger["transactions"]
        for t in group["attributes"]["transactions"]
    ]
    rows: list[dict[str, Any]] = []
    used: set[str] = set()
    seen: dict[str, dict[str, Any]] = {}
    for stream in evidence["streams"].values():
        for event in stream:
            identity = event["id"]
            if identity in seen:
                if seen[identity] != event:
                    raise CaptureError("conflicting_event")
                continue
            seen[identity] = event
            detail = evidence["details"].get(identity)
            if not detail or detail.get("id") != identity:
                raise CaptureError("detail_identity_mismatch")
            if event.get("cashAccountNumber") not in (None, cash[0]["accountNumber"]):
                raise CaptureError("mapping_mismatch")
            stamp = datetime.fromisoformat(event["timestamp"])
            if stamp.tzinfo is None:
                raise CaptureError("missing_timezone")
            value = Decimal(str(event["amount"]["value"]))
            if (
                not value.is_finite()
                or event["amount"]["currency"] != cash[0]["currencyId"]
            ):
                raise CaptureError("currency_mismatch")
            headers = [
                s["data"].get("status")
                for s in detail["sections"]
                if s.get("type") == "header"
            ]
            status = event["status"]
            if headers != [status.lower()]:
                raise CaptureError("conflicting_status")
            row = {
                "event_id": identity,
                "date": stamp.isoformat(),
                "event_type": event["eventType"],
                "description": event["title"],
                "amount": str(value),
                "currency": event["amount"]["currency"],
            }
            if status == "CANCELED":
                row["decision"] = "exclude_canceled"
            elif status != "EXECUTED":
                row["decision"] = "review_status"
            elif stamp > datetime.fromisoformat(evidence["finished_at"]):
                row["decision"] = "review_future_event"
            elif event.get("hidden") or event.get("deleted"):
                row["decision"] = "review_hidden_or_deleted"
            elif value == 0 and event["eventType"] == "CARD_VERIFICATION":
                row["decision"] = "exclude_zero_verification"
            elif value == 0:
                row["decision"] = "review_zero_event"
            else:
                candidates = [
                    t
                    for t in transactions
                    if t["currency_code"] == row["currency"]
                    and signed_amount(t, target) == value
                    and abs(datetime.fromisoformat(t["date"]).date() - stamp.date())
                    <= timedelta(days=3)
                ]
                exact = [
                    t
                    for t in candidates
                    if normalized(t["description"]) == normalized(event["title"])
                ]
                if len(exact) == 1 and exact[0]["transaction_journal_id"] not in used:
                    row["decision"] = "existing_description_match"
                    row["journal_id"] = exact[0]["transaction_journal_id"]
                    used.add(row["journal_id"])
                elif candidates:
                    row["decision"] = "review_legacy_overlap"
                    row["candidates"] = [
                        {
                            k: t[k]
                            for k in (
                                "transaction_journal_id",
                                "date",
                                "type",
                                "description",
                                "amount",
                                "currency_code",
                            )
                        }
                        for t in candidates
                    ]
                else:
                    # Even apparently new records need accounting-type validation.
                    # Executed card events don't provide a separate bank booking date.
                    row["decision"] = "review_unmatched_event"
            rows.append(row)
    counts = Counter(row["decision"] for row in rows)
    return {
        "status": "reconciliation_required",
        "checked_at": datetime.now(UTC).isoformat(),
        "ledger_imported": False,
        "identity_verified": True,
        "cash_equals_ledger": Decimal(str(cash[0]["amount"]))
        == Decimal(attributes["current_balance"]),
        "counts": dict(counts),
        "rows": rows,
    }


def main() -> None:
    try:
        capture = max(PRIVATE.glob("capture-*.json"), key=lambda p: p.stat().st_mtime)
        evidence = json.loads(capture.read_text())
        ledger = json.loads((PRIVATE / "ledger-preview.json").read_text())
        plan = reconcile(evidence, ledger)
        private_json(PRIVATE / f"review-{uuid.uuid4().hex}.json", plan)
        print(json.dumps({k: v for k, v in plan.items() if k != "rows"}))
    except Exception as error:
        print(
            json.dumps(
                {
                    "status": str(error)
                    if isinstance(error, CaptureError)
                    else "preview_failed"
                }
            )
        )
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()
