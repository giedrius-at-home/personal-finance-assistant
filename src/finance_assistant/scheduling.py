"""Timezone-aware daily and event-triggered evaluation coordination."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from datetime import datetime, time, timedelta
from typing import Protocol
from zoneinfo import ZoneInfo


class ScheduleState(Protocol):
    def claim_evaluation(
        self,
        job_name: str,
        *,
        now: datetime,
        due_after: datetime,
        lease_seconds: int = 900,
    ) -> bool: ...

    def complete_evaluation(self, job_name: str, *, completed_at: datetime) -> None: ...


class DailySchedule:
    """Resolve the most recent 18:00 local slot for bounded catch-up."""

    def __init__(self, timezone_name: str, *, hour: int = 18) -> None:
        if not 0 <= hour <= 23:
            raise ValueError("hour must be between 0 and 23")
        self.timezone = ZoneInfo(timezone_name)
        self.hour = hour

    def latest_due(self, now: datetime) -> datetime:
        if now.tzinfo is None:
            raise ValueError("now must be timezone-aware")
        local = now.astimezone(self.timezone)
        due = datetime.combine(local.date(), time(hour=self.hour), tzinfo=self.timezone)
        if local < due:
            due -= timedelta(days=1)
        return due


class EvaluationCoordinator:
    """Run at most one missed scheduled slot; events rely on alert deduplication."""

    def __init__(
        self,
        *,
        state: ScheduleState,
        evaluator: Callable[[datetime], Awaitable[None]],
        schedule: DailySchedule,
        job_name: str = "goals-and-budgets",
    ) -> None:
        self._state = state
        self._evaluator = evaluator
        self._schedule = schedule
        self._job_name = job_name

    async def run_scheduled(self, now: datetime) -> bool:
        due = self._schedule.latest_due(now)
        if not self._state.claim_evaluation(self._job_name, now=now, due_after=due):
            return False
        await self._evaluator(now)
        self._state.complete_evaluation(self._job_name, completed_at=now)
        return True

    async def run_event(self, now: datetime) -> None:
        await self._evaluator(now)
