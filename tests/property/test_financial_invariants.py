from datetime import date, timedelta
from decimal import Decimal

from hypothesis import given
from hypothesis import strategies as st

from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import BudgetWindow, Money, SavingsGoal


def euros(amount: int) -> Money:
    return Money(amount=Decimal(amount), currency="EUR")


@given(
    target=st.integers(min_value=1, max_value=1_000_000),
    current=st.integers(min_value=0, max_value=1_500_000),
    elapsed=st.integers(min_value=0, max_value=365),
)
def test_goal_remaining_and_contribution_are_never_negative(
    target: int, current: int, elapsed: int
) -> None:
    started = date(2026, 1, 1)
    goal = SavingsGoal(
        goal_id="property-goal",
        name="Property goal",
        target=euros(target),
        current=euros(current),
        started_on=started,
        target_date=started + timedelta(days=366),
    )

    result = evaluate_goal(goal, as_of=started + timedelta(days=elapsed))

    assert result.remaining >= 0
    assert result.required_weekly_contribution >= 0


@given(
    available=st.integers(min_value=0, max_value=1_000_000),
    committed=st.integers(min_value=0, max_value=500_000),
    savings=st.integers(min_value=0, max_value=500_000),
    buffer=st.integers(min_value=0, max_value=500_000),
)
def test_safe_to_spend_is_never_negative(
    available: int, committed: int, savings: int, buffer: int
) -> None:
    result = evaluate_budget(
        BudgetWindow(
            category="Property category",
            budget=euros(1000),
            spent=euros(100),
            available_cash=euros(available),
            upcoming_committed=euros(committed),
            planned_savings=euros(savings),
            minimum_buffer=euros(buffer),
            elapsed_days=1,
            total_days=7,
        )
    )

    assert result.safe_to_spend >= 0
