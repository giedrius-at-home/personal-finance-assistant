"""Bounded local-Qwen suggestion generation for uncategorised transactions."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

from finance_assistant.categorisation_service import (
    CategorisationService,
    QwenCategoriser,
)
from finance_assistant.domain.canonical import TransactionKind
from finance_assistant.domain.categorisation import (
    BaselineRuleSet,
    CalibrationPolicy,
    CategoryVocabulary,
)
from finance_assistant.integrations.ollama import OllamaClient
from finance_assistant.persistence.sqlite import SQLiteDerivedState

DEFAULT_CATEGORIES = (
    "Groceries",
    "Dining",
    "Transport",
    "Shopping",
    "Subscriptions",
    "Travel",
    "Housing",
    "Utilities",
    "Health",
    "Education",
    "Entertainment",
    "Fees",
    "Income",
    "Investments",
    "Cash",
)


@dataclass(frozen=True, slots=True)
class CategorySuggestionSettings:
    base_url: str
    model: str
    batch_size: int = 20

    @classmethod
    def from_environment(
        cls, environment: Mapping[str, str]
    ) -> CategorySuggestionSettings | None:
        enabled = environment.get(
            "PFA_CATEGORY_SUGGESTIONS_ENABLED", "false"
        ).casefold()
        if enabled not in {"true", "false"}:
            raise ValueError("PFA_CATEGORY_SUGGESTIONS_ENABLED must be true or false")
        if enabled == "false":
            return None
        base_url = environment.get("PFA_OLLAMA_BASE_URL", "").rstrip("/")
        model = environment.get("PFA_OLLAMA_MODEL", "").strip()
        if not base_url or not model:
            raise ValueError(
                "Ollama URL and model are required for category suggestions"
            )
        try:
            batch_size = int(
                environment.get("PFA_CATEGORY_SUGGESTION_BATCH_SIZE", "20")
            )
        except ValueError as error:
            raise ValueError(
                "PFA_CATEGORY_SUGGESTION_BATCH_SIZE must be an integer"
            ) from error
        if not 1 <= batch_size <= 100:
            raise ValueError(
                "PFA_CATEGORY_SUGGESTION_BATCH_SIZE must be between 1 and 100"
            )
        return cls(base_url=base_url, model=model, batch_size=batch_size)


class CategorySuggestionRunner:
    """Suggest categories locally; never approve or write them upstream."""

    def __init__(
        self, *, state: SQLiteDerivedState, settings: CategorySuggestionSettings
    ) -> None:
        self._state = state
        self._settings = settings

    async def __call__(self) -> int:
        candidates = tuple(
            transaction
            for transaction in sorted(
                self._state.list_transactions(),
                key=lambda item: item.occurred_at,
                reverse=True,
            )
            if transaction.kind is not TransactionKind.TRANSFER
            and transaction.category is None
            and transaction.description
            and not self._state.category_proposal_exists(transaction.source)
        )[: self._settings.batch_size]
        if not candidates:
            return 0
        vocabulary = CategoryVocabulary(categories=DEFAULT_CATEGORIES)
        async with OllamaClient(
            base_url=self._settings.base_url, model=self._settings.model
        ) as backend:
            service = CategorisationService(
                store=self._state,
                model=QwenCategoriser(
                    backend=backend,
                    vocabulary=vocabulary,
                    model=self._settings.model,
                ),
                vocabulary=vocabulary,
                baseline_rules=BaselineRuleSet(),
                calibration_policy=CalibrationPolicy(),
                model_version=self._settings.model,
                review_model_suggestions=True,
            )
            for transaction in candidates:
                await service.propose(
                    source=transaction.source,
                    description=transaction.description or "",
                    kind=transaction.kind,
                    original_category=transaction.category,
                )
        return len(candidates)
