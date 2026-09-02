"""Phase 4 synthetic lifecycle: deterministic-to-model-to-review with no real data.

Every merchant, category, and transaction identity below is invented. The model
is a deterministic fake; no Ollama process or upstream service is contacted.
"""

from datetime import UTC, datetime
from decimal import Decimal

import pytest

from finance_assistant.categorisation_service import (
    CategorisationService,
    CategoryWriteResult,
    ProposalStatus,
    QwenCategoriser,
)
from finance_assistant.domain.canonical import (
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.categorisation import (
    BaselineRule,
    BaselineRuleKind,
    BaselineRuleSet,
    CalibrationPolicy,
    CategorisationMethod,
    CategoryVocabulary,
)
from finance_assistant.persistence.sqlite import (
    PersistenceConflictError,
    SQLiteDerivedState,
)

VOCAB = CategoryVocabulary(
    version="2026-09-v1",
    categories=("dining", "groceries", "transport"),
)
BASELINE = BaselineRuleSet(
    rules=(
        BaselineRule(
            kind=BaselineRuleKind.EXACT, merchant="ACME BAKERY", category="dining"
        ),
    )
)
POLICY = CalibrationPolicy(
    min_holdout_examples=1,
    min_per_category_examples=1,
    target_precision=Decimal("0.95"),
)
NOW = datetime(2026, 9, 2, 12, 0, tzinfo=UTC)


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


class QueueBackend:
    def __init__(self, responses: list[str]) -> None:
        self.responses = responses

    async def generate(self, prompt: str, *, system: str | None = None) -> str:
        if not self.responses:
            raise RuntimeError("model is down")
        return self.responses.pop(0)


class UnavailableBackend:
    async def generate(self, prompt: str, *, system: str | None = None) -> str:
        raise RuntimeError("model is down")


class RecordingWritePort:
    def __init__(self) -> None:
        self.applied: list[str] = []
        self.reversed: list[str] = []

    def apply(self, proposal) -> CategoryWriteResult:
        self.applied.append(proposal.proposal_id)
        return CategoryWriteResult.APPLIED

    def reverse(self, batch_id: str) -> CategoryWriteResult:
        self.reversed.append(batch_id)
        return CategoryWriteResult.REVERSED


def source(external_id: str, revision: str | None = None) -> SourceReference:
    return SourceReference(
        system=SourceSystem.FIREFLY_III,
        entity=SourceEntity.TRANSACTION,
        external_id=external_id,
        source_revision=revision,
    )


def model_result(category: str, confidence: str = "0.95") -> str:
    return (
        f'{{"category": "{category}", "confidence": {confidence}, '
        f'"runner_up": "groceries", "runner_up_confidence": 0.10, '
        f'"reason": "evidence"}}'
    )


def build_service(store, *, backend, automation=frozenset(), write_port=None):
    model = QwenCategoriser(backend=backend, vocabulary=VOCAB, model="qwen3:8b")
    return CategorisationService(
        store=store,
        model=model,
        vocabulary=VOCAB,
        baseline_rules=BASELINE,
        calibration_policy=POLICY,
        model_version="qwen3:8b",
        automation=automation,
        write_port=write_port,
        clock=lambda: NOW,
    )


@pytest.mark.anyio
async def test_full_categorisation_lifecycle(tmp_path) -> None:
    path = tmp_path / "lifecycle.sqlite3"
    store = SQLiteDerivedState(path)
    store.migrate_up()
    write_port = RecordingWritePort()
    service = build_service(
        store,
        backend=QueueBackend([model_result("dining")]),
        automation=frozenset({"dining"}),
        write_port=write_port,
    )

    # Baseline match bypasses the model entirely.
    baseline = await service.propose(
        source=source("txn-baseline"),
        description="ACME BAKERY",
        kind=TransactionKind.OUTFLOW,
    )
    assert baseline is not None
    assert baseline.method is CategorisationMethod.BASELINE

    # Model proposal with an enabled, calibrated category.
    model = await service.propose(
        source=source("txn-model"),
        description="Neighbourhood Pizzeria",
        kind=TransactionKind.OUTFLOW,
    )
    assert model is not None
    assert model.method is CategorisationMethod.MODEL
    assert model.proposed_category == "dining"

    # Preview binds approval to the exact pending proposal set.
    pending_ids = tuple(p.proposal_id for p in store.list_pending())
    token = service.preview(pending_ids)
    with pytest.raises(PersistenceConflictError):
        service.approve(pending_ids, preview_token="0" * 64)

    approved = service.approve(pending_ids, preview_token=token)
    assert all(proposal.status is ProposalStatus.APPROVED for proposal in approved)
    assert write_port.applied == sorted(pending_ids)

    # Reversal is idempotent and restores the prior category through the port.
    assert service.reverse(token) == len(pending_ids)
    assert write_port.reversed == [token]
    assert service.reverse(token) == 0

    store.close()


@pytest.mark.anyio
async def test_unavailable_model_degrades_to_reviewable_abstention(tmp_path) -> None:
    store = SQLiteDerivedState(tmp_path / "abstention.sqlite3")
    store.migrate_up()
    service = build_service(store, backend=UnavailableBackend())

    proposal = await service.propose(
        source=source("txn-abstain"),
        description="Some Cafe",
        kind=TransactionKind.OUTFLOW,
    )
    assert proposal is not None
    assert proposal.method is CategorisationMethod.ABSTAIN
    assert proposal.proposed_category is None
    assert proposal in store.list_pending()

    # The owner can still review and correct the abstention.
    correction = service.record_correction(
        proposal.proposal_id, approved_category="dining"
    )
    assert correction.approved_category == "dining"
    store.close()


@pytest.mark.anyio
async def test_changed_upstream_produces_new_proposal_and_old_stays_reviewable(
    tmp_path,
) -> None:
    store = SQLiteDerivedState(tmp_path / "change.sqlite3")
    store.migrate_up()
    service = build_service(store, backend=QueueBackend([model_result("dining")]))

    first = await service.propose(
        source=source("txn-change", revision="rev-1"),
        description="Some Cafe",
        kind=TransactionKind.OUTFLOW,
    )
    second = await service.propose(
        source=source("txn-change", revision="rev-2"),
        description="Some Cafe",
        kind=TransactionKind.OUTFLOW,
    )
    assert first is not None and second is not None
    assert first.proposal_id != second.proposal_id
    assert len(store.list_pending()) == 2
    store.close()


@pytest.mark.anyio
async def test_proposals_survive_restart(tmp_path) -> None:
    path = tmp_path / "restart.sqlite3"
    store = SQLiteDerivedState(path)
    store.migrate_up()
    service = build_service(store, backend=QueueBackend([model_result("dining")]))

    proposal = await service.propose(
        source=source("txn-restart"),
        description="ACME BAKERY",
        kind=TransactionKind.OUTFLOW,
    )
    assert proposal is not None
    store.close()

    reopened = SQLiteDerivedState(path)
    reopened.migrate_up()
    assert proposal in reopened.list_pending()
    reopened.close()
