"""Invented end-to-end goal, alert, restart, escalation, and backup proof."""

from datetime import UTC, datetime
from pathlib import Path

from finance_assistant.alert_evaluation import AlertEvaluationService
from finance_assistant.domain.alerting import AlertDisposition, AlertPolicy
from finance_assistant.domain.models import AlertSeverity, Currency
from finance_assistant.ingestion.goals import build_savings_goals
from finance_assistant.ingestion.mapping import CurrencyRegistry
from finance_assistant.integrations.firefly import FireflyPiggyBank
from finance_assistant.notifications.macos import (
    NotificationMessage,
    NotificationOutcome,
)
from finance_assistant.notifications.service import AlertDispatcher
from finance_assistant.persistence.sqlite import SQLiteDerivedState

SYNTHETIC_NOTICE = "All identities and amounts in this module are invented."
NOW = datetime(2026, 9, 2, 18, tzinfo=UTC)


def piggy(target_date: str, revision: str) -> FireflyPiggyBank:
    return FireflyPiggyBank.model_validate(
        {
            "type": "piggy-banks",
            "id": "synthetic-emergency-fund",
            "attributes": {
                "name": "Synthetic emergency fund",
                "start_date": "2026-01-01T00:00:00+00:00",
                "target_date": target_date,
                "order": 1,
                "active": True,
                "currency_code": "EUR",
                "currency_decimal_places": 2,
                "target_amount": "1200.00",
                "current_amount": "100.00",
                "updated_at": revision,
            },
        }
    )


def test_phase3_goal_alert_survives_restart_escalates_and_backs_up(
    tmp_path: Path,
) -> None:
    path = tmp_path / "assistant.sqlite3"
    delivered: list[NotificationMessage] = []

    class SyntheticNotifier:
        def send(self, message: NotificationMessage) -> NotificationOutcome:
            delivered.append(message)
            return NotificationOutcome.DELIVERED

    registry = CurrencyRegistry((Currency(code="EUR", minor_unit=2),))
    first_goal = build_savings_goals(
        (piggy("2026-12-31T00:00:00+00:00", "2026-09-01T00:00:00+00:00"),),
        currencies=registry,
    ).goals[0]
    state = SQLiteDerivedState(path)
    state.migrate_up()
    state.upsert_goal(first_goal, observed_at=NOW)
    policy = AlertPolicy()
    service = AlertEvaluationService(
        state=state,
        dispatcher=AlertDispatcher(
            state=state, notifier=SyntheticNotifier(), policy=policy
        ),
        policy=policy,
        timezone_name="Europe/Amsterdam",
    )

    first = service.evaluate(now=NOW)[0]
    replay = service.evaluate(now=NOW)[0]
    assert SYNTHETIC_NOTICE.startswith("All identities")
    assert first.notify
    assert replay.disposition is AlertDisposition.DUPLICATE
    assert len(delivered) == 1
    state.close()

    restarted = SQLiteDerivedState(path)
    restarted.migrate_up()
    restarted_service = AlertEvaluationService(
        state=restarted,
        dispatcher=AlertDispatcher(
            state=restarted, notifier=SyntheticNotifier(), policy=policy
        ),
        policy=policy,
        timezone_name="Europe/Amsterdam",
    )
    assert (
        restarted_service.evaluate(now=NOW)[0].disposition is AlertDisposition.DUPLICATE
    )

    overdue_goal = build_savings_goals(
        (piggy("2026-08-31T00:00:00+00:00", "2026-09-02T00:00:00+00:00"),),
        currencies=registry,
    ).goals[0]
    restarted.upsert_goal(overdue_goal, observed_at=NOW)
    escalation = restarted_service.evaluate(now=NOW)[0]
    assert escalation.disposition is AlertDisposition.ESCALATED
    assert escalation.candidate.severity is AlertSeverity.CRITICAL
    assert len(delivered) == 2

    backup = tmp_path / "assistant-backup.sqlite3"
    restarted.backup_to(backup)
    restarted.close()
    restored = SQLiteDerivedState(backup)
    restored.migrate_up()
    restored_service = AlertEvaluationService(
        state=restored,
        dispatcher=AlertDispatcher(
            state=restored, notifier=SyntheticNotifier(), policy=policy
        ),
        policy=policy,
        timezone_name="Europe/Amsterdam",
    )
    assert (
        restored_service.evaluate(now=NOW)[0].disposition is AlertDisposition.DUPLICATE
    )
    restored.close()
