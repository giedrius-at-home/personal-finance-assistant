import hashlib
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from pathlib import Path
from uuid import uuid4

import pytest

from finance_assistant.domain.alerting import (
    AlertCandidate,
    AlertDisposition,
    AlertPolicy,
    AlertSubjectKind,
)
from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalSavingsGoal,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import AlertSeverity, Currency, Money, SavingsGoal
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)

NOW = datetime(2026, 1, 2, tzinfo=UTC)
EUR = Currency(code="EUR", minor_unit=2)


def account(balance: str = "125.50", revision: str = "r1") -> CanonicalAccount:
    return CanonicalAccount(
        account_id="firefly:synthetic-account",
        name="Synthetic cash",
        balance=Money(amount=balance, currency=EUR),
        canonical_owner=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.ACCOUNT,
            external_id="synthetic-account",
            source_revision=revision,
            source_updated_at=NOW,
        ),
    )


def test_repeated_import_is_idempotent_and_update_preserves_lineage(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    initial = account()
    updated = account("130.75", "r2")

    assert state.upsert(initial, observed_at=NOW)
    assert not state.upsert(initial, observed_at=NOW)
    assert state.upsert(updated, observed_at=NOW)
    assert state.list_accounts() == (updated,)
    assert state.version_count(initial.canonical_owner) == 2
    state.close()


def test_source_cannot_silently_change_canonical_owner(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    initial = account()
    state.upsert(initial, observed_at=NOW)

    with pytest.raises(PersistenceConflictError, match="different canonical owner"):
        state.upsert(
            initial.model_copy(update={"account_id": "firefly:different"}),
            observed_at=NOW,
        )
    state.close()


def test_read_only_connection_reads_but_sqlite_rejects_writes(tmp_path: Path) -> None:
    path = tmp_path / "state.sqlite3"
    writable = SQLiteDerivedState(path)
    writable.migrate_up()
    writable.upsert(account(), observed_at=NOW)
    writable.close()

    read_only = SQLiteDerivedState(path, read_only=True)
    assert read_only.list_accounts() == (account(),)
    with pytest.raises(sqlite3.OperationalError, match="readonly"):
        read_only.upsert(account("200"), observed_at=NOW)
    read_only.close()


def test_durable_delivery_claim_is_atomic_and_survives_restart(
    tmp_path: Path,
) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteDerivedState(path)
    state.migrate_up()
    delivery_id = uuid4()

    with ThreadPoolExecutor(max_workers=8) as executor:
        results = list(
            executor.map(
                lambda _: state.claim(delivery_id, now=1_800_000_000), range(32)
            )
        )
    assert results.count(True) == 1
    state.close()

    reopened = SQLiteDerivedState(path)
    reopened.migrate_up()
    assert not reopened.claim(delivery_id, now=1_800_000_100)
    reopened.close()


def test_projection_delete_retains_lineage_and_privacy_purge_removes_it(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    record = account()
    state.upsert(record, observed_at=NOW)

    assert state.delete_current("account", record.account_id)
    assert state.list_accounts() == ()
    assert state.version_count(record.canonical_owner) == 1
    state.purge_source(record.canonical_owner)
    assert state.version_count(record.canonical_owner) == 0
    state.close()


def test_migration_is_reversible_only_before_state_exists(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "empty.sqlite3")
    state.migrate_up()
    state.migrate_down()
    database = sqlite3.connect(state.path)
    try:
        tables = database.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        ).fetchall()
    finally:
        database.close()
    assert tables == []
    state.migrate_up()
    state.upsert(account(), observed_at=NOW)
    with pytest.raises(PersistenceConflictError, match="populated"):
        state.migrate_down()
    state.close()


def test_backup_is_consistent_and_refuses_overwrite(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    state.upsert(account(), observed_at=NOW)
    destination = tmp_path / "backup.sqlite3"

    state.backup_to(destination)
    backup = sqlite3.connect(destination)
    try:
        assert backup.execute("SELECT COUNT(*) FROM canonical_records").fetchone() == (
            1,
        )
    finally:
        backup.close()
    with pytest.raises(FileExistsError):
        state.backup_to(destination)
    state.close()


def canonical_goal(current: str = "100", revision: str = "r1") -> CanonicalSavingsGoal:
    source = SourceReference(
        system=SourceSystem.FIREFLY_III,
        entity=SourceEntity.PIGGY_BANK,
        external_id="synthetic-piggy",
        source_revision=revision,
        source_updated_at=NOW,
    )
    return CanonicalSavingsGoal(
        goal=SavingsGoal(
            goal_id="synthetic-goal",
            name="Synthetic goal",
            target=Money(amount="1000", currency=EUR),
            current=Money(amount=current, currency=EUR),
            started_on=NOW.date(),
            target_date=NOW.date().replace(year=2027),
        ),
        source=source,
        priority=1,
        protected_amount=Money(amount=current, currency=EUR),
    )


def candidate(
    severity: AlertSeverity,
    fingerprint: str,
    *,
    at: datetime = NOW,
) -> AlertCandidate:
    return AlertCandidate(
        subject_kind=AlertSubjectKind.GOAL,
        subject_id="synthetic-goal",
        severity=severity,
        reasons=("synthetic reason",),
        policy_version="test-v1",
        evidence_json="{}",
        input_hash=hashlib.sha256(b"{}").hexdigest(),
        fingerprint=fingerprint * 64,
        evaluated_at=at,
    )


def test_goals_are_idempotent_and_preserve_versions(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    first = canonical_goal()
    update = canonical_goal("125", "r2")

    assert state.upsert_goal(first, observed_at=NOW)
    assert not state.upsert_goal(first, observed_at=NOW)
    assert state.upsert_goal(update, observed_at=NOW)
    assert state.list_goals() == (update,)
    assert state.goal_version_count(first.source) == 2
    state.close()


def test_alert_lifecycle_deduplicates_cools_down_escalates_and_resolves(
    tmp_path: Path,
) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    policy = AlertPolicy(
        heads_up_cooldown_seconds=100,
        warning_cooldown_seconds=100,
        critical_cooldown_seconds=100,
    )

    first = state.process_alert(candidate(AlertSeverity.HEADS_UP, "1"), policy=policy)
    duplicate = state.process_alert(
        candidate(AlertSeverity.HEADS_UP, "1"), policy=policy
    )
    suppressed = state.process_alert(
        candidate(AlertSeverity.HEADS_UP, "2"), policy=policy
    )
    escalated = state.process_alert(
        candidate(AlertSeverity.WARNING, "3"), policy=policy
    )
    resolved = state.process_alert(candidate(AlertSeverity.NONE, "4"), policy=policy)

    assert first.disposition is AlertDisposition.DISPATCH
    assert duplicate.disposition is AlertDisposition.DUPLICATE
    assert suppressed.disposition is AlertDisposition.SUPPRESSED
    assert escalated.disposition is AlertDisposition.ESCALATED
    assert resolved.disposition is AlertDisposition.RESOLVED
    assert [result.notify for result in (first, duplicate, suppressed, escalated)] == [
        True,
        False,
        False,
        True,
    ]
    state.close()


def test_scheduler_claim_has_restart_safe_lease_and_completion(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    due = NOW
    assert state.claim_evaluation("daily", now=NOW, due_after=due)
    assert not state.claim_evaluation("daily", now=NOW, due_after=due)
    state.complete_evaluation("daily", completed_at=NOW)
    assert not state.claim_evaluation("daily", now=NOW, due_after=due)
    state.close()


def test_failed_delivery_retries_after_one_hour(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    policy = AlertPolicy()
    alert = candidate(AlertSeverity.WARNING, "9")
    assert state.process_alert(alert, policy=policy).notify
    state.record_notification(alert, outcome="failed", attempted_at=NOW)

    before_retry = alert.model_copy(
        update={"evaluated_at": NOW + timedelta(minutes=59)}
    )
    after_retry = alert.model_copy(update={"evaluated_at": NOW + timedelta(hours=1)})
    assert not state.process_alert(before_retry, policy=policy).notify
    assert state.process_alert(after_retry, policy=policy).notify
    state.close()


def test_goal_and_alert_privacy_deletion(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    goal = canonical_goal()
    state.upsert_goal(goal, observed_at=NOW)
    alert = candidate(AlertSeverity.WARNING, "8")
    state.process_alert(alert, policy=AlertPolicy())
    state.record_notification(alert, outcome="delivered", attempted_at=NOW)

    assert state.delete_goal(goal.goal.goal_id)
    assert state.goal_version_count(goal.source) == 1
    state.purge_source(goal.source)
    state.purge_alert_subject("goal", alert.subject_id)
    assert state.goal_version_count(goal.source) == 0
    assert state.list_goals() == ()
    state.close()


def test_concurrent_alert_candidate_dispatches_once(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    alert = candidate(AlertSeverity.WARNING, "7")

    with ThreadPoolExecutor(max_workers=8) as executor:
        decisions = list(
            executor.map(
                lambda _: state.process_alert(alert, policy=AlertPolicy()), range(32)
            )
        )

    assert sum(decision.notify for decision in decisions) == 1
    state.close()
