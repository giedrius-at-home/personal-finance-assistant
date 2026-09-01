from datetime import date

from fastapi.testclient import TestClient

from finance_assistant.api import app

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
            "target": {"amount": "1200", "currency": "eur"},
            "current": {"amount": "400", "currency": "EUR"},
            "started_on": "2026-01-01",
            "target_date": "2026-04-01",
        },
    )

    assert response.status_code == 200
    assert response.json()["currency"] == "EUR"
    assert response.json()["remaining"] == "800"


def test_budget_endpoint_rejects_mixed_currencies() -> None:
    payload = {
        "category": "Dining",
        "budget": {"amount": "120", "currency": "EUR"},
        "spent": {"amount": "70", "currency": "USD"},
        "available_cash": {"amount": "1000", "currency": "EUR"},
        "upcoming_committed": {"amount": "500", "currency": "EUR"},
        "planned_savings": {"amount": "200", "currency": "EUR"},
        "minimum_buffer": {"amount": "100", "currency": "EUR"},
        "elapsed_days": 3,
        "total_days": 7,
    }

    response = client.post("/v1/budgets/evaluate", json=payload)

    assert response.status_code == 422


def test_budget_endpoint_returns_explainable_warning() -> None:
    payload = {
        "category": "Dining",
        "budget": {"amount": "120", "currency": "EUR"},
        "spent": {"amount": "70", "currency": "EUR"},
        "available_cash": {"amount": "1000", "currency": "EUR"},
        "upcoming_committed": {"amount": "500", "currency": "EUR"},
        "planned_savings": {"amount": "200", "currency": "EUR"},
        "minimum_buffer": {"amount": "100", "currency": "EUR"},
        "elapsed_days": 3,
        "total_days": 7,
    }

    response = client.post("/v1/budgets/evaluate", json=payload)

    assert response.status_code == 200
    assert response.json()["severity"] == "warning"
    assert response.json()["reasons"] == ["forecast spend exceeds the category budget"]
