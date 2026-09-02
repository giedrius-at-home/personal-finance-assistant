"""Versioned conversion of exact evaluations into content-free alert candidates."""

from __future__ import annotations

import hashlib
import json
from datetime import date, datetime
from enum import StrEnum

from pydantic import AwareDatetime, Field, model_validator

from finance_assistant.domain.alerts import evaluate_budget
from finance_assistant.domain.canonical import CanonicalSavingsGoal
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import (
    AlertSeverity,
    BudgetPolicy,
    BudgetWindow,
    GoalEvaluation,
    StrictModel,
)


class AlertSubjectKind(StrEnum):
    GOAL = "goal"
    BUDGET = "budget"


class AlertDisposition(StrEnum):
    DISPATCH = "dispatch"
    ESCALATED = "escalated"
    SUPPRESSED = "suppressed"
    DUPLICATE = "duplicate"
    RESOLVED = "resolved"
    INACTIVE = "inactive"


class AlertPolicy(StrictModel):
    """Conservative initial policy approved through decision issue #19."""

    version: str = Field(default="2026-09-v1", min_length=1, max_length=64)
    budget: BudgetPolicy = BudgetPolicy()
    heads_up_cooldown_seconds: int = Field(
        default=7 * 24 * 60 * 60, ge=1, le=31 * 24 * 60 * 60
    )
    warning_cooldown_seconds: int = Field(
        default=3 * 24 * 60 * 60, ge=1, le=31 * 24 * 60 * 60
    )
    critical_cooldown_seconds: int = Field(
        default=24 * 60 * 60, ge=1, le=31 * 24 * 60 * 60
    )

    def cooldown_seconds(self, severity: AlertSeverity) -> int:
        return {
            AlertSeverity.NONE: 0,
            AlertSeverity.HEADS_UP: self.heads_up_cooldown_seconds,
            AlertSeverity.WARNING: self.warning_cooldown_seconds,
            AlertSeverity.CRITICAL: self.critical_cooldown_seconds,
        }[severity]


class AlertCandidate(StrictModel):
    subject_kind: AlertSubjectKind
    subject_id: str = Field(min_length=1, max_length=128)
    severity: AlertSeverity
    reasons: tuple[str, ...]
    policy_version: str
    evidence_json: str = Field(min_length=2, max_length=16_384)
    input_hash: str = Field(pattern="^[0-9a-f]{64}$")
    fingerprint: str = Field(pattern="^[0-9a-f]{64}$")
    evaluated_at: AwareDatetime

    @model_validator(mode="after")
    def validate_evidence_hash(self) -> AlertCandidate:
        try:
            evidence = json.loads(self.evidence_json)
        except json.JSONDecodeError as error:
            raise ValueError("evidence_json must contain valid JSON") from error
        canonical = json.dumps(evidence, sort_keys=True, separators=(",", ":"))
        if canonical != self.evidence_json:
            raise ValueError("evidence_json must use canonical JSON encoding")
        if hashlib.sha256(self.evidence_json.encode()).hexdigest() != self.input_hash:
            raise ValueError("input_hash does not match evidence_json")
        return self


class AlertDecision(StrictModel):
    candidate: AlertCandidate
    disposition: AlertDisposition
    notify: bool


def goal_alert_candidate(
    goal: CanonicalSavingsGoal,
    *,
    as_of: date,
    evaluated_at: datetime,
    policy: AlertPolicy,
) -> tuple[GoalEvaluation, AlertCandidate]:
    evaluation = evaluate_goal(goal.goal, as_of=as_of)
    if evaluation.completed or evaluation.on_track:
        severity = AlertSeverity.NONE
        reasons = ("goal is completed" if evaluation.completed else "goal is on track",)
    elif evaluation.overdue:
        severity = AlertSeverity.CRITICAL
        reasons = ("incomplete goal is past its target date",)
    elif -evaluation.schedule_variance >= evaluation.required_weekly_contribution:
        severity = AlertSeverity.WARNING
        reasons = ("goal is behind by at least one required weekly contribution",)
    else:
        severity = AlertSeverity.HEADS_UP
        reasons = ("goal is behind its contribution schedule",)
    candidate = _candidate(
        kind=AlertSubjectKind.GOAL,
        subject_id=goal.goal.goal_id,
        severity=severity,
        reasons=reasons,
        policy=policy,
        evaluated_at=evaluated_at,
        evidence=evaluation.model_dump(mode="json"),
    )
    return evaluation, candidate


def budget_alert_candidate(
    window: BudgetWindow,
    *,
    evaluated_at: datetime,
    policy: AlertPolicy,
) -> AlertCandidate:
    evaluation = evaluate_budget(window, policy=policy.budget)
    reasons = evaluation.reasons or ("budget is within configured limits",)
    return _candidate(
        kind=AlertSubjectKind.BUDGET,
        subject_id=window.category,
        severity=evaluation.severity,
        reasons=reasons,
        policy=policy,
        evaluated_at=evaluated_at,
        evidence=evaluation.model_dump(mode="json"),
    )


def _candidate(
    *,
    kind: AlertSubjectKind,
    subject_id: str,
    severity: AlertSeverity,
    reasons: tuple[str, ...],
    policy: AlertPolicy,
    evaluated_at: datetime,
    evidence: dict[str, object],
) -> AlertCandidate:
    encoded = json.dumps(evidence, sort_keys=True, separators=(",", ":"))
    input_hash = hashlib.sha256(encoded.encode()).hexdigest()
    fingerprint = hashlib.sha256(
        f"{kind.value}\0{subject_id}\0{severity.value}\0{policy.version}\0{input_hash}".encode()
    ).hexdigest()
    return AlertCandidate(
        subject_kind=kind,
        subject_id=subject_id,
        severity=severity,
        reasons=reasons,
        policy_version=policy.version,
        evidence_json=encoded,
        input_hash=input_hash,
        fingerprint=fingerprint,
        evaluated_at=evaluated_at,
    )
