"""Deterministic merchant features, baseline rules, and abstention policy.

This module is the deterministic core of Phase 4 categorisation. It owns no
I/O and no model calls. Merchant text is treated as untrusted data: it is
normalized deterministically, never interpreted as instructions, and only the
normalized, bounded form reaches downstream boundaries.
"""

from __future__ import annotations

import hashlib
import json
import unicodedata
from decimal import Decimal
from enum import StrEnum

from pydantic import Field, model_validator

from finance_assistant.domain.models import StrictModel

MERCHANT_MAX_LENGTH = 200
REASON_MAX_LENGTH = 256
DEFAULT_POLICY_VERSION = "2026-09-v1"
DEFAULT_VOCABULARY_VERSION = "2026-09-v1"

# Character categories that carry instruction-like or non-rendering structure
# (control, format, surrogate, private-use, and unassigned code points). These
# are stripped so a merchant string such as "ACME\u200b<script>" becomes plain
# data instead of a channel for control or injection structure.
_STRIPPED_UNICODE_CATEGORIES = frozenset({"Cc", "Cf", "Cs", "Co", "Cn"})


class CategorisationMethod(StrEnum):
    """How a category was proposed."""

    BASELINE = "baseline"
    MODEL = "model"
    ABSTAIN = "abstain"
    UNSUPPORTED = "unsupported"


class BaselineRuleKind(StrEnum):
    """Precedence tier for deterministic merchant rules."""

    EXACT = "exact"
    NORMALIZED = "normalized"


class CategoryVocabulary(StrictModel):
    """Versioned set of approved categories; a model cannot invent categories."""

    version: str = Field(
        default=DEFAULT_VOCABULARY_VERSION, min_length=1, max_length=64
    )
    categories: tuple[str, ...]

    @model_validator(mode="after")
    def validate_categories(self) -> CategoryVocabulary:
        if not self.categories:
            raise ValueError("vocabulary must contain at least one category")
        for category in self.categories:
            if not category or category != category.strip():
                raise ValueError("category cannot be empty or padded with whitespace")
        if len(set(self.categories)) != len(self.categories):
            raise ValueError("vocabulary contains duplicate categories")
        return self

    def __contains__(self, category: str) -> bool:
        return category in self.categories


class BaselineRule(StrictModel):
    """One versioned merchant rule mapping to an approved category."""

    kind: BaselineRuleKind
    merchant: str = Field(min_length=1, max_length=MERCHANT_MAX_LENGTH)
    category: str = Field(min_length=1, max_length=128)


class BaselineRuleSet(StrictModel):
    """Versioned deterministic rules applied before any model call."""

    version: str = Field(default=DEFAULT_POLICY_VERSION, min_length=1, max_length=64)
    rules: tuple[BaselineRule, ...] = ()


class MerchantFeatures(StrictModel):
    """Bounded features derived only from one transaction description."""

    exact: str = Field(min_length=1, max_length=MERCHANT_MAX_LENGTH)
    normalized: str = Field(min_length=1, max_length=MERCHANT_MAX_LENGTH)

    @property
    def feature_hash(self) -> str:
        """Return a stable hash of the bounded, normalized feature set."""

        encoded = json.dumps(
            self.model_dump(mode="json"), sort_keys=True, separators=(",", ":")
        ).encode()
        return hashlib.sha256(encoded).hexdigest()


def normalize_merchant(value: str) -> str:
    """Normalize untrusted merchant text deterministically and bound its length."""

    text = unicodedata.normalize("NFKC", value)
    text = "".join(
        character
        for character in text
        if unicodedata.category(character) not in _STRIPPED_UNICODE_CATEGORIES
    )
    return " ".join(text.split()).lower()[:MERCHANT_MAX_LENGTH]


def extract_features(description: str) -> MerchantFeatures:
    """Derive bounded features from one description or fail closed on empty input."""

    exact = description.strip()
    if not exact:
        raise ValueError("merchant description cannot be empty")
    return MerchantFeatures(
        exact=exact[:MERCHANT_MAX_LENGTH],
        normalized=normalize_merchant(description),
    )


class CategorisationOutcome(StrictModel):
    """Explainable result from one categorisation stage."""

    method: CategorisationMethod
    category: str | None = None
    confidence: Decimal | None = None
    runner_up: str | None = None
    runner_up_confidence: Decimal | None = None
    reasons: tuple[str, ...] = ()


class ModelCategorisation(StrictModel):
    """Validated structured output returned by the local model."""

    category: str = Field(min_length=1, max_length=128)
    confidence: Decimal = Field(ge=0, le=1)
    runner_up: str | None = Field(default=None, min_length=1, max_length=128)
    runner_up_confidence: Decimal = Field(default=Decimal("0"), ge=0, le=1)
    reason: str = Field(min_length=1, max_length=REASON_MAX_LENGTH)


def baseline_categorise(
    features: MerchantFeatures,
    *,
    rules: BaselineRuleSet,
    vocabulary: CategoryVocabulary,
    supported: bool,
) -> CategorisationOutcome | None:
    """Apply deterministic rules; return ``None`` to fall through to the model."""

    if not supported:
        return CategorisationOutcome(
            method=CategorisationMethod.UNSUPPORTED,
            reasons=("transaction kind is not categorisable",),
        )
    for kind, needle in (
        (BaselineRuleKind.EXACT, features.exact),
        (BaselineRuleKind.NORMALIZED, features.normalized),
    ):
        matched = _match_rules(rules.rules, kind, needle, vocabulary)
        if matched is not None:
            return matched
    return None


def _match_rules(
    rules: tuple[BaselineRule, ...],
    kind: BaselineRuleKind,
    needle: str,
    vocabulary: CategoryVocabulary,
) -> CategorisationOutcome | None:
    categories = {
        rule.category for rule in rules if rule.kind is kind and rule.merchant == needle
    }
    if not categories:
        return None
    if len(categories) > 1:
        return CategorisationOutcome(
            method=CategorisationMethod.ABSTAIN,
            reasons=(f"conflicting {kind.value} merchant rules",),
        )
    category = next(iter(categories))
    if category not in vocabulary:
        return CategorisationOutcome(
            method=CategorisationMethod.ABSTAIN,
            reasons=("baseline rule names an unapproved category",),
        )
    return CategorisationOutcome(
        method=CategorisationMethod.BASELINE,
        category=category,
        reasons=(f"matched {kind.value} merchant rule",),
    )


class CalibrationPolicy(StrictModel):
    """Versioned precision and abstention targets approved by decision issue #20."""

    version: str = Field(default=DEFAULT_POLICY_VERSION, min_length=1, max_length=64)
    min_holdout_examples: int = Field(default=100, ge=1)
    min_per_category_examples: int = Field(default=20, ge=1)
    target_precision: Decimal = Field(default=Decimal("0.95"), gt=0, le=1)
    automation_confidence: Decimal = Field(default=Decimal("0.92"), ge=0, le=1)
    ambiguity_margin: Decimal = Field(default=Decimal("0.15"), ge=0, le=1)


class CalibrationExample(StrictModel):
    """An owner-approved correction eligible for the frozen evaluation set."""

    feature_hash: str = Field(pattern="^[0-9a-f]{64}$")
    normalized_merchant: str = Field(min_length=1, max_length=MERCHANT_MAX_LENGTH)
    approved_category: str = Field(min_length=1, max_length=128)
    source_identity: str = Field(min_length=1, max_length=256)

    @model_validator(mode="after")
    def validate_fields(self) -> CalibrationExample:
        if self.approved_category != self.approved_category.strip():
            raise ValueError("approved_category cannot have surrounding whitespace")
        if self.source_identity != self.source_identity.strip():
            raise ValueError("source_identity cannot have surrounding whitespace")
        return self


class CalibrationSplit(StrictModel):
    """Identity-separated buckets that prevent exact-example leakage."""

    train: tuple[CalibrationExample, ...] = ()
    calibration: tuple[CalibrationExample, ...] = ()
    holdout: tuple[CalibrationExample, ...] = ()


def _identity_bucket(identity: str, bucket_count: int) -> int:
    digest = hashlib.sha256(identity.encode()).digest()
    return digest[0] % bucket_count


def split_by_identity(
    examples: tuple[CalibrationExample, ...],
    *,
    bucket_count: int = 3,
) -> CalibrationSplit:
    """Split examples so one source identity never spans buckets."""

    if bucket_count < 3:
        raise ValueError("bucket_count must be at least 3")
    buckets: list[list[CalibrationExample]] = [[] for _ in range(bucket_count)]
    for example in examples:
        buckets[_identity_bucket(example.source_identity, bucket_count)].append(example)
    return CalibrationSplit(
        train=tuple(buckets[0]),
        calibration=tuple(buckets[1]),
        holdout=tuple(buckets[2]),
    )


class PredictionRecord(StrictModel):
    """One holdout example paired with the model's predicted category."""

    feature_hash: str = Field(pattern="^[0-9a-f]{64}$")
    approved_category: str = Field(min_length=1, max_length=128)
    predicted_category: str | None = Field(default=None, min_length=1, max_length=128)


class CategoryMetric(StrictModel):
    """Per-category precision evidence for the automation gate."""

    category: str = Field(min_length=1, max_length=128)
    support: int = Field(ge=0)
    correct: int = Field(ge=0)
    precision: Decimal = Field(ge=0, le=1)

    @model_validator(mode="after")
    def validate_metric(self) -> CategoryMetric:
        if self.correct > self.support:
            raise ValueError("correct cannot exceed support")
        return self


class CalibrationReport(StrictModel):
    """Explains whether automation may be enabled and for which categories."""

    policy_version: str
    total_examples: int = Field(ge=0)
    total_support: int = Field(ge=0)
    coverage: Decimal = Field(ge=0, le=1)
    overall_precision: Decimal = Field(ge=0, le=1)
    per_category: tuple[CategoryMetric, ...] = ()


def _precision(correct: int, support: int) -> Decimal:
    if support == 0:
        return Decimal(0)
    return Decimal(correct) / Decimal(support)


def compute_calibration(
    records: tuple[PredictionRecord, ...],
    *,
    policy: CalibrationPolicy,
) -> CalibrationReport:
    """Compute precision, coverage, support, and per-category failures."""

    total = len(records)
    support = sum(1 for record in records if record.predicted_category is not None)
    correct = sum(
        1
        for record in records
        if record.predicted_category is not None
        and record.predicted_category == record.approved_category
    )
    by_category: dict[str, tuple[int, int]] = {}
    for record in records:
        predicted = record.predicted_category
        if predicted is None:
            continue
        current_support, current_correct = by_category.get(predicted, (0, 0))
        by_category[predicted] = (
            current_support + 1,
            current_correct + (1 if predicted == record.approved_category else 0),
        )
    metrics = tuple(
        CategoryMetric(
            category=category,
            support=category_support,
            correct=category_correct,
            precision=_precision(category_correct, category_support),
        )
        for category, (category_support, category_correct) in sorted(
            by_category.items()
        )
    )
    return CalibrationReport(
        policy_version=policy.version,
        total_examples=total,
        total_support=support,
        coverage=_precision(support, total),
        overall_precision=_precision(correct, support),
        per_category=metrics,
    )


def automation_categories(
    report: CalibrationReport,
    *,
    policy: CalibrationPolicy,
) -> frozenset[str]:
    """Return the categories whose frozen holdout evidence clears every gate."""

    if report.total_examples < policy.min_holdout_examples:
        return frozenset()
    return frozenset(
        metric.category
        for metric in report.per_category
        if metric.support >= policy.min_per_category_examples
        and metric.precision >= policy.target_precision
    )


def automation_decision(
    result: ModelCategorisation,
    *,
    policy: CalibrationPolicy,
    automation: frozenset[str],
) -> CategorisationOutcome:
    """Convert a calibrated model result into an automation decision."""

    if result.category not in automation:
        return CategorisationOutcome(
            method=CategorisationMethod.ABSTAIN,
            reasons=("category is not enabled for automation",),
        )
    if result.confidence < policy.automation_confidence:
        return CategorisationOutcome(
            method=CategorisationMethod.ABSTAIN,
            confidence=result.confidence,
            reasons=("confidence is below the automation threshold",),
        )
    margin = result.confidence - result.runner_up_confidence
    if result.runner_up is not None and margin < policy.ambiguity_margin:
        return CategorisationOutcome(
            method=CategorisationMethod.ABSTAIN,
            confidence=result.confidence,
            runner_up=result.runner_up,
            runner_up_confidence=result.runner_up_confidence,
            reasons=("runner-up alternative is too close to the top category",),
        )
    return CategorisationOutcome(
        method=CategorisationMethod.MODEL,
        category=result.category,
        confidence=result.confidence,
        runner_up=result.runner_up,
        runner_up_confidence=result.runner_up_confidence,
        reasons=(result.reason,),
    )
