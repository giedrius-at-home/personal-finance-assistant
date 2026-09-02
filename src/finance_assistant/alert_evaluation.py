"""Evaluate stored goals and supplied budget windows through one alert boundary."""

from __future__ import annotations

from datetime import datetime
from typing import Protocol
from zoneinfo import ZoneInfo

from finance_assistant.domain.alerting import (
    AlertDecision,
    AlertPolicy,
    budget_alert_candidate,
    goal_alert_candidate,
)
from finance_assistant.domain.canonical import CanonicalSavingsGoal
from finance_assistant.domain.models import BudgetWindow
from finance_assistant.notifications.service import AlertDispatcher


class GoalState(Protocol):
    def list_goals(self) -> tuple[CanonicalSavingsGoal, ...]: ...


class AlertEvaluationService:
    """Create and dispatch candidates from deterministic aggregate inputs."""

    def __init__(
        self,
        *,
        state: GoalState,
        dispatcher: AlertDispatcher,
        policy: AlertPolicy,
        timezone_name: str,
    ) -> None:
        self._state = state
        self._dispatcher = dispatcher
        self._policy = policy
        self._timezone = ZoneInfo(timezone_name)

    def evaluate(
        self, *, now: datetime, budgets: tuple[BudgetWindow, ...] = ()
    ) -> tuple[AlertDecision, ...]:
        if now.tzinfo is None:
            raise ValueError("now must be timezone-aware")
        decisions: list[AlertDecision] = []
        as_of = now.astimezone(self._timezone).date()
        for goal in self._state.list_goals():
            _, candidate = goal_alert_candidate(
                goal,
                as_of=as_of,
                evaluated_at=now,
                policy=self._policy,
            )
            decisions.append(self._dispatcher.handle(candidate))
        for budget in budgets:
            candidate = budget_alert_candidate(
                budget, evaluated_at=now, policy=self._policy
            )
            decisions.append(self._dispatcher.handle(candidate))
        return tuple(decisions)
