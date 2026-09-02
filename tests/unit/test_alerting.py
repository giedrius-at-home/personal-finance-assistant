from datetime import UTC, date, datetime

from finance_assistant.domain.alerting import (
    AlertPolicy,
    budget_alert_candidate,
    goal_alert_candidate,
)
from finance_assistant.domain.canonical import (
    CanonicalSavingsGoal,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import (
    AlertSeverity,
    BudgetWindow,
    Currency,
    Money,
    SavingsGoal,
)

EUR = Currency(code="EUR", minor_unit=2)
NOW = datetime(2026, 2, 1, 18, tzinfo=UTC)


def money(amount: str) -> Money:
    return Money(amount=amount, currency=EUR)


def canonical_goal(*, current: str, target_date: date) -> CanonicalSavingsGoal:
    return CanonicalSavingsGoal(
        goal=SavingsGoal(
            goal_id="synthetic-goal",
            name="Synthetic goal",
            target=money("1200"),
            current=money(current),
            started_on=date(2026, 1, 1),
            target_date=target_date,
        ),
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.PIGGY_BANK,
            external_id="synthetic-piggy",
        ),
        priority=1,
        protected_amount=money(current),
    )


def test_goal_policy_covers_on_track_behind_and_overdue() -> None:
    policy = AlertPolicy()
    _, on_track = goal_alert_candidate(
        canonical_goal(current="500", target_date=date(2026, 4, 1)),
        as_of=NOW.date(),
        evaluated_at=NOW,
        policy=policy,
    )
    _, behind = goal_alert_candidate(
        canonical_goal(current="10", target_date=date(2026, 4, 1)),
        as_of=NOW.date(),
        evaluated_at=NOW,
        policy=policy,
    )
    _, overdue = goal_alert_candidate(
        canonical_goal(current="100", target_date=date(2026, 1, 31)),
        as_of=NOW.date(),
        evaluated_at=NOW,
        policy=policy,
    )

    assert on_track.severity is AlertSeverity.NONE
    assert behind.severity in {AlertSeverity.HEADS_UP, AlertSeverity.WARNING}
    assert overdue.severity is AlertSeverity.CRITICAL
    assert len({on_track.input_hash, behind.input_hash, overdue.input_hash}) == 3


def test_budget_candidate_uses_versioned_configurable_policy() -> None:
    policy = AlertPolicy()
    window = BudgetWindow(
        category="Synthetic dining",
        budget=money("100"),
        spent=money("70"),
        available_cash=money("1000"),
        upcoming_committed=money("100"),
        planned_savings=money("200"),
        minimum_buffer=money("100"),
        elapsed_days=20,
        total_days=30,
    )

    candidate = budget_alert_candidate(window, evaluated_at=NOW, policy=policy)

    assert candidate.policy_version == policy.version
    assert candidate.severity is AlertSeverity.WARNING
    assert candidate.reasons == ("forecast spend exceeds the category budget",)
    assert candidate.fingerprint != candidate.input_hash
