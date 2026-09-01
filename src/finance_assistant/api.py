"""HTTP boundary for deterministic financial calculations."""

from datetime import date

from fastapi import FastAPI

from finance_assistant import __version__
from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import (
    BudgetEvaluation,
    BudgetWindow,
    GoalEvaluation,
    SavingsGoal,
)

app = FastAPI(
    title="Personal Finance Assistant",
    description="Local deterministic analytics for Firefly III and Ghostfolio.",
    version=__version__,
)


@app.get("/health", tags=["system"])
def health() -> dict[str, str]:
    """Return process health without revealing configuration or financial data."""

    return {"status": "ok", "version": __version__}


@app.post("/v1/goals/evaluate", tags=["goals"])
def goal_evaluation(goal: SavingsGoal, as_of: date | None = None) -> GoalEvaluation:
    """Evaluate savings progress as of a supplied date or today."""

    return evaluate_goal(goal, as_of=as_of or date.today())


@app.post("/v1/budgets/evaluate", tags=["budgets"])
def budget_evaluation(window: BudgetWindow) -> BudgetEvaluation:
    """Evaluate spending pace, forecast, and safe-to-spend amount."""

    return evaluate_budget(window)
