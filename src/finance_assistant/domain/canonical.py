"""Source-independent account, transaction, and lineage contracts."""

from enum import StrEnum

from pydantic import AwareDatetime, Field, model_validator

from finance_assistant.domain.models import Currency, Money, StrictModel


class SourceSystem(StrEnum):
    """Upstream systems allowed to own canonical source records."""

    FIREFLY_III = "firefly_iii"
    GHOSTFOLIO = "ghostfolio"


class SourceEntity(StrEnum):
    """Stable upstream entity kinds used in source lineage."""

    ACCOUNT = "account"
    TRANSACTION = "transaction"
    ACTIVITY = "activity"


class SourceReference(StrictModel):
    """Immutable identity and optional revision evidence for one source record."""

    system: SourceSystem
    entity: SourceEntity
    external_id: str = Field(min_length=1, max_length=256)
    source_revision: str | None = Field(default=None, min_length=1, max_length=256)
    source_updated_at: AwareDatetime | None = None

    @model_validator(mode="after")
    def reject_ambiguous_identity(self) -> SourceReference:
        if self.external_id != self.external_id.strip():
            raise ValueError("external_id cannot have surrounding whitespace")
        if self.source_revision is not None:
            if self.source_revision != self.source_revision.strip():
                raise ValueError("source_revision cannot have surrounding whitespace")
        return self

    @property
    def identity(self) -> tuple[SourceSystem, SourceEntity, str]:
        """Return the immutable portion used for idempotency and deduplication."""

        return (self.system, self.entity, self.external_id)


class CanonicalAccount(StrictModel):
    """One canonical account with one explicit authoritative source record."""

    account_id: str = Field(min_length=1, max_length=128)
    name: str = Field(min_length=1, max_length=256)
    balance: Money
    canonical_owner: SourceReference
    linked_sources: tuple[SourceReference, ...] = ()

    @property
    def currency(self) -> Currency:
        """Expose the balance currency for callers that only need identity."""

        return self.balance.currency

    @model_validator(mode="after")
    def validate_ownership(self) -> CanonicalAccount:
        references = (self.canonical_owner, *self.linked_sources)
        if any(
            reference.entity is not SourceEntity.ACCOUNT for reference in references
        ):
            raise ValueError("account lineage must reference account entities")
        identities = [reference.identity for reference in references]
        if len(identities) != len(set(identities)):
            raise ValueError("account lineage contains a duplicate source identity")
        if self.account_id != self.account_id.strip():
            raise ValueError("account_id cannot have surrounding whitespace")
        if self.name != self.name.strip():
            raise ValueError("account name cannot have surrounding whitespace")
        return self


class TransactionKind(StrEnum):
    """Direction of a positive canonical transaction amount."""

    INFLOW = "inflow"
    OUTFLOW = "outflow"
    TRANSFER = "transfer"


class CanonicalTransaction(StrictModel):
    """A positive amount and direction linked to one immutable source identity."""

    transaction_id: str = Field(min_length=1, max_length=128)
    source: SourceReference
    account_id: str = Field(min_length=1, max_length=128)
    occurred_at: AwareDatetime
    kind: TransactionKind
    amount: Money
    counterparty_account_id: str | None = Field(
        default=None, min_length=1, max_length=128
    )
    description: str | None = Field(default=None, max_length=1024)

    @model_validator(mode="after")
    def validate_transaction(self) -> CanonicalTransaction:
        if self.source.entity not in {
            SourceEntity.TRANSACTION,
            SourceEntity.ACTIVITY,
        }:
            raise ValueError(
                "transaction lineage must reference a transaction or activity"
            )
        if self.amount.amount <= 0:
            raise ValueError("canonical transaction amount must be positive")
        if self.kind is TransactionKind.TRANSFER:
            if self.counterparty_account_id is None:
                raise ValueError("a transfer requires a counterparty_account_id")
            if self.counterparty_account_id == self.account_id:
                raise ValueError("a transfer must use two distinct accounts")
        elif self.counterparty_account_id is not None:
            raise ValueError("only a transfer can have a counterparty_account_id")
        for field_name, value in (
            ("transaction_id", self.transaction_id),
            ("account_id", self.account_id),
            ("counterparty_account_id", self.counterparty_account_id),
        ):
            if value is not None and value != value.strip():
                raise ValueError(f"{field_name} cannot have surrounding whitespace")
        return self
