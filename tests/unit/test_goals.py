from datetime import date
from decimal import Decimal

import pytest
from pydantic import ValidationError

from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import Currency, Money, SavingsGoal

EUR = Currency(code="EUR", minor_unit=2)


def make_goal(*, current: str = "300") -> SavingsGoal:
    return SavingsGoal(
        goal_id="holiday",
        name="Holiday",
        target=Money(amount=Decimal("900"), currency=EUR),
        current=Money(amount=Decimal(current), currency=EUR),
        started_on=date(2026, 1, 1),
        target_date=date(2026, 4, 1),
    )


def test_goal_evaluation_is_exact_and_explainable() -> None:
    result = evaluate_goal(make_goal(), as_of=date(2026, 1, 31))

    assert result.remaining == Decimal("600")
    assert result.expected_saved == Decimal("300")
    assert result.schedule_variance == Decimal("0")
    assert result.required_weekly_contribution == Decimal("70")
    assert result.days_remaining == 60
    assert result.on_track is True


def test_completed_goal_never_has_negative_remaining() -> None:
    result = evaluate_goal(make_goal(current="1000"), as_of=date(2026, 2, 1))

    assert result.completed is True
    assert result.remaining == 0
    assert result.required_weekly_contribution == 0


def test_overdue_goal_requires_remaining_amount_immediately() -> None:
    result = evaluate_goal(make_goal(), as_of=date(2026, 4, 2))

    assert result.overdue is True
    assert result.days_remaining == 0
    assert result.required_weekly_contribution == Decimal("600")


def test_goal_rejects_mixed_currencies() -> None:
    with pytest.raises(ValidationError, match="same currency"):
        SavingsGoal(
            goal_id="invalid",
            name="Invalid",
            target=Money(amount=Decimal("100"), currency=EUR),
            current=Money(
                amount=Decimal("10"), currency=Currency(code="USD", minor_unit=2)
            ),
            started_on=date(2026, 1, 1),
            target_date=date(2026, 2, 1),
        )


def test_goal_rejects_conflicting_precision_for_same_currency_code() -> None:
    with pytest.raises(ValidationError, match="same currency"):
        SavingsGoal(
            goal_id="invalid-precision",
            name="Invalid precision",
            target=Money(amount="100", currency=EUR),
            current=Money(
                amount="10",
                currency=Currency(code="EUR", minor_unit=3),
            ),
            started_on=date(2026, 1, 1),
            target_date=date(2026, 2, 1),
        )
