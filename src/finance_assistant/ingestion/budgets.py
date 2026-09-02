"""Convert Firefly budget limits into exact deterministic evaluation windows."""

from __future__ import annotations

from datetime import date

from pydantic import model_validator

from finance_assistant.domain.models import BudgetWindow, Money, StrictModel
from finance_assistant.ingestion.mapping import CurrencyRegistry, MappingError
from finance_assistant.integrations.firefly import FireflyBudgetLimit


class BudgetCashContext(StrictModel):
    """Explicit cash protections not represented by a Firefly budget limit."""

    available_cash: Money
    upcoming_committed: Money
    planned_savings: Money
    minimum_buffer: Money

    @model_validator(mode="after")
    def require_one_currency(self) -> BudgetCashContext:
        currencies = {
            self.available_cash.currency,
            self.upcoming_committed.currency,
            self.planned_savings.currency,
            self.minimum_buffer.currency,
        }
        if len(currencies) != 1:
            raise ValueError("budget cash context must use one currency")
        return self


def build_budget_windows(
    limits: tuple[FireflyBudgetLimit, ...],
    *,
    as_of: date,
    currencies: CurrencyRegistry,
    cash_by_currency: dict[str, BudgetCashContext],
) -> tuple[BudgetWindow, ...]:
    """Build windows without inferring missing cash protections or currency."""

    windows: list[BudgetWindow] = []
    identities: set[str] = set()
    for limit in limits:
        if limit.id in identities:
            raise MappingError("duplicate Firefly budget-limit identity")
        identities.add(limit.id)
        attributes = limit.attributes
        currency = currencies.require(
            attributes.currency_code, attributes.currency_decimal_places
        )
        context = cash_by_currency.get(currency.code)
        if context is None or context.available_cash.currency != currency:
            raise MappingError("budget cash context is missing or has wrong precision")
        if attributes.amount <= 0:
            raise MappingError("budget limit amount must be positive")
        matching = [
            item
            for item in attributes.spent
            if currencies.require(item.currency_code, item.currency_decimal_places)
            == currency
        ]
        if len(matching) > 1:
            raise MappingError("budget limit has duplicate currency totals")
        signed_spent = matching[0].sum if matching else 0
        if signed_spent > 0:
            raise MappingError("budget spent total has an unexpected positive sign")
        start = attributes.start.date()
        end = attributes.end.date()
        if end < start:
            raise MappingError("budget limit ends before it starts")
        total_days = (end - start).days + 1
        elapsed_days = min(max((as_of - start).days + 1, 0), total_days)
        windows.append(
            BudgetWindow(
                category=f"firefly-budget:{attributes.budget_id}",
                budget=Money(amount=attributes.amount, currency=currency),
                spent=Money(amount=-signed_spent, currency=currency),
                available_cash=context.available_cash,
                upcoming_committed=context.upcoming_committed,
                planned_savings=context.planned_savings,
                minimum_buffer=context.minimum_buffer,
                elapsed_days=elapsed_days,
                total_days=total_days,
            )
        )
    return tuple(windows)
