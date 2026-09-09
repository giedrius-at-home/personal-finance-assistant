"""Read models and invented demo records for the localhost owner dashboard."""

from __future__ import annotations

import hashlib
from collections import Counter
from datetime import UTC, datetime, timedelta
from decimal import Decimal

from pydantic import AwareDatetime, Field

from finance_assistant.categorisation_service import CategoryProposal, ProposalStatus
from finance_assistant.domain.canonical import (
    CanonicalAccount,
    CanonicalSavingsGoal,
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.categorisation import CategorisationMethod
from finance_assistant.domain.models import (
    AlertSeverity,
    Currency,
    Money,
    SavingsGoal,
    StrictModel,
)
from finance_assistant.domain.reporting import (
    CategoryAggregate,
    CategoryAssignment,
    NetWorthReport,
    TaxonomyRecommendation,
    WeeklyReport,
    net_worth_report,
    taxonomy_recommendations,
    weekly_report,
)
from finance_assistant.domain.settings import OwnerSettings
from finance_assistant.persistence.sqlite import SQLiteDerivedState


class ReviewItem(StrictModel):
    proposal_id: str
    merchant: str
    original_category: str | None
    proposed_category: str | None
    method: CategorisationMethod
    confidence: Decimal | None
    runner_up: str | None
    runner_up_confidence: Decimal | None
    reasons: tuple[str, ...]
    model_version: str | None
    prompt_version: str | None
    policy_version: str
    vocabulary_version: str
    status: ProposalStatus
    source_updated_at: AwareDatetime | None
    transaction_description: str | None = Field(default=None, max_length=1024)
    transaction_amount: Decimal | None = None
    transaction_currency: Currency | None = None
    transaction_occurred_at: AwareDatetime | None = None


class SettingsRevision(StrictModel):
    revision_id: int
    actor: str
    applied_at: AwareDatetime
    active: bool
    rolled_back_at: AwareDatetime | None
    rolled_back_by: str | None


class DashboardView(StrictModel):
    generated_at: AwareDatetime
    data_mode: str
    upstream_writes: str = "disabled"
    weekly: WeeklyReport
    net_worth: NetWorthReport
    review_items: tuple[ReviewItem, ...] = Field(max_length=100)
    taxonomy: tuple[TaxonomyRecommendation, ...]
    settings: OwnerSettings
    settings_revisions: tuple[SettingsRevision, ...] = Field(max_length=50)


class DashboardService:
    """Build the HTML and JSON dashboard from the same deterministic view."""

    def __init__(
        self,
        state: SQLiteDerivedState | None,
        *,
        demo_mode: bool = False,
        owner_name_aliases: tuple[str, ...] = (),
        upstream_writes: str = "disabled",
    ):
        self.state = state
        self.demo_mode = demo_mode
        self.owner_name_aliases = owner_name_aliases
        self.upstream_writes = upstream_writes

    def view(self, *, now: datetime) -> DashboardView:
        if now.tzinfo is None:
            raise ValueError("now must be timezone-aware")
        settings = self.state.get_settings() if self.state else OwnerSettings()
        if self.demo_mode:
            accounts, transactions, goals, proposals = _demo_records(now)
            severities: tuple[AlertSeverity, ...] = (AlertSeverity.HEADS_UP,)
            safe: tuple[Money, ...] = (Money(amount="420", currency=_EUR),)
            revisions: tuple[SettingsRevision, ...] = ()
        else:
            accounts = self.state.list_accounts() if self.state else ()
            transactions = self.state.list_transactions() if self.state else ()
            goals = self.state.list_goals() if self.state else ()
            proposals = self.state.list_proposals(limit=100) if self.state else ()
            alerts = self.state.list_active_alerts(limit=100) if self.state else ()
            severities = tuple(item[2] for item in alerts)
            safe = self.state.list_safe_to_spend() if self.state else ()
            revisions = tuple(
                SettingsRevision(
                    revision_id=item[0],
                    actor=item[1],
                    applied_at=item[2],
                    active=item[3],
                    rolled_back_at=item[4],
                    rolled_back_by=item[5],
                )
                for item in (self.state.list_settings_revisions() if self.state else ())
            )
        assignments = _assignments(transactions, proposals)
        weekly = weekly_report(
            transactions,
            assignments=assignments,
            goals=goals,
            safe_to_spend=safe,
            alert_severities=severities,
            as_of=now,
            timezone=settings.timezone,
            owner_name_aliases=self.owner_name_aliases,
        )
        aggregates = tuple(
            CategoryAggregate(
                category=item.category,
                transaction_count=item.transaction_count,
                amount=abs(item.amount),
            )
            for currency in weekly.currencies
            for item in currency.categories
        )
        transactions_by_source = {
            transaction.source.identity: transaction for transaction in transactions
        }
        return DashboardView(
            generated_at=now,
            data_mode="invented demo" if self.demo_mode else "local derived state",
            upstream_writes=self.upstream_writes,
            weekly=weekly,
            net_worth=net_worth_report(accounts, as_of=now),
            review_items=tuple(
                _review_item(item, transactions_by_source.get(item.source.identity))
                for item in proposals
            ),
            taxonomy=taxonomy_recommendations(_merge_aggregates(aggregates)),
            settings=settings,
            settings_revisions=revisions,
        )


def _review_item(
    proposal: CategoryProposal, transaction: CanonicalTransaction | None
) -> ReviewItem:
    signed_amount: Decimal | None = None
    if transaction is not None:
        signed_amount = transaction.amount.amount
        if transaction.kind is TransactionKind.OUTFLOW:
            signed_amount = -signed_amount
    return ReviewItem(
        proposal_id=proposal.proposal_id,
        merchant=proposal.normalized_merchant,
        original_category=proposal.original_category,
        proposed_category=proposal.proposed_category,
        method=proposal.method,
        confidence=proposal.confidence,
        runner_up=proposal.runner_up,
        runner_up_confidence=proposal.runner_up_confidence,
        reasons=proposal.reasons,
        model_version=proposal.model_version,
        prompt_version=proposal.prompt_version,
        policy_version=proposal.policy_version,
        vocabulary_version=proposal.vocabulary_version,
        status=proposal.status,
        source_updated_at=proposal.source.source_updated_at,
        transaction_description=(transaction.description if transaction else None),
        transaction_amount=signed_amount,
        transaction_currency=(transaction.amount.currency if transaction else None),
        transaction_occurred_at=(transaction.occurred_at if transaction else None),
    )


def _assignments(
    transactions: tuple[CanonicalTransaction, ...],
    proposals: tuple[CategoryProposal, ...],
) -> tuple[CategoryAssignment, ...]:
    by_source = {
        item.source.identity: item.proposed_category
        for item in proposals
        if item.proposed_category is not None
        and item.status in {ProposalStatus.PENDING, ProposalStatus.APPROVED}
    }
    assignments: list[CategoryAssignment] = []
    for item in transactions:
        category = item.category or by_source.get(item.source.identity)
        if category is not None:
            assignments.append(
                CategoryAssignment(
                    transaction_id=item.transaction_id, category=category
                )
            )
    return tuple(assignments)


def _merge_aggregates(
    aggregates: tuple[CategoryAggregate, ...],
) -> tuple[CategoryAggregate, ...]:
    counts: Counter[str] = Counter()
    amounts: dict[str, Decimal] = {}
    for item in aggregates:
        counts[item.category] += item.transaction_count
        amounts[item.category] = amounts.get(item.category, Decimal()) + item.amount
    return tuple(
        CategoryAggregate(
            category=name, transaction_count=counts[name], amount=amounts[name]
        )
        for name in sorted(counts, key=str.casefold)
    )


_EUR = Currency(code="EUR", minor_unit=2)


def _source(
    system: SourceSystem, entity: SourceEntity, external_id: str, now: datetime
) -> SourceReference:
    return SourceReference(
        system=system,
        entity=entity,
        external_id=external_id,
        source_revision="invented-demo-v1",
        source_updated_at=now.astimezone(UTC),
    )


def _demo_records(
    now: datetime,
) -> tuple[
    tuple[CanonicalAccount, ...],
    tuple[CanonicalTransaction, ...],
    tuple[CanonicalSavingsGoal, ...],
    tuple[CategoryProposal, ...],
]:
    """Return ephemeral, explicitly invented records; nothing is persisted."""

    accounts = (
        CanonicalAccount(
            account_id="demo-cash",
            name="Invented current account",
            balance=Money(amount="2300", currency=_EUR),
            canonical_owner=_source(
                SourceSystem.FIREFLY_III, SourceEntity.ACCOUNT, "demo-cash", now
            ),
        ),
        CanonicalAccount(
            account_id="demo-investments",
            name="Invented portfolio",
            balance=Money(amount="8000", currency=_EUR),
            canonical_owner=_source(
                SourceSystem.GHOSTFOLIO, SourceEntity.ACCOUNT, "demo-investments", now
            ),
        ),
    )
    specs = (
        ("salary", TransactionKind.INFLOW, "2400", "Invented salary"),
        ("grocer", TransactionKind.OUTFLOW, "86.40", "Northstar Market"),
        ("bicycle", TransactionKind.OUTFLOW, "42.00", "Northstar Bicycle Repair"),
        ("coffee", TransactionKind.OUTFLOW, "4.80", "Moonbeam Coffee"),
    )
    transactions = tuple(
        CanonicalTransaction(
            transaction_id=f"demo-{external_id}",
            source=_source(
                SourceSystem.FIREFLY_III, SourceEntity.TRANSACTION, external_id, now
            ),
            account_id="demo-cash",
            occurred_at=now - timedelta(hours=index + 1),
            kind=kind,
            amount=Money(amount=amount, currency=_EUR),
            description=description,
        )
        for index, (external_id, kind, amount, description) in enumerate(specs)
    )
    goal = CanonicalSavingsGoal(
        goal=SavingsGoal(
            goal_id="demo-emergency",
            name="Invented emergency fund",
            target=Money(amount="6000", currency=_EUR),
            current=Money(amount="2400", currency=_EUR),
            started_on=(now - timedelta(days=120)).date(),
            target_date=(now + timedelta(days=240)).date(),
        ),
        source=_source(
            SourceSystem.FIREFLY_III, SourceEntity.PIGGY_BANK, "demo-goal", now
        ),
        priority=1,
        protected_amount=Money(amount="2400", currency=_EUR),
    )
    categories = {"grocer": "Groceries", "bicycle": "Transport", "coffee": "Dining"}
    proposals = tuple(
        CategoryProposal(
            proposal_id=hashlib.sha256(f"demo:{external_id}".encode()).hexdigest(),
            source=_source(
                SourceSystem.FIREFLY_III, SourceEntity.TRANSACTION, external_id, now
            ),
            original_category=None,
            proposed_category=category,
            method=CategorisationMethod.MODEL
            if external_id == "bicycle"
            else CategorisationMethod.BASELINE,
            confidence=Decimal("0.95") if external_id == "bicycle" else Decimal("1"),
            runner_up="Home maintenance" if external_id == "bicycle" else None,
            runner_up_confidence=Decimal("0.03") if external_id == "bicycle" else None,
            reasons=("Invented merchant matched the approved local vocabulary.",),
            model_version="qwen3:8b" if external_id == "bicycle" else None,
            prompt_version="2026-09-v1" if external_id == "bicycle" else None,
            policy_version="2026-09-v1",
            vocabulary_version="2026-09-v1",
            feature_hash=hashlib.sha256(f"feature:{external_id}".encode()).hexdigest(),
            normalized_merchant=next(
                item[3] for item in specs if item[0] == external_id
            ),
            created_at=now,
        )
        for external_id, category in categories.items()
    )
    return accounts, transactions, (goal,), proposals
