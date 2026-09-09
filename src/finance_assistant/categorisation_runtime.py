"""Bounded local-Qwen suggestion generation for uncategorised transactions."""

from __future__ import annotations

import hashlib
import json
import math
from collections import Counter, defaultdict
from collections.abc import Mapping
from dataclasses import dataclass

from finance_assistant.categorisation_service import (
    DEFAULT_PROMPT_VERSION,
    CategorisationService,
    ProposalStatus,
    QwenCategoriser,
)
from finance_assistant.domain.canonical import (
    CanonicalTransaction,
    SourceEntity,
    SourceSystem,
    TransactionKind,
)
from finance_assistant.domain.categorisation import (
    MAX_CATEGORIES,
    BaselineRule,
    BaselineRuleKind,
    BaselineRuleSet,
    CalibrationPolicy,
    CategoryVocabulary,
    normalize_merchant,
)
from finance_assistant.integrations.ollama import OllamaClient, OllamaError
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
MAX_HISTORY_EXAMPLES = 48
MAX_RETRIEVAL_EXAMPLES = 256
MAX_RETRIEVED_PER_QUERY = 12
MAX_RETRIEVED_PER_CATEGORY = 3


@dataclass(frozen=True, slots=True)
class CategoryHistoryContext:
    """Bounded, conflict-safe supervision derived from Firefly's current state."""

    vocabulary: CategoryVocabulary
    rules: BaselineRuleSet
    examples: tuple[tuple[str, str], ...]
    retrieval_examples: tuple[tuple[str, str], ...]
    version: str


def build_category_history_context(
    transactions: tuple[CanonicalTransaction, ...],
) -> CategoryHistoryContext:
    """Turn categorized Firefly history into local-only rules and model examples."""

    category_counts: Counter[str] = Counter()
    merchant_categories: dict[str, Counter[str]] = defaultdict(Counter)
    for transaction in transactions:
        if (
            transaction.kind is TransactionKind.TRANSFER
            or transaction.category is None
            or not transaction.description
        ):
            continue
        try:
            merchant = normalize_merchant(transaction.description)
        except ValueError:
            continue
        category_counts[transaction.category] += 1
        merchant_categories[merchant][transaction.category] += 1

    ordered_history_categories = tuple(
        category
        for category, _count in sorted(
            category_counts.items(), key=lambda item: (-item[1], item[0].casefold())
        )
        if category not in DEFAULT_CATEGORIES
    )
    categories = (*DEFAULT_CATEGORIES, *ordered_history_categories)
    categories = categories[:MAX_CATEGORIES]
    allowed = frozenset(categories)

    unambiguous = tuple(
        (merchant, next(iter(counts)), sum(counts.values()))
        for merchant, counts in merchant_categories.items()
        if len(counts) == 1 and next(iter(counts)) in allowed
    )
    rules = tuple(
        BaselineRule(
            kind=BaselineRuleKind.NORMALIZED,
            merchant=merchant,
            category=category,
        )
        for merchant, category, _count in sorted(unambiguous)
    )

    by_category: dict[str, list[tuple[str, int]]] = defaultdict(list)
    for merchant, category, count in unambiguous:
        by_category[category].append((merchant, count))
    for candidates in by_category.values():
        candidates.sort(key=lambda item: (-item[1], item[0]))
    examples: list[tuple[str, str]] = []
    active_categories = sorted(by_category, key=str.casefold)
    while active_categories and len(examples) < MAX_HISTORY_EXAMPLES:
        remaining: list[str] = []
        for category in active_categories:
            candidates = by_category[category]
            merchant, _count = candidates.pop(0)
            examples.append((merchant, category))
            if candidates:
                remaining.append(category)
            if len(examples) == MAX_HISTORY_EXAMPLES:
                break
        active_categories = remaining

    fingerprint = hashlib.sha256(
        json.dumps(
            {
                "categories": categories,
                "rules": [(r.merchant, r.category) for r in rules],
            },
            ensure_ascii=True,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
    ).hexdigest()[:12]
    version = f"firefly-history-v1-{fingerprint}"
    return CategoryHistoryContext(
        vocabulary=CategoryVocabulary(version=version, categories=categories),
        rules=BaselineRuleSet(version=version, rules=rules),
        examples=tuple(examples),
        retrieval_examples=tuple(
            (merchant, category)
            for merchant, category, _count in sorted(
                unambiguous,
                key=lambda item: (-item[2], item[0], item[1].casefold()),
            )[:MAX_RETRIEVAL_EXAMPLES]
        ),
        version=version,
    )


def _cosine(left: tuple[float, ...], right: tuple[float, ...]) -> float:
    dot = math.fsum(a * b for a, b in zip(left, right, strict=True))
    left_norm = math.sqrt(math.fsum(value * value for value in left))
    right_norm = math.sqrt(math.fsum(value * value for value in right))
    if left_norm == 0 or right_norm == 0:
        return -1.0
    return dot / (left_norm * right_norm)


async def retrieve_category_examples(
    client: OllamaClient,
    *,
    queries: tuple[str, ...],
    corpus: tuple[tuple[str, str], ...],
) -> dict[str, tuple[tuple[str, str], ...]]:
    """Retrieve a small, category-diverse local context for each merchant."""

    if not queries or not corpus:
        return {}
    unique_queries = tuple(dict.fromkeys(queries))
    texts = tuple(merchant for merchant, _category in corpus) + unique_queries
    vectors = await client.embed(texts)
    corpus_vectors = vectors[: len(corpus)]
    selected: dict[str, tuple[tuple[str, str], ...]] = {}
    for query, query_vector in zip(unique_queries, vectors[len(corpus) :], strict=True):
        ranked = sorted(
            zip(corpus, corpus_vectors, strict=True),
            key=lambda item: (-_cosine(query_vector, item[1]), item[0][0]),
        )
        category_counts: Counter[str] = Counter()
        examples: list[tuple[str, str]] = []
        for example, _vector in ranked:
            if category_counts[example[1]] >= MAX_RETRIEVED_PER_CATEGORY:
                continue
            examples.append(example)
            category_counts[example[1]] += 1
            if len(examples) == MAX_RETRIEVED_PER_QUERY:
                break
        selected[query] = tuple(examples)
    return selected


@dataclass(frozen=True, slots=True)
class CategorySuggestionSettings:
    base_url: str
    model: str
    batch_size: int = 20
    embedding_model: str | None = None

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
        embedding_model = environment.get("PFA_OLLAMA_EMBEDDING_MODEL", "").strip()
        if len(embedding_model) > 128:
            raise ValueError("PFA_OLLAMA_EMBEDDING_MODEL is oversized")
        return cls(
            base_url=base_url,
            model=model,
            batch_size=batch_size,
            embedding_model=embedding_model or None,
        )


class CategorySuggestionRunner:
    """Suggest categories locally; never approve or write them upstream."""

    def __init__(
        self, *, state: SQLiteDerivedState, settings: CategorySuggestionSettings
    ) -> None:
        self._state = state
        self._settings = settings

    async def __call__(self) -> int:
        transactions = self._state.list_transactions()
        history = build_category_history_context(transactions)
        prompt_version = f"{DEFAULT_PROMPT_VERSION}+{history.version}"
        current = {
            (
                transaction.source.system,
                transaction.source.entity,
                transaction.source.external_id,
            ): transaction
            for transaction in transactions
        }
        regenerate_sources: set[tuple[SourceSystem, SourceEntity, str]] = set()
        for proposal in self._state.list_pending():
            source_key = (
                proposal.source.system,
                proposal.source.entity,
                proposal.source.external_id,
            )
            transaction = current.get(source_key)
            if transaction is not None and transaction.category is not None:
                self._state.resolve_proposal(
                    proposal.proposal_id, status=ProposalStatus.SUPERSEDED
                )
            elif (
                proposal.model_version != self._settings.model
                or proposal.prompt_version != prompt_version
            ):
                self._state.resolve_proposal(
                    proposal.proposal_id, status=ProposalStatus.SUPERSEDED
                )
                regenerate_sources.add(source_key)
        candidates = tuple(
            transaction
            for transaction in sorted(
                transactions,
                key=lambda item: item.occurred_at,
                reverse=True,
            )
            if transaction.kind is not TransactionKind.TRANSFER
            and transaction.category is None
            and transaction.description
            and (
                (
                    transaction.source.system,
                    transaction.source.entity,
                    transaction.source.external_id,
                )
                in regenerate_sources
                or not self._state.category_proposal_exists(transaction.source)
            )
        )[: self._settings.batch_size]
        if not candidates:
            return 0
        normalized_candidates = tuple(
            normalize_merchant(transaction.description or "")
            for transaction in candidates
        )
        retrieved: dict[str, tuple[tuple[str, str], ...]] = {}
        if self._settings.embedding_model and history.retrieval_examples:
            try:
                async with OllamaClient(
                    base_url=self._settings.base_url,
                    model=self._settings.embedding_model,
                ) as embedding_client:
                    retrieved = await retrieve_category_examples(
                        embedding_client,
                        queries=normalized_candidates,
                        corpus=history.retrieval_examples,
                    )
            except OllamaError, ValueError:
                retrieved = {}
        async with OllamaClient(
            base_url=self._settings.base_url, model=self._settings.model
        ) as backend:
            for transaction, normalized in zip(
                candidates, normalized_candidates, strict=True
            ):
                service = CategorisationService(
                    store=self._state,
                    model=QwenCategoriser(
                        backend=backend,
                        vocabulary=history.vocabulary,
                        model=self._settings.model,
                        prompt_version=prompt_version,
                        category_examples=retrieved.get(normalized, history.examples),
                    ),
                    vocabulary=history.vocabulary,
                    baseline_rules=history.rules,
                    calibration_policy=CalibrationPolicy(),
                    model_version=self._settings.model,
                    prompt_version=prompt_version,
                    review_model_suggestions=True,
                )
                await service.propose(
                    source=transaction.source,
                    description=transaction.description or "",
                    kind=transaction.kind,
                    original_category=transaction.category,
                )
        return len(candidates)
