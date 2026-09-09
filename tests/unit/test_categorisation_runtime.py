from datetime import UTC, datetime
from pathlib import Path

import pytest

from finance_assistant.categorisation_runtime import (
    CategorySuggestionRunner,
    CategorySuggestionSettings,
    build_category_history_context,
    retrieve_category_examples,
)
from finance_assistant.categorisation_service import ProposalStatus
from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceEntity,
    SourceReference,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.models import Currency, Money
from finance_assistant.persistence.sqlite import SQLiteDerivedState


def test_category_suggestions_are_disabled_by_default() -> None:
    assert CategorySuggestionSettings.from_environment({}) is None


def test_category_suggestion_settings_are_bounded() -> None:
    environment = {
        "PFA_CATEGORY_SUGGESTIONS_ENABLED": "true",
        "PFA_OLLAMA_BASE_URL": "http://localhost:11434",
        "PFA_OLLAMA_MODEL": "synthetic-qwen",
    }
    settings = CategorySuggestionSettings.from_environment(environment)
    assert settings is not None
    assert settings.batch_size == 20
    assert settings.embedding_model is None

    rag_settings = CategorySuggestionSettings.from_environment(
        {**environment, "PFA_OLLAMA_EMBEDDING_MODEL": "synthetic-embed"}
    )
    assert rag_settings is not None
    assert rag_settings.embedding_model == "synthetic-embed"

    with pytest.raises(ValueError, match="between 1 and 100"):
        CategorySuggestionSettings.from_environment(
            {**environment, "PFA_CATEGORY_SUGGESTION_BATCH_SIZE": "101"}
        )
    with pytest.raises(ValueError, match="must be true or false"):
        CategorySuggestionSettings.from_environment(
            {**environment, "PFA_CATEGORY_SUGGESTIONS_ENABLED": "yes"}
        )
    with pytest.raises(ValueError, match="URL and model"):
        CategorySuggestionSettings.from_environment(
            {
                "PFA_CATEGORY_SUGGESTIONS_ENABLED": "true",
                "PFA_OLLAMA_BASE_URL": "http://localhost:11434",
            }
        )
    with pytest.raises(ValueError, match="must be an integer"):
        CategorySuggestionSettings.from_environment(
            {**environment, "PFA_CATEGORY_SUGGESTION_BATCH_SIZE": "many"}
        )
    with pytest.raises(ValueError, match="oversized"):
        CategorySuggestionSettings.from_environment(
            {**environment, "PFA_OLLAMA_EMBEDDING_MODEL": "x" * 129}
        )


@pytest.mark.anyio
async def test_runner_creates_review_only_local_suggestion(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class FakeOllama:
        def __init__(self, **_kwargs: object) -> None:
            pass

        async def __aenter__(self) -> FakeOllama:
            return self

        async def __aexit__(self, *_args: object) -> None:
            pass

        async def generate(self, _prompt: str, *, system: str | None = None) -> str:
            assert system is not None
            return (
                '{"category":"Groceries","confidence":0.96,'
                '"runner_up":"Shopping","runner_up_confidence":0.02,'
                '"reason":"synthetic evidence"}'
            )

    monkeypatch.setattr(
        "finance_assistant.categorisation_runtime.OllamaClient", FakeOllama
    )
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    now = datetime(2026, 9, 7, tzinfo=UTC)
    transaction = CanonicalTransaction(
        transaction_id="firefly:synthetic-journal",
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id="synthetic-journal",
            source_updated_at=now,
        ),
        account_id="firefly:synthetic-account",
        occurred_at=now,
        kind=TransactionKind.OUTFLOW,
        amount=Money(amount="12.34", currency=Currency(code="EUR", minor_unit=2)),
        description="Synthetic market",
    )
    state.upsert(transaction, observed_at=now)
    settings = CategorySuggestionSettings(
        base_url="http://localhost:11434", model="synthetic-qwen", batch_size=20
    )

    assert await CategorySuggestionRunner(state=state, settings=settings)() == 1
    proposals = state.list_pending()
    assert len(proposals) == 1
    assert proposals[0].proposed_category == "Groceries"
    assert await CategorySuggestionRunner(state=state, settings=settings)() == 0
    state.upsert(
        transaction.model_copy(update={"category": "Groceries"}), observed_at=now
    )
    assert await CategorySuggestionRunner(state=state, settings=settings)() == 0
    assert state.list_pending() == ()
    assert state.list_proposals()[0].status is ProposalStatus.SUPERSEDED
    state.close()


def synthetic_transaction(
    *, external_id: str, description: str, category: str | None
) -> CanonicalTransaction:
    now = datetime(2026, 9, 7, tzinfo=UTC)
    return CanonicalTransaction(
        transaction_id=f"firefly:{external_id}",
        source=SourceReference(
            system=SourceSystem.FIREFLY_III,
            entity=SourceEntity.TRANSACTION,
            external_id=external_id,
            source_updated_at=now,
        ),
        account_id="firefly:synthetic-account",
        occurred_at=now,
        kind=TransactionKind.OUTFLOW,
        amount=Money(amount="1.00", currency=Currency(code="EUR", minor_unit=2)),
        description=description,
        category=category,
    )


def test_firefly_history_builds_dynamic_vocabulary_and_conflict_safe_rules() -> None:
    history = build_category_history_context(
        (
            synthetic_transaction(
                external_id="1", description="Synthetic Bakery 1234", category="Food"
            ),
            synthetic_transaction(
                external_id="2", description="Synthetic Bakery 5678", category="Food"
            ),
            synthetic_transaction(
                external_id="3", description="Conflicted Shop", category="Shopping"
            ),
            synthetic_transaction(
                external_id="4", description="Conflicted Shop", category="Groceries"
            ),
        )
    )

    assert "Food" in history.vocabulary
    assert [(rule.merchant, rule.category) for rule in history.rules.rules] == [
        ("synthetic bakery number", "Food")
    ]
    assert ("synthetic bakery number", "Food") in history.examples
    assert all(merchant != "conflicted shop" for merchant, _ in history.examples)
    assert history.retrieval_examples == (("synthetic bakery number", "Food"),)


@pytest.mark.anyio
async def test_local_rag_retrieves_similar_diverse_category_examples() -> None:
    class FakeEmbeddings:
        async def embed(self, inputs: tuple[str, ...]) -> tuple[tuple[float, ...], ...]:
            assert inputs == ("bakery one", "bakery two", "airline", "new bakery")
            return (
                (1.0, 0.0),
                (0.9, 0.1),
                (0.0, 1.0),
                (1.0, 0.0),
            )

    retrieved = await retrieve_category_examples(
        FakeEmbeddings(),  # type: ignore[arg-type]
        queries=("new bakery",),
        corpus=(
            ("bakery one", "Food"),
            ("bakery two", "Food"),
            ("airline", "Travel"),
        ),
    )

    assert retrieved["new bakery"][:2] == (
        ("bakery one", "Food"),
        ("bakery two", "Food"),
    )


@pytest.mark.anyio
async def test_runner_reuses_a_firefly_category_without_calling_the_model(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class ModelMustNotRun:
        def __init__(self, **_kwargs: object) -> None:
            pass

        async def __aenter__(self) -> ModelMustNotRun:
            return self

        async def __aexit__(self, *_args: object) -> None:
            pass

        async def generate(self, _prompt: str, *, system: str | None = None) -> str:
            raise AssertionError("an exact reviewed merchant should use the baseline")

    monkeypatch.setattr(
        "finance_assistant.categorisation_runtime.OllamaClient", ModelMustNotRun
    )
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    now = datetime(2026, 9, 7, tzinfo=UTC)
    state.upsert(
        synthetic_transaction(
            external_id="reviewed",
            description="Synthetic Bakery 1234",
            category="Food",
        ),
        observed_at=now,
    )
    state.upsert(
        synthetic_transaction(
            external_id="new",
            description="Synthetic Bakery 5678",
            category=None,
        ),
        observed_at=now,
    )

    count = await CategorySuggestionRunner(
        state=state,
        settings=CategorySuggestionSettings(
            base_url="http://localhost:11434", model="synthetic-qwen"
        ),
    )()

    assert count == 1
    proposal = state.list_pending()[0]
    assert proposal.proposed_category == "Food"
    assert proposal.method.value == "baseline"
    state.close()


@pytest.mark.anyio
async def test_runner_regenerates_pending_suggestions_after_model_upgrade(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class FakeOllama:
        def __init__(self, **_kwargs: object) -> None:
            pass

        async def __aenter__(self) -> FakeOllama:
            return self

        async def __aexit__(self, *_args: object) -> None:
            pass

        async def generate(self, _prompt: str, *, system: str | None = None) -> str:
            return (
                '{"category":"Groceries","confidence":0.96,'
                '"runner_up":"Shopping","runner_up_confidence":0.02,'
                '"reason":"synthetic evidence"}'
            )

    monkeypatch.setattr(
        "finance_assistant.categorisation_runtime.OllamaClient", FakeOllama
    )
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    now = datetime(2026, 9, 7, tzinfo=UTC)
    state.upsert(
        synthetic_transaction(
            external_id="upgrade", description="Synthetic Unknown", category=None
        ),
        observed_at=now,
    )
    assert (
        await CategorySuggestionRunner(
            state=state,
            settings=CategorySuggestionSettings(
                base_url="http://localhost:11434", model="small-model"
            ),
        )()
        == 1
    )
    assert (
        await CategorySuggestionRunner(
            state=state,
            settings=CategorySuggestionSettings(
                base_url="http://localhost:11434", model="larger-model"
            ),
        )()
        == 1
    )

    proposals = state.list_proposals()
    assert {item.model_version: item.status for item in proposals} == {
        "small-model": ProposalStatus.SUPERSEDED,
        "larger-model": ProposalStatus.PENDING,
    }
    state.close()


@pytest.mark.anyio
async def test_runner_uses_local_retrieval_context_when_configured(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls: list[tuple[str, tuple[str, ...] | str]] = []

    class FakeOllama:
        def __init__(self, *, model: str, **_kwargs: object) -> None:
            self.model = model

        async def __aenter__(self) -> FakeOllama:
            return self

        async def __aexit__(self, *_args: object) -> None:
            pass

        async def embed(self, inputs: tuple[str, ...]) -> tuple[tuple[float, ...], ...]:
            calls.append((self.model, inputs))
            return ((1.0, 0.0), (1.0, 0.0))

        async def generate(self, prompt: str, *, system: str | None = None) -> str:
            calls.append((self.model, prompt))
            assert '"merchant":"reviewed bakery"' in prompt
            return (
                '{"category":"Food","confidence":0.96,'
                '"runner_up":"Dining","runner_up_confidence":0.02,'
                '"reason":"retrieved synthetic evidence"}'
            )

    monkeypatch.setattr(
        "finance_assistant.categorisation_runtime.OllamaClient", FakeOllama
    )
    state = SQLiteDerivedState(tmp_path / "state.sqlite3")
    state.migrate_up()
    now = datetime(2026, 9, 7, tzinfo=UTC)
    for transaction in (
        synthetic_transaction(
            external_id="history", description="Reviewed Bakery", category="Food"
        ),
        synthetic_transaction(
            external_id="candidate", description="New Bakery", category=None
        ),
    ):
        state.upsert(transaction, observed_at=now)

    count = await CategorySuggestionRunner(
        state=state,
        settings=CategorySuggestionSettings(
            base_url="http://localhost:11434",
            model="larger-model",
            embedding_model="synthetic-embed",
        ),
    )()

    assert count == 1
    assert calls[0] == (
        "synthetic-embed",
        ("reviewed bakery", "new bakery"),
    )
    assert state.list_pending()[0].proposed_category == "Food"
    state.close()
