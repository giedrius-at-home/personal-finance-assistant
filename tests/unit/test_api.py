import hashlib
import hmac
import json
from datetime import date
from pathlib import Path
from uuid import uuid4

from fastapi.testclient import TestClient

from finance_assistant.api import app, create_app
from finance_assistant.integrations.firefly_webhook import MAX_WEBHOOK_BYTES

client = TestClient(app)


def test_health_does_not_expose_configuration() -> None:
    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {"status": "ok", "version": "0.1.0"}


def test_goal_endpoint() -> None:
    response = client.post(
        "/v1/goals/evaluate",
        params={"as_of": date(2026, 2, 1).isoformat()},
        json={
            "goal_id": "emergency-fund",
            "name": "Emergency fund",
            "target": {
                "amount": "1200",
                "currency": {"code": "eur", "minor_unit": 2},
            },
            "current": {
                "amount": "400",
                "currency": {"code": "EUR", "minor_unit": 2},
            },
            "started_on": "2026-01-01",
            "target_date": "2026-04-01",
        },
    )

    assert response.status_code == 200
    assert response.json()["currency"] == {"code": "EUR", "minor_unit": 2}
    assert response.json()["remaining"] == "800"


def test_budget_endpoint_rejects_mixed_currencies() -> None:
    payload = {
        "category": "Dining",
        "budget": {"amount": "120", "currency": {"code": "EUR", "minor_unit": 2}},
        "spent": {"amount": "70", "currency": {"code": "USD", "minor_unit": 2}},
        "available_cash": {
            "amount": "1000",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "upcoming_committed": {
            "amount": "500",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "planned_savings": {
            "amount": "200",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "minimum_buffer": {
            "amount": "100",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "elapsed_days": 3,
        "total_days": 7,
    }

    response = client.post("/v1/budgets/evaluate", json=payload)

    assert response.status_code == 422


def test_budget_endpoint_returns_explainable_warning() -> None:
    payload = {
        "category": "Dining",
        "budget": {"amount": "120", "currency": {"code": "EUR", "minor_unit": 2}},
        "spent": {"amount": "70", "currency": {"code": "EUR", "minor_unit": 2}},
        "available_cash": {
            "amount": "1000",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "upcoming_committed": {
            "amount": "500",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "planned_savings": {
            "amount": "200",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "minimum_buffer": {
            "amount": "100",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "elapsed_days": 3,
        "total_days": 7,
    }

    response = client.post("/v1/budgets/evaluate", json=payload)

    assert response.status_code == 200
    assert response.json()["severity"] == "warning"
    assert response.json()["reasons"] == ["forecast spend exceeds the category budget"]


def test_financial_endpoint_rejects_binary_float_and_excess_precision() -> None:
    payload = {
        "goal_id": "precision-test",
        "name": "Precision test",
        "target": {
            "amount": 100.0,
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "current": {
            "amount": "10.001",
            "currency": {"code": "EUR", "minor_unit": 2},
        },
        "started_on": "2026-01-01",
        "target_date": "2026-04-01",
    }

    response = client.post("/v1/goals/evaluate", json=payload)

    assert response.status_code == 422
    messages = [error["msg"] for error in response.json()["detail"]]
    assert any("decimal string or integer" in message for message in messages)
    assert any("precision exceeds" in message for message in messages)


def test_webhook_is_unavailable_without_a_configured_secret() -> None:
    response = client.post("/v1/webhooks/firefly", content=b"{}")

    assert response.status_code == 503
    assert response.json() == {"detail": "webhook receiver is not configured"}


def webhook_body() -> bytes:
    return json.dumps(
        {
            "uuid": str(uuid4()),
            "user_id": 1,
            "user_group_id": 1,
            "trigger": "STORE_TRANSACTION",
            "response": "TRANSACTIONS",
            "url": "http://gateway.test/v1/webhooks/firefly",
            "version": "v0",
            "content": [],
        },
        separators=(",", ":"),
    ).encode()


def webhook_signature(body: bytes, timestamp: int, secret: str) -> str:
    digest = hmac.new(
        secret.encode(),
        str(timestamp).encode() + b"." + body,
        hashlib.sha3_256,
    ).hexdigest()
    return f"t={timestamp},v1={digest}"


def test_webhook_accepts_once_and_acknowledges_a_duplicate() -> None:
    now = 1_800_000_000
    secret = "w" * 32
    webhook_client = TestClient(
        create_app(webhook_secret=secret, webhook_clock=lambda: now)
    )
    body = webhook_body()
    headers = {"Signature": webhook_signature(body, now, secret)}

    accepted = webhook_client.post(
        "/v1/webhooks/firefly", content=body, headers=headers
    )
    duplicate = webhook_client.post(
        "/v1/webhooks/firefly", content=body, headers=headers
    )

    assert accepted.status_code == 202
    assert accepted.json() == {"status": "accepted"}
    assert duplicate.status_code == 202
    assert duplicate.json() == {"status": "duplicate"}


def test_webhook_duplicate_is_retained_across_app_restart(tmp_path: Path) -> None:
    now = 1_800_000_000
    secret = "w" * 32
    database = tmp_path / "derived.sqlite3"
    body = webhook_body()
    headers = {"Signature": webhook_signature(body, now, secret)}

    with TestClient(
        create_app(
            webhook_secret=secret,
            webhook_clock=lambda: now,
            database_path=database,
        )
    ) as first_client:
        assert first_client.post(
            "/v1/webhooks/firefly", content=body, headers=headers
        ).json() == {"status": "accepted"}
    with TestClient(
        create_app(
            webhook_secret=secret,
            webhook_clock=lambda: now,
            database_path=database,
        )
    ) as restarted_client:
        assert restarted_client.post(
            "/v1/webhooks/firefly", content=body, headers=headers
        ).json() == {"status": "duplicate"}


def test_webhook_rejects_before_parsing_and_never_echoes_the_body() -> None:
    now = 1_800_000_000
    secret = "w" * 32
    webhook_client = TestClient(
        create_app(webhook_secret=secret, webhook_clock=lambda: now)
    )
    body = b'sensitive invalid body {"account":"synthetic"}'

    unauthenticated = webhook_client.post(
        "/v1/webhooks/firefly",
        content=body,
        headers={"Signature": webhook_signature(b"different", now, secret)},
    )
    signed_invalid = webhook_client.post(
        "/v1/webhooks/firefly",
        content=body,
        headers={"Signature": webhook_signature(body, now, secret)},
    )

    assert unauthenticated.status_code == 401
    assert signed_invalid.status_code == 422
    assert body.decode() not in unauthenticated.text
    assert body.decode() not in signed_invalid.text
    assert secret not in unauthenticated.text


def test_webhook_rejects_an_oversized_body() -> None:
    webhook_client = TestClient(
        create_app(webhook_secret="w" * 32, webhook_clock=lambda: 1_800_000_000)
    )

    response = webhook_client.post(
        "/v1/webhooks/firefly", content=b"x" * (MAX_WEBHOOK_BYTES + 1)
    )

    assert response.status_code == 413


def test_webhook_stops_reading_an_oversized_chunked_body() -> None:
    webhook_client = TestClient(
        create_app(webhook_secret="w" * 32, webhook_clock=lambda: 1_800_000_000)
    )

    response = webhook_client.post(
        "/v1/webhooks/firefly",
        content=iter((b"x" * MAX_WEBHOOK_BYTES, b"x")),
    )

    assert response.status_code == 413
