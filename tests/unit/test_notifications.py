import hashlib
import subprocess
from datetime import UTC, datetime
from pathlib import Path

from finance_assistant.domain.alerting import (
    AlertCandidate,
    AlertDisposition,
    AlertPolicy,
    AlertSubjectKind,
)
from finance_assistant.domain.models import AlertSeverity
from finance_assistant.notifications.macos import (
    MacOSNotifier,
    NotificationMessage,
    NotificationOutcome,
)
from finance_assistant.notifications.service import AlertDispatcher
from finance_assistant.persistence.sqlite import SQLiteDerivedState

NOW = datetime(2026, 9, 2, 18, tzinfo=UTC)


def test_macos_adapter_passes_content_as_arguments_not_script() -> None:
    commands: list[list[str]] = []
    notifier = MacOSNotifier(
        enabled=True,
        system="Darwin",
        runner=commands.append,
    )
    message = NotificationMessage(
        title='Synthetic "title"', body="Synthetic body; do shell things"
    )

    assert notifier.send(message) is NotificationOutcome.DELIVERED
    command = commands[0]
    assert command[0] == "/usr/bin/osascript"
    assert message.title in command[-2:]
    assert message.body in command[-2:]
    assert all(message.title not in part for part in command[1:-2])
    assert all(message.body not in part for part in command[1:-2])


def test_macos_adapter_is_opt_in_and_failure_safe() -> None:
    message = NotificationMessage(title="Synthetic", body="Synthetic")
    assert MacOSNotifier().send(message) is NotificationOutcome.DISABLED
    assert (
        MacOSNotifier(enabled=True, system="Linux").send(message)
        is NotificationOutcome.UNAVAILABLE
    )

    def fail(_command: list[str]) -> None:
        raise subprocess.TimeoutExpired("osascript", 10)

    assert (
        MacOSNotifier(enabled=True, system="Darwin", runner=fail).send(message)
        is NotificationOutcome.FAILED
    )


def test_dispatcher_invokes_adapter_only_after_durable_decision(tmp_path: Path) -> None:
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    delivered: list[NotificationMessage] = []

    class FakeNotifier:
        def send(self, message: NotificationMessage) -> NotificationOutcome:
            delivered.append(message)
            return NotificationOutcome.DELIVERED

    dispatcher = AlertDispatcher(
        state=state, notifier=FakeNotifier(), policy=AlertPolicy()
    )
    candidate = AlertCandidate(
        subject_kind=AlertSubjectKind.BUDGET,
        subject_id="Synthetic dining",
        severity=AlertSeverity.WARNING,
        reasons=("synthetic forecast exceeds budget",),
        policy_version="2026-09-v1",
        evidence_json="{}",
        input_hash=hashlib.sha256(b"{}").hexdigest(),
        fingerprint="b" * 64,
        evaluated_at=NOW,
    )

    first = dispatcher.handle(candidate)
    replay = dispatcher.handle(candidate)

    assert first.disposition is AlertDisposition.DISPATCH
    assert replay.disposition is AlertDisposition.DUPLICATE
    assert len(delivered) == 1
    state.close()
