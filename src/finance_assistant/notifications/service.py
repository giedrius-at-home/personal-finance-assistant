"""Apply durable alert decisions before invoking a notification adapter."""

from __future__ import annotations

from datetime import datetime
from typing import Protocol

from finance_assistant.domain.alerting import AlertCandidate, AlertDecision, AlertPolicy
from finance_assistant.notifications.macos import (
    NotificationMessage,
    NotificationOutcome,
)


class AlertState(Protocol):
    def process_alert(
        self, candidate: AlertCandidate, *, policy: AlertPolicy
    ) -> AlertDecision: ...

    def record_notification(
        self,
        candidate: AlertCandidate,
        *,
        outcome: str,
        attempted_at: datetime,
    ) -> None: ...


class Notifier(Protocol):
    def send(self, message: NotificationMessage) -> NotificationOutcome: ...


class AlertDispatcher:
    def __init__(
        self, *, state: AlertState, notifier: Notifier, policy: AlertPolicy
    ) -> None:
        self._state = state
        self._notifier = notifier
        self._policy = policy

    def handle(self, candidate: AlertCandidate) -> AlertDecision:
        """Persist policy state before attempting content-bearing delivery."""

        decision = self._state.process_alert(candidate, policy=self._policy)
        if not decision.notify:
            return decision
        message = NotificationMessage(
            title=f"Personal finance: {candidate.severity.value.replace('_', ' ')}",
            body=f"{candidate.subject_id}: {candidate.reasons[0]}",
        )
        outcome = self._notifier.send(message)
        self._state.record_notification(
            candidate,
            outcome=outcome.value,
            attempted_at=candidate.evaluated_at,
        )
        return decision
