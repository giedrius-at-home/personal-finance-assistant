"""Deterministic weekly, net-worth, and taxonomy reporting contracts."""

from __future__ import annotations

import hashlib
import json
import re
from collections import defaultdict
from datetime import date, datetime, timedelta
from decimal import Decimal
from enum import StrEnum
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import AwareDatetime, Field, model_validator

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalSavingsGoal,
    CanonicalTransaction,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.categorisation import normalize_merchant
from finance_assistant.domain.goals import evaluate_goal
from finance_assistant.domain.models import (
    AlertSeverity,
    Currency,
    GoalEvaluation,
    Money,
    StrictModel,
)


class CategoryAssignment(StrictModel):
    transaction_id: str = Field(min_length=1, max_length=128)
    category: str = Field(min_length=1, max_length=128)


class CategoryTotal(StrictModel):
    category: str
    amount: Decimal
    transaction_count: int = Field(ge=0)


class CurrencyWeek(StrictModel):
    currency: Currency
    income: Decimal
    spending: Decimal
    net_cash_flow: Decimal
    transfer_count: int = Field(ge=0)
    categories: tuple[CategoryTotal, ...]


class CurrencyAmount(StrictModel):
    currency: Currency
    amount: Decimal


class AlertCount(StrictModel):
    severity: AlertSeverity
    count: int = Field(ge=0)


class WeeklyReport(StrictModel):
    report_version: str = "2026-09-v1"
    basis: str = "observed"
    timezone: str
    week_start: date
    through_date: date
    complete_week: bool
    currencies: tuple[CurrencyWeek, ...]
    safe_to_spend: tuple[CurrencyAmount, ...]
    active_alerts: tuple[AlertCount, ...]
    goals: tuple[GoalEvaluation, ...]
    source_transaction_count: int = Field(ge=0)

    @property
    def evidence_hash(self) -> str:
        encoded = json.dumps(
            self.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        )
        return hashlib.sha256(encoded.encode()).hexdigest()


def weekly_report(
    transactions: tuple[CanonicalTransaction, ...],
    *,
    assignments: tuple[CategoryAssignment, ...] = (),
    goals: tuple[CanonicalSavingsGoal, ...] = (),
    safe_to_spend: tuple[Money, ...] = (),
    alert_severities: tuple[AlertSeverity, ...] = (),
    as_of: datetime,
    timezone: str,
    owner_name_aliases: tuple[str, ...] = (),
) -> WeeklyReport:
    """Aggregate one local ISO week without converting or mixing currencies."""

    if as_of.tzinfo is None:
        raise ValueError("as_of must be timezone-aware")
    try:
        zone = ZoneInfo(timezone)
    except ZoneInfoNotFoundError as error:
        raise ValueError("timezone is unknown") from error
    local_date = as_of.astimezone(zone).date()
    start = local_date - timedelta(days=local_date.weekday())
    end = min(start + timedelta(days=6), local_date)
    category_by_transaction = {
        item.transaction_id: item.category for item in assignments
    }
    if len(category_by_transaction) != len(assignments):
        raise ValueError("category assignments must have unique transaction ids")

    income: dict[Currency, Decimal] = defaultdict(Decimal)
    spending: dict[Currency, Decimal] = defaultdict(Decimal)
    transfers: dict[Currency, int] = defaultdict(int)
    category_totals: dict[Currency, dict[str, Decimal]] = defaultdict(
        lambda: defaultdict(Decimal)
    )
    category_counts: dict[Currency, dict[str, int]] = defaultdict(
        lambda: defaultdict(int)
    )
    included = 0
    normalized_aliases = tuple(
        alias
        for raw_alias in owner_name_aliases
        if (alias := normalize_merchant(raw_alias))
    )
    for transaction in transactions:
        occurred = transaction.occurred_at.astimezone(zone).date()
        if not start <= occurred <= end:
            continue
        included += 1
        currency = transaction.amount.currency
        description = (
            normalize_merchant(transaction.description)
            if transaction.description
            else ""
        )
        owner_movement = any(
            re.search(rf"(?<!\\w){re.escape(alias)}(?!\\w)", description)
            for alias in normalized_aliases
        )
        if transaction.kind is TransactionKind.TRANSFER or owner_movement:
            transfers[currency] += 1
        elif transaction.kind is TransactionKind.INFLOW:
            income[currency] += transaction.amount.amount
        elif transaction.kind is TransactionKind.REFUND:
            spending[currency] -= transaction.amount.amount
            category = category_by_transaction.get(
                transaction.transaction_id, "Uncategorised"
            )
            category_totals[currency][category] -= transaction.amount.amount
            category_counts[currency][category] += 1
        else:
            spending[currency] += transaction.amount.amount
            category = category_by_transaction.get(
                transaction.transaction_id, "Uncategorised"
            )
            category_totals[currency][category] += transaction.amount.amount
            category_counts[currency][category] += 1

    currencies = sorted(
        set(income) | set(spending) | set(transfers), key=lambda item: item.code
    )
    summaries = tuple(
        CurrencyWeek(
            currency=currency,
            income=income[currency],
            spending=spending[currency],
            net_cash_flow=income[currency] - spending[currency],
            transfer_count=transfers[currency],
            categories=tuple(
                CategoryTotal(
                    category=category,
                    amount=amount,
                    transaction_count=category_counts[currency][category],
                )
                for category, amount in sorted(
                    category_totals[currency].items(),
                    key=lambda item: (-item[1], item[0].casefold()),
                )
            ),
        )
        for currency in currencies
    )
    safe_totals: dict[Currency, Decimal] = defaultdict(Decimal)
    for amount in safe_to_spend:
        if amount.amount < 0:
            raise ValueError("safe-to-spend amounts cannot be negative")
        current_safe = safe_totals.get(amount.currency)
        safe_totals[amount.currency] = (
            amount.amount if current_safe is None else min(current_safe, amount.amount)
        )
    alert_counts: dict[AlertSeverity, int] = defaultdict(int)
    for severity in alert_severities:
        if severity is not AlertSeverity.NONE:
            alert_counts[severity] += 1
    return WeeklyReport(
        timezone=timezone,
        week_start=start,
        through_date=end,
        complete_week=local_date >= start + timedelta(days=6),
        currencies=summaries,
        safe_to_spend=tuple(
            CurrencyAmount(currency=currency, amount=amount)
            for currency, amount in sorted(
                safe_totals.items(), key=lambda item: item[0].code
            )
        ),
        active_alerts=tuple(
            AlertCount(severity=severity, count=count)
            for severity, count in sorted(
                alert_counts.items(), key=lambda item: item[0].value
            )
        ),
        goals=tuple(
            sorted(
                (evaluate_goal(item.goal, as_of=local_date) for item in goals),
                key=lambda item: item.goal_id,
            )
        ),
        source_transaction_count=included,
    )


class NetWorthComponent(StrictModel):
    account_id: str
    label: str
    owner: SourceSystem
    currency: Currency
    amount: Decimal
    updated_at: AwareDatetime | None
    stale: bool
    excluded_mirror_count: int = Field(ge=0)


class CurrencyNetWorth(StrictModel):
    currency: Currency
    amount: Decimal


class NetWorthReport(StrictModel):
    report_version: str = "2026-09-v1"
    as_of: AwareDatetime
    components: tuple[NetWorthComponent, ...]
    currency_totals: tuple[CurrencyNetWorth, ...]
    total: Decimal | None
    total_currency: Currency | None
    reasons: tuple[str, ...]

    @model_validator(mode="after")
    def validate_total(self) -> NetWorthReport:
        if (self.total is None) != (self.total_currency is None):
            raise ValueError("total and total_currency must be present together")
        return self


def net_worth_report(
    accounts: tuple[CanonicalAccount, ...],
    *,
    as_of: datetime,
    stale_after: timedelta = timedelta(days=2),
) -> NetWorthReport:
    """Count every canonical account once and expose stale/ownership evidence."""

    if as_of.tzinfo is None:
        raise ValueError("as_of must be timezone-aware")
    account_ids = [item.account_id for item in accounts]
    if len(account_ids) != len(set(account_ids)):
        raise ValueError("canonical account ids must be unique")
    components: list[NetWorthComponent] = []
    totals: dict[Currency, Decimal] = defaultdict(Decimal)
    reasons: list[str] = []
    has_stale = False
    included_accounts = tuple(item for item in accounts if item.include_net_worth)
    excluded_count = len(accounts) - len(included_accounts)
    if excluded_count:
        account_label = "account" if excluded_count == 1 else "accounts"
        reasons.append(
            f"source settings exclude {excluded_count} {account_label} from net worth"
        )
    for account in sorted(included_accounts, key=lambda item: item.account_id):
        updated_at = account.canonical_owner.source_updated_at
        stale = updated_at is None or as_of - updated_at > stale_after
        if stale:
            has_stale = True
        components.append(
            NetWorthComponent(
                account_id=account.account_id,
                label=account.name,
                owner=account.canonical_owner.system,
                currency=account.currency,
                amount=account.balance.amount,
                updated_at=updated_at,
                stale=stale,
                excluded_mirror_count=len(account.linked_sources),
            )
        )
        totals[account.currency] += account.balance.amount
    currency_totals = tuple(
        CurrencyNetWorth(currency=currency, amount=amount)
        for currency, amount in sorted(totals.items(), key=lambda item: item[0].code)
    )
    if has_stale:
        reasons.append("stale valuations prevent a combined total")
    if len(currency_totals) > 1:
        reasons.append("mixed currencies require an explicit valuation policy")
    can_total = len(currency_totals) == 1 and not any(item.stale for item in components)
    return NetWorthReport(
        as_of=as_of,
        components=tuple(components),
        currency_totals=currency_totals,
        total=currency_totals[0].amount if can_total else None,
        total_currency=currency_totals[0].currency if can_total else None,
        reasons=tuple(reasons),
    )


class TaxonomySignal(StrEnum):
    SPARSE = "sparse"
    DUPLICATE_LIKE = "duplicate_like"
    OVERLOADED = "overloaded"


class CategoryAggregate(StrictModel):
    category: str = Field(min_length=1, max_length=128)
    transaction_count: int = Field(ge=0)
    amount: Decimal = Field(ge=0)


class TaxonomyRecommendation(StrictModel):
    signal: TaxonomySignal
    categories: tuple[str, ...] = Field(min_length=1)
    reason: str


_CATEGORY_KEY = re.compile(r"[^a-z0-9]+")


def taxonomy_recommendations(
    aggregates: tuple[CategoryAggregate, ...],
) -> tuple[TaxonomyRecommendation, ...]:
    """Propose aggregate-only review signals without mutating the vocabulary."""

    ordered = tuple(sorted(aggregates, key=lambda item: item.category.casefold()))
    names = [item.category for item in ordered]
    if len(names) != len(set(names)):
        raise ValueError("category aggregates must be unique")
    result: list[TaxonomyRecommendation] = []
    for aggregate in ordered:
        if aggregate.transaction_count <= 1:
            result.append(
                TaxonomyRecommendation(
                    signal=TaxonomySignal.SPARSE,
                    categories=(aggregate.category,),
                    reason="At most one transaction uses this category.",
                )
            )
    normalized: dict[str, list[str]] = defaultdict(list)
    for name in names:
        normalized[_CATEGORY_KEY.sub("", name.casefold())].append(name)
    for key in sorted(normalized):
        group = normalized[key]
        if len(group) > 1:
            result.append(
                TaxonomyRecommendation(
                    signal=TaxonomySignal.DUPLICATE_LIKE,
                    categories=tuple(sorted(group)),
                    reason="Category labels normalize to the same value.",
                )
            )
    total_count = sum(item.transaction_count for item in aggregates)
    for aggregate in ordered:
        if total_count and aggregate.transaction_count >= 20:
            share = Decimal(aggregate.transaction_count) / Decimal(total_count)
            if share >= Decimal("0.50"):
                result.append(
                    TaxonomyRecommendation(
                        signal=TaxonomySignal.OVERLOADED,
                        categories=(aggregate.category,),
                        reason=(
                            "At least half of categorized transactions use this label."
                        ),
                    )
                )
    return tuple(result)
