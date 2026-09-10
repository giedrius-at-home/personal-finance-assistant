"""Fail-closed conversion from pinned upstream DTOs into canonical records."""

from __future__ import annotations

import hashlib
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from datetime import datetime
from decimal import Decimal

from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.integrations.firefly import (
    FireflyAccount,
    FireflyCurrency,
    FireflyTransaction,
    FireflyTransactionSplit,
)
from finance_assistant.integrations.ghostfolio import GhostfolioAccount


class MappingError(ValueError):
    """Source data cannot be mapped without making a financial assumption."""


class CurrencyRegistry:
    """Resolve observed currencies and reject conflicting precision metadata."""

    def __init__(self, currencies: Iterable[Currency] = ()) -> None:
        self._currencies: dict[str, Currency] = {}
        for currency in currencies:
            self.add(currency)

    @classmethod
    def from_firefly(cls, currencies: Iterable[FireflyCurrency]) -> CurrencyRegistry:
        registry = cls()
        for source in currencies:
            registry.add(
                Currency(
                    code=source.attributes.code,
                    minor_unit=source.attributes.decimal_places,
                )
            )
        return registry

    def add(self, currency: Currency) -> None:
        existing = self._currencies.get(currency.code)
        if existing is not None and existing != currency:
            raise MappingError(f"conflicting precision for currency {currency.code}")
        self._currencies[currency.code] = currency

    def require(self, code: str, minor_unit: int | None = None) -> Currency:
        currency = self._currencies.get(code.upper())
        if currency is None:
            raise MappingError(f"unknown currency {code.upper()}")
        if minor_unit is not None and currency.minor_unit != minor_unit:
            raise MappingError(f"conflicting precision for currency {currency.code}")
        return currency


@dataclass(frozen=True, slots=True)
class AccountLink:
    """Link two representations and explicitly select one authoritative owner."""

    firefly_account_id: str
    ghostfolio_account_id: str
    owner: SourceSystem

    def __post_init__(self) -> None:
        if self.owner not in {SourceSystem.FIREFLY_III, SourceSystem.GHOSTFOLIO}:
            raise MappingError("linked account owner is unsupported")
        if (
            not self.firefly_account_id.strip()
            or not self.ghostfolio_account_id.strip()
        ):
            raise MappingError("linked account IDs cannot be empty")


def _revision(updated_at: datetime | None, payload: str) -> str:
    if updated_at is not None:
        return str(updated_at)
    return hashlib.sha256(payload.encode()).hexdigest()


def _account_reference(
    system: SourceSystem, external_id: str, updated_at: datetime | None
) -> SourceReference:
    return SourceReference(
        system=system,
        entity=SourceEntity.ACCOUNT,
        external_id=external_id,
        source_revision=_revision(updated_at, external_id),
        source_updated_at=updated_at,
    )


def build_canonical_accounts(
    *,
    firefly_accounts: Iterable[FireflyAccount],
    ghostfolio_accounts: Iterable[GhostfolioAccount],
    currencies: CurrencyRegistry,
    links: Iterable[AccountLink] = (),
) -> tuple[CanonicalAccount, ...]:
    """Map accounts, using Firefly as owner only for explicit cross-system links."""

    firefly_source = tuple(firefly_accounts)
    ghostfolio_source = tuple(ghostfolio_accounts)
    firefly = {account.id: account for account in firefly_source}
    ghostfolio = {account.id: account for account in ghostfolio_source}
    if len(firefly) != len(firefly_source):
        raise MappingError("duplicate Firefly account identity")
    if len(ghostfolio) != len(ghostfolio_source):
        raise MappingError("duplicate Ghostfolio account identity")

    linked_firefly: set[str] = set()
    linked_ghostfolio: set[str] = set()
    link_map: dict[str, AccountLink] = {}
    for link in links:
        if (
            link.firefly_account_id in linked_firefly
            or link.ghostfolio_account_id in linked_ghostfolio
        ):
            raise MappingError("ambiguous account ownership link")
        if link.firefly_account_id not in firefly:
            raise MappingError("linked Firefly account does not exist")
        if link.ghostfolio_account_id not in ghostfolio:
            raise MappingError("linked Ghostfolio account does not exist")
        linked_firefly.add(link.firefly_account_id)
        linked_ghostfolio.add(link.ghostfolio_account_id)
        link_map[link.firefly_account_id] = link

    records: list[CanonicalAccount] = []
    for firefly_account in firefly.values():
        attributes = firefly_account.attributes
        currency = currencies.require(
            attributes.currency_code, attributes.currency_decimal_places
        )
        owner = _account_reference(
            SourceSystem.FIREFLY_III, firefly_account.id, attributes.updated_at
        )
        linked: tuple[SourceReference, ...] = ()
        include_net_worth = attributes.include_net_worth
        configured_link = link_map.get(firefly_account.id)
        if configured_link is not None:
            mirror = ghostfolio[configured_link.ghostfolio_account_id]
            if mirror.currency is None:
                raise MappingError("linked Ghostfolio account has no currency")
            if currencies.require(mirror.currency) != currency:
                raise MappingError("linked accounts have conflicting currencies")
            mirror_reference = _account_reference(
                SourceSystem.GHOSTFOLIO, mirror.id, mirror.updated_at
            )
            if configured_link.owner is SourceSystem.GHOSTFOLIO:
                owner, linked = mirror_reference, (owner,)
                account_id = f"ghostfolio:{mirror.id}"
                name = mirror.name or f"Ghostfolio {mirror.id}"
                balance = mirror.value
                include_net_worth = True
            else:
                linked = (mirror_reference,)
                account_id = f"firefly:{firefly_account.id}"
                name = attributes.name
                balance = attributes.current_balance
        else:
            account_id = f"firefly:{firefly_account.id}"
            name = attributes.name
            balance = attributes.current_balance
        records.append(
            CanonicalAccount(
                account_id=account_id,
                name=name,
                balance=Money(amount=balance, currency=currency),
                canonical_owner=owner,
                linked_sources=linked,
                include_net_worth=include_net_worth,
            )
        )

    for ghostfolio_account in ghostfolio.values():
        if ghostfolio_account.id in linked_ghostfolio:
            continue
        if ghostfolio_account.currency is None:
            raise MappingError("Ghostfolio account has no currency")
        currency = currencies.require(ghostfolio_account.currency)
        records.append(
            CanonicalAccount(
                account_id=f"ghostfolio:{ghostfolio_account.id}",
                name=(ghostfolio_account.name or f"Ghostfolio {ghostfolio_account.id}"),
                balance=Money(amount=ghostfolio_account.value, currency=currency),
                canonical_owner=_account_reference(
                    SourceSystem.GHOSTFOLIO,
                    ghostfolio_account.id,
                    ghostfolio_account.updated_at,
                ),
            )
        )
    return tuple(records)


def _map_split(
    split: FireflyTransactionSplit,
    *,
    updated_at: datetime | None,
    currencies: CurrencyRegistry,
    account_ids: Mapping[str, str],
) -> CanonicalTransaction:
    kind_by_source = {
        "deposit": TransactionKind.INFLOW,
        "withdrawal": TransactionKind.OUTFLOW,
        "transfer": TransactionKind.TRANSFER,
    }
    kind = kind_by_source.get(split.type.lower())
    if kind is None:
        raise MappingError(f"unsupported Firefly transaction type {split.type}")
    if split.amount <= Decimal(0):
        raise MappingError("Firefly transaction amount must be positive")
    if kind is TransactionKind.INFLOW:
        source_account, counterparty = split.destination_id, None
    elif kind is TransactionKind.OUTFLOW:
        source_account, counterparty = split.source_id, None
    else:
        source_account, counterparty = split.source_id, split.destination_id
    if source_account is None or source_account not in account_ids:
        raise MappingError("transaction account ownership is unresolved")
    if counterparty is not None and counterparty not in account_ids:
        raise MappingError("transfer counterparty ownership is unresolved")
    currency = currencies.require(split.currency_code, split.currency_decimal_places)
    return CanonicalTransaction(
        transaction_id=f"firefly:{split.transaction_journal_id}",
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=split.transaction_journal_id,
            source_revision=_revision(updated_at, split.model_dump_json()),
            source_updated_at=updated_at,
        ),
        account_id=account_ids[source_account],
        occurred_at=split.date,
        kind=kind,
        amount=Money(amount=split.amount, currency=currency),
        counterparty_account_id=(
            account_ids[counterparty] if counterparty is not None else None
        ),
        description=split.description,
        category=split.category_name,
        subscription_id=split.bill_id,
    )


def build_canonical_transactions(
    transactions: Iterable[FireflyTransaction],
    *,
    currencies: CurrencyRegistry,
    accounts: Iterable[CanonicalAccount],
) -> tuple[CanonicalTransaction, ...]:
    """Map all Firefly journals and reject missing or ambiguous account ownership."""

    account_ids: dict[str, str] = {}
    for account in accounts:
        for reference in (account.canonical_owner, *account.linked_sources):
            if reference.system is not SourceSystem.FIREFLY_III:
                continue
            if reference.external_id in account_ids:
                raise MappingError("ambiguous Firefly account ownership")
            account_ids[reference.external_id] = account.account_id
    records: list[CanonicalTransaction] = []
    identities: set[str] = set()
    for group in transactions:
        for split in group.attributes.transactions:
            if split.transaction_journal_id in identities:
                raise MappingError("duplicate Firefly transaction identity")
            identities.add(split.transaction_journal_id)
            records.append(
                _map_split(
                    split,
                    updated_at=group.attributes.updated_at,
                    currencies=currencies,
                    account_ids=account_ids,
                )
            )
    return tuple(records)
