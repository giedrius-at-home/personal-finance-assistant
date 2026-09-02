from datetime import UTC, datetime
from pathlib import Path

import pytest

from finance_assistant.persistence.sqlite import SQLiteDerivedState
from finance_assistant.scheduling import DailySchedule, EvaluationCoordinator


def test_daily_schedule_handles_local_time_and_sleep_catch_up() -> None:
    schedule = DailySchedule("Europe/Amsterdam", hour=18)

    before = schedule.latest_due(datetime(2026, 9, 2, 12, tzinfo=UTC))
    after = schedule.latest_due(datetime(2026, 9, 2, 20, tzinfo=UTC))

    assert before.isoformat() == "2026-09-01T18:00:00+02:00"
    assert after.isoformat() == "2026-09-02T18:00:00+02:00"


@pytest.mark.anyio
async def test_coordinator_runs_one_catch_up_and_event_evaluations(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    evaluations: list[datetime] = []

    async def evaluate(now: datetime) -> None:
        evaluations.append(now)

    coordinator = EvaluationCoordinator(
        state=state,
        evaluator=evaluate,
        schedule=DailySchedule("Europe/Amsterdam"),
    )
    now = datetime(2026, 9, 2, 20, tzinfo=UTC)

    assert await coordinator.run_scheduled(now)
    assert not await coordinator.run_scheduled(now)
    await coordinator.run_event(now)

    assert evaluations == [now, now]
    state.close()
