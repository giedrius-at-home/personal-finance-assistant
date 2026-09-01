from datetime import date
from decimal import Decimal

import pytest
from pydantic import ValidationError

from finance_assistant.domain.models import BudgetWindow, Currency, Money, SavingsGoal

EUR = Currency(code="EUR", minor_unit=2)


def euros(amount: str) -> Money:
    return Money(amount=Decimal(amount), currency=EUR)


@pytest.mark.parametrize("amount", ["NaN", "Infinity", "-Infinity"])
def test_money_rejects_non_finite_amounts(amount: str) -> None:
    with pytest.raises(ValidationError, match="finite"):
        euros(amount)


def test_money_rejects_invalid_currency() -> None:
    with pytest.raises(ValidationError, match="ASCII letters"):
        Currency(code="E1R", minor_unit=2)


def test_money_rejects_binary_float_and_implicit_rounding() -> None:
    with pytest.raises(ValidationError, match="decimal string or integer"):
        Money(amount=1.1, currency=EUR)

    with pytest.raises(ValidationError, match="precision exceeds"):
        Money(amount="1.001", currency=EUR)


def test_money_accepts_explicit_currency_scales_and_trailing_zeros() -> None:
    assert (
        Money(amount="100.00", currency=Currency(code="jpy", minor_unit=0)).amount
        == 100
    )
    assert Money(
        amount="1.2345", currency=Currency(code="UYW", minor_unit=4)
    ).amount == Decimal("1.2345")


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
