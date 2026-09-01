from datetime import date
from decimal import Decimal

import pytest
from pydantic import ValidationError

from finance_assistant.domain.models import BudgetWindow, Money, SavingsGoal


def euros(amount: str) -> Money:
    return Money(amount=Decimal(amount), currency="EUR")


@pytest.mark.parametrize("amount", ["NaN", "Infinity", "-Infinity"])
def test_money_rejects_non_finite_amounts(amount: str) -> None:
    with pytest.raises(ValidationError, match="finite"):
        euros(amount)


def test_money_rejects_invalid_currency() -> None:
    with pytest.raises(ValidationError, match="three letters"):
        Money(amount=Decimal("10"), currency="E1R")


@pytest.mark.parametrize(
    ("current", "started", "target_date", "message"),
    [
        ("-1", date(2026, 1, 1), date(2026, 2, 1), "cannot be negative"),
        ("1", date(2026, 2, 1), date(2026, 2, 1), "must be later"),
    ],
)
def test_goal_rejects_invalid_state(
    current: str, started: date, target_date: date, message: str
) -> None:
    with pytest.raises(ValidationError, match=message):
        SavingsGoal(
            goal_id="invalid",
            name="Invalid",
            target=euros("100"),
            current=euros(current),
            started_on=started,
            target_date=target_date,
        )


@pytest.mark.parametrize(
    ("overrides", "message"),
    [
        ({"budget": euros("0")}, "must be positive"),
        ({"spent": euros("-1")}, "cannot be negative"),
        ({"elapsed_days": 8}, "cannot exceed"),
    ],
)
def test_budget_rejects_invalid_state(
    overrides: dict[str, object], message: str
) -> None:
    values: dict[str, object] = {
        "category": "Dining",
        "budget": euros("100"),
        "spent": euros("10"),
        "available_cash": euros("1000"),
        "upcoming_committed": euros("500"),
        "planned_savings": euros("200"),
        "minimum_buffer": euros("100"),
        "elapsed_days": 2,
        "total_days": 7,
    }
    values.update(overrides)

    with pytest.raises(ValidationError, match=message):
        BudgetWindow.model_validate(values)
