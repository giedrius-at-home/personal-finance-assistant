from datetime import date

import pytest

from finance_assistant.domain.models import Currency, Money
from finance_assistant.ingestion.budgets import BudgetCashContext, build_budget_windows
from finance_assistant.ingestion.mapping import CurrencyRegistry, MappingError
from finance_assistant.integrations.firefly import FireflyBudgetLimit

EUR = Currency(code="EUR", minor_unit=2)


def budget_limit(**overrides: object) -> FireflyBudgetLimit:
    attributes: dict[str, object] = {
        "start": "2026-09-01T00:00:00+02:00",
        "end": "2026-09-30T23:59:59+02:00",
        "budget_id": "synthetic-dining",
        "currency_code": "EUR",
        "currency_decimal_places": 2,
        "amount": "300.00",
        "spent": [
            {
                "currency_code": "EUR",
                "currency_decimal_places": 2,
                "sum": "-90.00",
            }
        ],
    }
    attributes.update(overrides)
    return FireflyBudgetLimit.model_validate(
        {"type": "budget_limits", "id": "synthetic-limit", "attributes": attributes}
    )


def context() -> BudgetCashContext:
    return BudgetCashContext(
        available_cash=Money(amount="1000", currency=EUR),
        upcoming_committed=Money(amount="200", currency=EUR),
        planned_savings=Money(amount="300", currency=EUR),
        minimum_buffer=Money(amount="100", currency=EUR),
    )


def test_budget_limit_maps_exact_inclusive_window() -> None:
    windows = build_budget_windows(
        (budget_limit(),),
        as_of=date(2026, 9, 10),
        currencies=CurrencyRegistry((EUR,)),
        cash_by_currency={"EUR": context()},
    )

    assert windows[0].spent.amount == 90
    assert windows[0].elapsed_days == 10
    assert windows[0].total_days == 30
    assert windows[0].category == "firefly-budget:synthetic-dining"


@pytest.mark.parametrize(
    "overrides,message",
    [
        ({"amount": "0"}, "must be positive"),
        (
            {
                "spent": [
                    {"currency_code": "EUR", "currency_decimal_places": 2, "sum": "1"}
                ]
            },
            "positive sign",
        ),
        ({"end": "2026-08-31T00:00:00+02:00"}, "ends before"),
    ],
)
def test_budget_limit_ambiguity_fails_closed(
    overrides: dict[str, object], message: str
) -> None:
    with pytest.raises(MappingError, match=message):
        build_budget_windows(
            (budget_limit(**overrides),),
            as_of=date(2026, 9, 10),
            currencies=CurrencyRegistry((EUR,)),
            cash_by_currency={"EUR": context()},
        )
