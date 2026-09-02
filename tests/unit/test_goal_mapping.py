from datetime import UTC, datetime
from decimal import Decimal

import pytest

from finance_assistant.domain.canonical import SourceEntity
from finance_assistant.domain.models import Currency
from finance_assistant.ingestion.goals import build_savings_goals
from finance_assistant.ingestion.mapping import CurrencyRegistry, MappingError
from finance_assistant.integrations.firefly import FireflyPiggyBank


def piggy_bank(**overrides: object) -> FireflyPiggyBank:
    attributes: dict[str, object] = {
        "name": "Synthetic emergency fund",
        "start_date": "2026-01-01T00:00:00+00:00",
        "target_date": "2026-12-31T00:00:00+00:00",
        "order": 2,
        "active": True,
        "currency_code": "EUR",
        "currency_decimal_places": 2,
        "target_amount": "1200.00",
        "current_amount": "300.00",
        "updated_at": "2026-09-02T00:00:00+00:00",
    }
    attributes.update(overrides)
    return FireflyPiggyBank.model_validate(
        {"type": "piggy_banks", "id": "synthetic-piggy", "attributes": attributes}
    )


def test_piggy_bank_maps_exact_goal_metadata_and_lineage() -> None:
    report = build_savings_goals(
        (piggy_bank(),),
        currencies=CurrencyRegistry((Currency(code="EUR", minor_unit=2),)),
    )

    assert report.ineligible == ()
    mapped = report.goals[0]
    assert mapped.goal.target.amount == Decimal("1200.00")
    assert mapped.goal.current.amount == Decimal("300.00")
    assert mapped.protected_amount == mapped.goal.current
    assert mapped.priority == 2
    assert mapped.source.entity is SourceEntity.PIGGY_BANK
    assert mapped.source.source_updated_at == datetime(2026, 9, 2, tzinfo=UTC)


@pytest.mark.parametrize(
    "override,reason",
    [
        ({"active": False}, "inactive"),
        ({"target_amount": None}, "target amount is missing"),
        ({"target_amount": "0"}, "target amount is not positive"),
        ({"current_amount": "-1"}, "current amount is negative"),
        ({"start_date": None}, "start date is missing"),
        ({"target_date": None}, "target date is missing"),
        (
            {"target_date": "2025-12-31T00:00:00+00:00"},
            "target date is not after start date",
        ),
    ],
)
def test_ineligible_piggy_bank_has_content_free_reason(
    override: dict[str, object], reason: str
) -> None:
    report = build_savings_goals(
        (piggy_bank(**override),),
        currencies=CurrencyRegistry((Currency(code="EUR", minor_unit=2),)),
    )

    assert report.goals == ()
    assert report.ineligible[0].reason == reason
    assert "Synthetic emergency fund" not in report.ineligible[0].reason


def test_goal_mapping_rejects_duplicate_identity_and_currency_drift() -> None:
    registry = CurrencyRegistry((Currency(code="EUR", minor_unit=2),))
    with pytest.raises(MappingError, match="duplicate"):
        build_savings_goals((piggy_bank(), piggy_bank()), currencies=registry)
    with pytest.raises(MappingError, match="conflicting precision"):
        build_savings_goals(
            (piggy_bank(currency_decimal_places=3),), currencies=registry
        )
