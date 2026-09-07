from datetime import UTC, datetime
from pathlib import Path

import pytest

from finance_assistant.categorisation_runtime import (
    CategorySuggestionRunner,
    CategorySuggestionSettings,
)
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

    with pytest.raises(ValueError, match="between 1 and 100"):
        CategorySuggestionSettings.from_environment(
            {**environment, "PFA_CATEGORY_SUGGESTION_BATCH_SIZE": "101"}
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
    state.close()
