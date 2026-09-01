from datetime import UTC, datetime
from decimal import Decimal

import pytest
from pydantic import ValidationError

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money

EUR = Currency(code="EUR", minor_unit=2)


def source(
    external_id: str,
    *,
    system: SourceSystem = SourceSystem.FIREFLY_III,
    entity: SourceEntity = SourceEntity.ACCOUNT,
) -> SourceReference:
    return SourceReference(
        system=system,
        entity=entity,
        external_id=external_id,
        source_revision="revision-1",
        source_updated_at=datetime(2026, 9, 1, tzinfo=UTC),
    )


def test_account_has_one_explicit_owner_and_deduplicated_lineage() -> None:
    owner = source("firefly-account-1")
    linked = source(
        "ghostfolio-account-1",
        system=SourceSystem.GHOSTFOLIO,
    )

    account = CanonicalAccount(
        account_id="canonical-account-1",
        name="Brokerage cash",
        currency=EUR,
        canonical_owner=owner,
        linked_sources=(linked,),
    )

    assert account.canonical_owner is owner
    assert account.linked_sources == (linked,)

    with pytest.raises(ValidationError, match="duplicate source identity"):
        CanonicalAccount(
            account_id="canonical-account-1",
            name="Brokerage cash",
            currency=EUR,
            canonical_owner=owner,
            linked_sources=(
                owner.model_copy(update={"source_revision": "revision-2"}),
            ),
        )


def test_account_lineage_rejects_non_account_entities() -> None:
    with pytest.raises(ValidationError, match="account entities"):
        CanonicalAccount(
            account_id="canonical-account-1",
            name="Cash",
            currency=EUR,
            canonical_owner=source(
                "transaction-1",
                entity=SourceEntity.TRANSACTION,
            ),
        )


def transaction(**overrides: object) -> CanonicalTransaction:
    values: dict[str, object] = {
        "transaction_id": "canonical-transaction-1",
        "source": source("firefly-transaction-1", entity=SourceEntity.TRANSACTION),
        "account_id": "canonical-account-1",
        "occurred_at": datetime(2026, 9, 1, 12, tzinfo=UTC),
        "kind": TransactionKind.OUTFLOW,
        "amount": Money(amount=Decimal("12.34"), currency=EUR),
        "description": "Synthetic merchant",
    }
    values.update(overrides)
    return CanonicalTransaction.model_validate(values)


def test_transaction_requires_positive_amount_and_aware_time() -> None:
    with pytest.raises(ValidationError, match="must be positive"):
        transaction(amount=Money(amount="0", currency=EUR))

    with pytest.raises(ValidationError, match="timezone"):
        transaction(occurred_at=datetime(2026, 9, 1, 12))


def test_transfer_requires_distinct_counterparty() -> None:
    with pytest.raises(ValidationError, match="requires a counterparty"):
        transaction(kind=TransactionKind.TRANSFER)

    with pytest.raises(ValidationError, match="two distinct"):
        transaction(
            kind=TransactionKind.TRANSFER,
            counterparty_account_id="canonical-account-1",
        )

    transfer = transaction(
        kind=TransactionKind.TRANSFER,
        counterparty_account_id="canonical-account-2",
    )
    assert transfer.counterparty_account_id == "canonical-account-2"


def test_non_transfer_rejects_counterparty_and_account_lineage() -> None:
    with pytest.raises(ValidationError, match="only a transfer"):
        transaction(counterparty_account_id="canonical-account-2")

    with pytest.raises(ValidationError, match="transaction or activity"):
        transaction(source=source("firefly-account-1"))


def test_source_identity_excludes_mutable_revision_evidence() -> None:
    initial = source("firefly-transaction-1", entity=SourceEntity.TRANSACTION)
    updated = initial.model_copy(update={"source_revision": "revision-2"})

    assert initial.identity == updated.identity
    assert initial.source_revision != updated.source_revision
