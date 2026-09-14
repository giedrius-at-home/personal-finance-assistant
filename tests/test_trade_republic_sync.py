"""Prospective import boundaries and idempotency using synthetic records."""

from copy import deepcopy
from datetime import datetime

import pytest

from scripts.trade_republic_capture import CaptureError
from scripts.trade_republic_sync import build_plan, reference
from tests.test_trade_republic_plan import fixture


def setup():
    evidence, ledger = fixture()
    ledger["ledger_complete"] = True
    ledger["assets"] = []
    ledger["transactions"] = []
    evidence["finished_at"] = "2026-09-14T12:00:00+02:00"
    evidence["streams"]["timelineTransactions"][0]["timestamp"] = (
        "2026-09-14T10:00:00+02:00"
    )
    cutoff = datetime.fromisoformat("2026-09-14T00:00:00+02:00")
    return evidence, ledger, cutoff


def test_reconciled_history_is_not_reinterpreted():
    e, ledger, c = setup()
    old = e["streams"]["timelineTransactions"][0]
    old.update(timestamp="2026-09-13T23:59:59+02:00", eventType="OLD_UNKNOWN_TYPE")
    assert build_plan(e, ledger, c)["transactions"] == []
    assert build_plan(e, ledger, c)["historical_preserved"] == 1


def test_new_card_event_has_stable_reference_and_repeats_are_empty():
    e, ledger, c = setup()
    plan = build_plan(e, ledger, c)
    transaction = plan["transactions"][0]
    assert transaction["external_id"] == reference("SYNTHETIC", "native-one")
    assert transaction["type"] == "withdrawal"
    ledger["transactions"] = [
        {"attributes": {"transactions": [{**transaction, "destination_id": 2}]}}
    ]
    assert build_plan(e, ledger, c)["transactions"] == []


def test_changed_native_amount_is_not_silently_skipped():
    e, ledger, c = setup()
    t = build_plan(e, ledger, c)["transactions"][0]
    t["amount"] = "999"
    ledger["transactions"] = [{"attributes": {"transactions": [t]}}]
    with pytest.raises(CaptureError, match="existing_event_changed"):
        build_plan(e, ledger, c)


def test_only_post_cutoff_legacy_overlap_blocks_new_event():
    e, ledger, c = setup()
    _, old = fixture()
    ledger["transactions"] = old["transactions"]
    assert len(build_plan(e, ledger, c)["transactions"]) == 1
    ledger["transactions"][0]["attributes"]["transactions"][0]["date"] = (
        "2026-09-14T00:00:00+02:00"
    )
    with pytest.raises(CaptureError, match="legacy_overlap_after_cutoff"):
        build_plan(e, ledger, c)


@pytest.mark.parametrize("status", ["CANCELED", "PENDING"])
def test_nonexecuted_events_are_not_posted(status):
    e, ledger, c = setup()
    e["streams"]["timelineTransactions"][0]["status"] = status
    e["details"]["native-one"]["sections"][0]["data"]["status"] = status.lower()
    assert build_plan(e, ledger, c)["transactions"] == []


@pytest.mark.parametrize(
    "kind", ["TRADING_SAVINGSPLAN_EXECUTED", "SAVEBACK_AGGREGATE", "UNKNOWN"]
)
def test_investments_and_unknown_events_cannot_become_spending(kind):
    e, ledger, c = setup()
    e["streams"]["timelineTransactions"][0]["eventType"] = kind
    with pytest.raises(CaptureError, match="event_review_required"):
        build_plan(e, ledger, c)


def test_internal_bank_transfer_requires_existing_iban_mapping():
    e, ledger, c = setup()
    e["streams"]["timelineTransactions"][0]["eventType"] = "BANK_TRANSACTION_OUTGOING"
    with pytest.raises(CaptureError, match="transfer_mapping_required"):
        build_plan(e, ledger, c)
    ledger["assets"] = [
        {"id": "2", "attributes": {"iban": "SYNTHETIC-OTHER", "active": True}}
    ]
    e["details"]["native-one"]["sections"].append(
        {"type": "table", "data": [{"recipient": "SYNTHETIC-OTHER"}]}
    )
    assert build_plan(e, ledger, c)["transactions"][0]["type"] == "transfer"


def test_conflicting_currency_and_identity_fail_closed():
    e, ledger, c = setup()
    ledger["account"]["attributes"]["iban"] = "DIFFERENT"
    with pytest.raises(CaptureError, match="mapping_mismatch"):
        build_plan(e, ledger, c)


def test_repeated_native_feed_id_with_changed_content_fails():
    e, ledger, c = setup()
    other = deepcopy(e["streams"]["timelineTransactions"][0])
    other["title"] = "Changed"
    e["streams"]["timelineTransactions"].append(other)
    with pytest.raises(CaptureError, match="conflicting_event"):
        build_plan(e, ledger, c)
