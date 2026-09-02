"""Fail-closed conversion of Firefly piggy banks into canonical savings goals."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass

from finance_assistant.domain.canonical import (
    CanonicalSavingsGoal,
    SourceEntity,
    SourceReference,
    SourceSystem,
)
from finance_assistant.domain.models import Money, SavingsGoal
from finance_assistant.ingestion.mapping import CurrencyRegistry, MappingError
from finance_assistant.integrations.firefly import FireflyPiggyBank


@dataclass(frozen=True, slots=True)
class IneligibleGoal:
    """Content-free explanation for a piggy bank that cannot be evaluated."""

    external_id: str
    reason: str


@dataclass(frozen=True, slots=True)
class GoalMappingReport:
    goals: tuple[CanonicalSavingsGoal, ...]
    ineligible: tuple[IneligibleGoal, ...]


def build_savings_goals(
    piggy_banks: tuple[FireflyPiggyBank, ...],
    *,
    currencies: CurrencyRegistry,
) -> GoalMappingReport:
    """Map eligible active piggy banks and explicitly explain skipped records."""

    identities: set[str] = set()
    goals: list[CanonicalSavingsGoal] = []
    ineligible: list[IneligibleGoal] = []
    for piggy_bank in piggy_banks:
        if piggy_bank.id in identities:
            raise MappingError("duplicate Firefly piggy-bank identity")
        identities.add(piggy_bank.id)
        attributes = piggy_bank.attributes
        reason = _ineligible_reason(piggy_bank)
        if reason is not None:
            ineligible.append(IneligibleGoal(piggy_bank.id, reason))
            continue
        if attributes.target_amount is None:  # pragma: no cover - narrowed above
            raise AssertionError("eligible goal has a target")
        if attributes.start_date is None or attributes.target_date is None:
            raise AssertionError("eligible goal has dates")  # pragma: no cover
        currency = currencies.require(
            attributes.currency_code, attributes.currency_decimal_places
        )
        current = Money(amount=attributes.current_amount, currency=currency)
        goal = SavingsGoal(
            goal_id=f"firefly-piggy-bank:{piggy_bank.id}",
            name=attributes.name,
            target=Money(amount=attributes.target_amount, currency=currency),
            current=current,
            started_on=attributes.start_date.date(),
            target_date=attributes.target_date.date(),
        )
        goals.append(
            CanonicalSavingsGoal(
                goal=goal,
                source=SourceReference(
                    system=SourceSystem.FIREFLY_III,
                    entity=SourceEntity.PIGGY_BANK,
                    external_id=piggy_bank.id,
                    source_revision=(
                        attributes.updated_at.isoformat()
                        if attributes.updated_at
                        else hashlib.sha256(
                            piggy_bank.model_dump_json().encode()
                        ).hexdigest()
                    ),
                    source_updated_at=attributes.updated_at,
                ),
                priority=attributes.order or 10_000,
                protected_amount=current,
            )
        )
    return GoalMappingReport(tuple(goals), tuple(ineligible))


def _ineligible_reason(piggy_bank: FireflyPiggyBank) -> str | None:
    attributes = piggy_bank.attributes
    if not attributes.active:
        return "inactive"
    if attributes.target_amount is None:
        return "target amount is missing"
    if attributes.target_amount <= 0:
        return "target amount is not positive"
    if attributes.current_amount < 0:
        return "current amount is negative"
    if attributes.start_date is None:
        return "start date is missing"
    if attributes.target_date is None:
        return "target date is missing"
    if attributes.target_date.date() <= attributes.start_date.date():
        return "target date is not after start date"
    return None
