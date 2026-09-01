from decimal import Decimal

from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.models import AlertSeverity, BudgetWindow, Money


def money(amount: str) -> Money:
    return Money(amount=Decimal(amount), currency="EUR")


def window(**overrides: object) -> BudgetWindow:
    values: dict[str, object] = {
        "category": "Dining",
        "budget": money("120"),
        "spent": money("30"),
        "available_cash": money("1000"),
        "upcoming_committed": money("500"),
        "planned_savings": money("200"),
        "minimum_buffer": money("100"),
        "elapsed_days": 3,
        "total_days": 7,
    }
    values.update(overrides)
    return BudgetWindow.model_validate(values)


def test_normal_budget_has_safe_to_spend() -> None:
    result = evaluate_budget(window())

    assert result.severity is AlertSeverity.NONE
    assert result.safe_to_spend == Decimal("200")
    assert result.remaining_budget == Decimal("90")


def test_forecast_over_budget_warns() -> None:
    result = evaluate_budget(window(spent=money("70")))

    assert result.severity is AlertSeverity.WARNING
    assert result.forecast_spend > Decimal("120")
    assert "forecast spend exceeds" in result.reasons[0]


def test_high_spent_ratio_is_a_heads_up() -> None:
    result = evaluate_budget(window(spent=money("80"), elapsed_days=6))

    assert result.severity is AlertSeverity.HEADS_UP
    assert result.reasons == ("spending is ahead of the configured budget pace",)


def test_exhausted_budget_is_critical() -> None:
    result = evaluate_budget(window(spent=money("120")))

    assert result.severity is AlertSeverity.CRITICAL
    assert result.remaining_budget == 0


def test_protected_cash_is_critical() -> None:
    result = evaluate_budget(window(available_cash=money("800")))

    assert result.severity is AlertSeverity.CRITICAL
    assert result.safe_to_spend == 0


def test_spend_on_first_day_does_not_divide_by_zero() -> None:
    result = evaluate_budget(window(spent=money("10"), elapsed_days=0))

    assert result.pace_ratio.quantize(Decimal("0.0001")) == Decimal("0.5833")
    assert result.forecast_spend.quantize(Decimal("0.01")) == Decimal("70.00")
    assert result.severity is AlertSeverity.NONE
