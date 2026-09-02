"""Savings-goal calculations with no model or I/O dependencies."""

from datetime import date, timedelta
from decimal import ROUND_CEILING, Decimal

from finance_assistant.domain.models import GoalEvaluation, SavingsGoal


def evaluate_goal(goal: SavingsGoal, *, as_of: date) -> GoalEvaluation:
    """Evaluate goal progress against a linear contribution schedule."""

    total_days = (goal.target_date - goal.started_on).days
    elapsed_days = min(max((as_of - goal.started_on).days, 0), total_days)
    days_remaining = max((goal.target_date - as_of).days, 0)

    remaining = max(goal.target.amount - goal.current.amount, Decimal(0))
    completed = remaining == 0
    overdue = as_of > goal.target_date and not completed

    elapsed_ratio = Decimal(elapsed_days) / Decimal(total_days)
    expected_saved = goal.target.amount * elapsed_ratio
    schedule_variance = goal.current.amount - expected_saved

    if completed:
        required_weekly = Decimal(0)
    elif days_remaining == 0:
        required_weekly = remaining
    else:
        required_weekly = remaining * Decimal(7) / Decimal(days_remaining)

    projected_completion: date | None = None
    observed_days = max((as_of - goal.started_on).days, 0)
    if completed:
        projected_completion = as_of
    elif observed_days > 0 and goal.current.amount > 0:
        daily_rate = goal.current.amount / Decimal(observed_days)
        projected_days = int(
            (remaining / daily_rate).to_integral_value(rounding=ROUND_CEILING)
        )
        try:
            projected_completion = as_of + timedelta(days=projected_days)
        except OverflowError:
            projected_completion = None

    return GoalEvaluation(
        goal_id=goal.goal_id,
        currency=goal.target.currency,
        completed=completed,
        overdue=overdue,
        on_track=completed or schedule_variance >= 0,
        remaining=remaining,
        expected_saved=expected_saved,
        schedule_variance=schedule_variance,
        required_weekly_contribution=required_weekly,
        days_remaining=days_remaining,
        projected_completion_date=projected_completion,
    )
