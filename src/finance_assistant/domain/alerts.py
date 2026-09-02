"""Deterministic spending pace and safe-to-spend calculations."""

from decimal import Decimal

from finance_assistant.domain.models import (
    AlertSeverity,
    BudgetEvaluation,
    BudgetPolicy,
    BudgetWindow,
)


def evaluate_budget(
    window: BudgetWindow, *, policy: BudgetPolicy | None = None
) -> BudgetEvaluation:
    """Evaluate one budget window using explicit and explainable thresholds."""

    configured = policy or BudgetPolicy()
    budget = window.budget.amount
    spent = window.spent.amount
    effective_elapsed_days = max(window.elapsed_days, 1)
    elapsed_ratio = Decimal(effective_elapsed_days) / Decimal(window.total_days)
    spent_ratio = spent / budget
    pace_ratio = spent_ratio / elapsed_ratio
    forecast = spent / elapsed_ratio

    remaining_budget = max(budget - spent, Decimal(0))
    safe_to_spend = max(
        window.available_cash.amount
        - window.upcoming_committed.amount
        - window.planned_savings.amount
        - window.minimum_buffer.amount,
        Decimal(0),
    )

    severity = AlertSeverity.NONE
    reasons: list[str] = []

    protected_cash = (
        window.upcoming_committed.amount
        + window.planned_savings.amount
        + window.minimum_buffer.amount
    )
    if window.available_cash.amount <= protected_cash:
        severity = AlertSeverity.CRITICAL
        reasons.append("available cash does not cover commitments, savings, and buffer")
    if spent >= budget:
        severity = AlertSeverity.CRITICAL
        reasons.append("category budget is exhausted")
    elif forecast > budget:
        severity = max(severity, AlertSeverity.WARNING, key=_severity_rank)
        reasons.append("forecast spend exceeds the category budget")
    elif (
        spent_ratio >= configured.heads_up_spent_ratio
        or pace_ratio >= configured.heads_up_pace_ratio
    ):
        severity = max(severity, AlertSeverity.HEADS_UP, key=_severity_rank)
        reasons.append("spending is ahead of the configured budget pace")

    return BudgetEvaluation(
        category=window.category,
        currency=window.budget.currency,
        severity=severity,
        spent_ratio=spent_ratio,
        elapsed_ratio=elapsed_ratio,
        pace_ratio=pace_ratio,
        forecast_spend=forecast,
        remaining_budget=remaining_budget,
        safe_to_spend=safe_to_spend,
        reasons=tuple(reasons),
        policy_version=configured.version,
    )


def _severity_rank(severity: AlertSeverity) -> int:
    return {
        AlertSeverity.NONE: 0,
        AlertSeverity.HEADS_UP: 1,
        AlertSeverity.WARNING: 2,
        AlertSeverity.CRITICAL: 3,
    }[severity]
