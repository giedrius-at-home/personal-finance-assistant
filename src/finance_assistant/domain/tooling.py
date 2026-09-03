"""Aggregate-only contracts exposed to an optional model tool host."""

from __future__ import annotations

from datetime import date
from decimal import Decimal

from pydantic import AwareDatetime

from finance_assistant.domain.models import AlertSeverity, Currency, StrictModel
from finance_assistant.domain.reporting import TaxonomySignal


class ToolCurrencyAmount(StrictModel):
    currency: Currency
    amount: Decimal


class ToolCashFlow(StrictModel):
    currency: Currency
    income: Decimal
    spending: Decimal
    net_cash_flow: Decimal
    transfer_count: int


class ToolGoalProgress(StrictModel):
    currency: Currency
    completed: bool
    overdue: bool
    on_track: bool
    remaining: Decimal
    required_weekly_contribution: Decimal
    projected_completion_date: date | None = None


class ToolAlertCount(StrictModel):
    severity: AlertSeverity
    count: int


class ToolTaxonomySignal(StrictModel):
    signal: TaxonomySignal
    category_count: int
    reason: str


class AggregateFinanceSnapshot(StrictModel):
    schema_version: str = "2026-09-v1"
    policy_version: str = "2026-09-v1"
    generated_at: AwareDatetime
    data_freshness: AwareDatetime
    report_basis: str
    complete_week: bool
    cash_flow: tuple[ToolCashFlow, ...]
    safe_to_spend: tuple[ToolCurrencyAmount, ...]
    active_alerts: tuple[ToolAlertCount, ...]
    goals: tuple[ToolGoalProgress, ...]
    net_worth_by_currency: tuple[ToolCurrencyAmount, ...]
    net_worth_total: ToolCurrencyAmount | None
    net_worth_reasons: tuple[str, ...]
    taxonomy: tuple[ToolTaxonomySignal, ...]
