"""Synthetic identity, status and overlap checks; no personal data."""

from copy import deepcopy

import pytest

from scripts.trade_republic_capture import CaptureError
from scripts.trade_republic_plan import reconcile


def fixture():
    event = {
        "id": "native-one",
        "timestamp": "2026-09-12T12:00:00+02:00",
        "eventType": "CARD_TRANSACTION",
        "title": "Synthetic merchant",
        "status": "EXECUTED",
        "cashAccountNumber": "synthetic-cash",
        "amount": {"value": -12.34, "currency": "EUR"},
        "hidden": False,
        "deleted": False,
    }
    evidence = {
        "pagination_complete": True,
        "finished_at": "2026-09-13T12:00:00Z",
        "settings": {"cashAccount": {"iban": "SYNTHETIC"}},
        "cash": [
            {"accountNumber": "synthetic-cash", "currencyId": "EUR", "amount": 100}
        ],
        "streams": {"timelineTransactions": [event]},
        "details": {
            "native-one": {
                "id": "native-one",
                "sections": [{"type": "header", "data": {"status": "executed"}}],
            }
        },
    }
    ledger = {
        "account": {
            "id": "1",
            "attributes": {
                "iban": "SYNTHETIC",
                "type": "asset",
                "active": True,
                "currency_code": "EUR",
                "current_balance": "100",
            },
        },
        "transactions": [
            {
                "attributes": {
                    "transactions": [
                        {
                            "transaction_journal_id": "ledger-one",
                            "source_id": "1",
                            "destination_id": "2",
                            "amount": "12.34",
                            "currency_code": "EUR",
                            "date": "2026-09-13T00:00:00+02:00",
                            "type": "withdrawal",
                            "description": "Synthetic merchant",
                        }
                    ]
                }
            }
        ],
    }
    return evidence, ledger


def test_description_match_does_not_claim_completed_import():
    evidence, ledger = fixture()
    plan = reconcile(evidence, ledger)
    assert plan["counts"] == {"existing_description_match": 1}
    assert plan["status"] == "reconciliation_required"
    assert plan["ledger_imported"] is False


def test_canceled_and_zero_verification_are_not_cash_postings():
    for status, event_type, value, expected in [
        ("CANCELED", "CARD_TRANSACTION", -12.34, "exclude_canceled"),
        ("EXECUTED", "CARD_VERIFICATION", 0, "exclude_zero_verification"),
        ("PENDING", "CARD_TRANSACTION", -12.34, "review_status"),
        ("EXECUTED", "UNRECOGNIZED_EVENT", 0, "review_zero_event"),
    ]:
        evidence, ledger = fixture()
        e = evidence["streams"]["timelineTransactions"][0]
        e.update(status=status, eventType=event_type)
        e["amount"]["value"] = value
        evidence["details"]["native-one"]["sections"][0]["data"]["status"] = (
            status.lower()
        )
        assert reconcile(evidence, ledger)["counts"] == {expected: 1}


@pytest.mark.parametrize(
    "change", ["iban", "currency", "event_account", "status", "pages"]
)
def test_conflicts_fail_closed(change):
    evidence, ledger = fixture()
    if change == "iban":
        ledger["account"]["attributes"]["iban"] = "DIFFERENT"
    elif change == "currency":
        evidence["cash"][0]["currencyId"] = "USD"
    elif change == "event_account":
        evidence["streams"]["timelineTransactions"][0]["cashAccountNumber"] = (
            "different"
        )
    elif change == "status":
        evidence["details"]["native-one"]["sections"][0]["data"]["status"] = "canceled"
    else:
        evidence["pagination_complete"] = False
    with pytest.raises(CaptureError):
        reconcile(evidence, ledger)


def test_duplicate_ledger_candidates_require_review():
    evidence, ledger = fixture()
    duplicate = deepcopy(ledger["transactions"][0])
    duplicate["attributes"]["transactions"][0]["transaction_journal_id"] = "ledger-two"
    ledger["transactions"].append(duplicate)
    plan = reconcile(evidence, ledger)
    assert plan["counts"] == {"review_legacy_overlap": 1}
    assert len(plan["rows"][0]["candidates"]) == 2


def test_one_ledger_record_cannot_cover_two_events():
    evidence, ledger = fixture()
    event = deepcopy(evidence["streams"]["timelineTransactions"][0])
    event["id"] = "native-two"
    evidence["streams"]["timelineTransactions"].append(event)
    detail = deepcopy(evidence["details"]["native-one"])
    detail["id"] = "native-two"
    evidence["details"]["native-two"] = detail
    plan = reconcile(evidence, ledger)
    assert plan["counts"] == {
        "existing_description_match": 1,
        "review_legacy_overlap": 1,
    }


def test_amount_alone_never_establishes_overlap():
    evidence, ledger = fixture()
    ledger["transactions"][0]["attributes"]["transactions"][0]["description"] = (
        "Other shop"
    )
    assert reconcile(evidence, ledger)["counts"] == {"review_legacy_overlap": 1}
