"""Scrub the owner dashboard into its documented aggregate-only tool view."""

from __future__ import annotations

from datetime import datetime

from finance_assistant.dashboard import DashboardService
from finance_assistant.domain.tooling import (
    AggregateFinanceSnapshot,
    ToolAlertCount,
    ToolCashFlow,
    ToolCurrencyAmount,
    ToolGoalProgress,
    ToolTaxonomySignal,
)
from finance_assistant.persistence.sqlite import SQLiteDerivedState


class AggregateToolService:
    """Expose report aggregates without source, account, goal, or merchant IDs."""

    def __init__(self, state: SQLiteDerivedState) -> None:
        self._dashboard = DashboardService(state)

    def snapshot(self, *, now: datetime) -> AggregateFinanceSnapshot:
        view = self._dashboard.view(now=now)
        net_total = None
        if (
            view.net_worth.total is not None
            and view.net_worth.total_currency is not None
        ):
            net_total = ToolCurrencyAmount(
                currency=view.net_worth.total_currency,
                amount=view.net_worth.total,
            )
        return AggregateFinanceSnapshot(
            generated_at=now,
            data_freshness=view.generated_at,
            report_basis=view.weekly.basis,
            complete_week=view.weekly.complete_week,
            cash_flow=tuple(
                ToolCashFlow(
                    currency=item.currency,
                    income=item.income,
                    spending=item.spending,
                    net_cash_flow=item.net_cash_flow,
                    transfer_count=item.transfer_count,
                )
                for item in view.weekly.currencies
            ),
            safe_to_spend=tuple(
                ToolCurrencyAmount(currency=item.currency, amount=item.amount)
                for item in view.weekly.safe_to_spend
            ),
            active_alerts=tuple(
                ToolAlertCount(severity=item.severity, count=item.count)
                for item in view.weekly.active_alerts
            ),
            goals=tuple(
                ToolGoalProgress(
                    currency=item.currency,
                    completed=item.completed,
                    overdue=item.overdue,
                    on_track=item.on_track,
                    remaining=item.remaining,
                    required_weekly_contribution=item.required_weekly_contribution,
                    projected_completion_date=item.projected_completion_date,
                )
                for item in view.weekly.goals
            ),
            net_worth_by_currency=tuple(
                ToolCurrencyAmount(currency=item.currency, amount=item.amount)
                for item in view.net_worth.currency_totals
            ),
            net_worth_total=net_total,
            net_worth_reasons=view.net_worth.reasons,
            taxonomy=tuple(
                ToolTaxonomySignal(
                    signal=item.signal,
                    category_count=len(item.categories),
                    reason=item.reason,
                )
                for item in view.taxonomy
            ),
        )
