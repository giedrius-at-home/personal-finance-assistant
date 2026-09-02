"""Deterministic categorisation domain tests using invented merchant strings."""

from decimal import Decimal

import pytest

from finance_assistant.domain.categorisation import (
    BaselineRule,
    BaselineRuleKind,
    BaselineRuleSet,
    CalibrationExample,
    CalibrationPolicy,
    CategorisationMethod,
    CategoryVocabulary,
    ModelCategorisation,
    PredictionRecord,
    automation_categories,
    automation_decision,
    baseline_categorise,
    compute_calibration,
    extract_features,
    normalize_merchant,
    split_by_identity,
)

VOCAB = CategoryVocabulary(
    version="2026-09-v1",
    categories=("dining", "groceries", "transport"),
)


def test_normalize_merchant_strips_controls_collapses_and_lowercases() -> None:
    assert normalize_merchant("  ACME\u200b\t Market\n ") == "acme market"
    # Bidi/control/format characters are removed, never interpreted.
    assert normalize_merchant("\u202eACME<script>\u200d") == "acme<script>"
    # Unicode normalization folds compatibility forms.
    assert normalize_merchant("ＣＡＦＥ") == "cafe"


def test_normalize_merchant_redacts_common_identifiers() -> None:
    normalized = normalize_merchant(
        "Cafe LT601010012345678901 receipt 123456 john@example.test "
        "https://merchant.invalid/order"
    )
    assert normalized == "cafe account receipt number email url"
    assert "123456" not in normalized


def test_normalize_merchant_is_bounded() -> None:
    assert len(normalize_merchant("x" * 500)) == 200


def test_extract_features_derives_stable_hash_and_rejects_empty() -> None:
    features = extract_features("  ACME Market ")
    assert features.exact == "ACME Market"
    assert features.normalized == "acme market"
    assert len(features.feature_hash) == 64
    assert features.feature_hash == extract_features("ACME Market").feature_hash
    with pytest.raises(ValueError):
        extract_features("   ")


def test_vocabulary_rejects_duplicates_and_padding() -> None:
    with pytest.raises(ValueError):
        CategoryVocabulary(categories=("dining", "dining"))
    with pytest.raises(ValueError):
        CategoryVocabulary(categories=(" dining",))
    with pytest.raises(ValueError):
        CategoryVocabulary(categories=())


def test_baseline_exact_precedes_normalized() -> None:
    rules = BaselineRuleSet(
        rules=(
            BaselineRule(
                kind=BaselineRuleKind.NORMALIZED, merchant="acme", category="transport"
            ),
            BaselineRule(
                kind=BaselineRuleKind.EXACT, merchant="ACME", category="dining"
            ),
        )
    )
    outcome = baseline_categorise(
        extract_features("ACME"), rules=rules, vocabulary=VOCAB, supported=True
    )
    assert outcome is not None
    assert outcome.method is CategorisationMethod.BASELINE
    assert outcome.category == "dining"


def test_baseline_conflict_abstains() -> None:
    rules = BaselineRuleSet(
        rules=(
            BaselineRule(
                kind=BaselineRuleKind.EXACT, merchant="ACME", category="dining"
            ),
            BaselineRule(
                kind=BaselineRuleKind.EXACT, merchant="ACME", category="groceries"
            ),
        )
    )
    outcome = baseline_categorise(
        extract_features("ACME"), rules=rules, vocabulary=VOCAB, supported=True
    )
    assert outcome is not None
    assert outcome.method is CategorisationMethod.ABSTAIN


def test_baseline_unapproved_category_abstains() -> None:
    rules = BaselineRuleSet(
        rules=(
            BaselineRule(
                kind=BaselineRuleKind.EXACT, merchant="ACME", category="travel"
            ),
        )
    )
    outcome = baseline_categorise(
        extract_features("ACME"), rules=rules, vocabulary=VOCAB, supported=True
    )
    assert outcome is not None
    assert outcome.method is CategorisationMethod.ABSTAIN


def test_baseline_no_match_returns_none_and_unsupported_fails_closed() -> None:
    assert (
        baseline_categorise(
            extract_features("Unknown"),
            rules=BaselineRuleSet(),
            vocabulary=VOCAB,
            supported=True,
        )
        is None
    )
    unsupported = baseline_categorise(
        extract_features("ACME"),
        rules=BaselineRuleSet(),
        vocabulary=VOCAB,
        supported=False,
    )
    assert unsupported is not None
    assert unsupported.method is CategorisationMethod.UNSUPPORTED


def _prediction(approved: str, predicted: str | None) -> PredictionRecord:
    return PredictionRecord(
        feature_hash="a" * 64,
        approved_category=approved,
        predicted_category=predicted,
    )


def test_compute_calibration_reports_precision_coverage_and_support() -> None:
    report = compute_calibration(
        (
            _prediction("dining", "dining"),
            _prediction("dining", "dining"),
            _prediction("groceries", "dining"),
            _prediction("groceries", None),
        ),
        policy=CalibrationPolicy(),
        vocabulary_version=VOCAB.version,
        model_version="qwen3:8b",
    )
    assert report.total_examples == 4
    assert report.total_support == 3
    assert report.coverage == Decimal("0.75")
    assert report.overall_precision == Decimal(2) / Decimal(3)
    dining = next(m for m in report.per_category if m.category == "dining")
    assert dining.support == 3
    assert dining.correct == 2


def test_automation_categories_gates_on_evidence() -> None:
    policy = CalibrationPolicy(
        min_holdout_examples=3,
        min_per_category_examples=2,
        target_precision=Decimal("0.95"),
    )
    below_precision = compute_calibration(
        (
            _prediction("dining", "dining"),
            _prediction("groceries", "dining"),
            _prediction("transport", "dining"),
        ),
        policy=policy,
        vocabulary_version=VOCAB.version,
        model_version="qwen3:8b",
    )
    assert automation_categories(below_precision, policy=policy) == frozenset()

    at_precision = compute_calibration(
        (
            _prediction("dining", "dining"),
            _prediction("dining", "dining"),
            _prediction("dining", "groceries"),
        ),
        policy=policy,
        vocabulary_version=VOCAB.version,
        model_version="qwen3:8b",
    )
    assert automation_categories(at_precision, policy=policy) == frozenset({"dining"})


def test_automation_decision_applies_confidence_and_margin() -> None:
    policy = CalibrationPolicy()
    automation = frozenset({"dining"})
    good = ModelCategorisation(
        category="dining",
        confidence=Decimal("0.95"),
        runner_up="groceries",
        runner_up_confidence=Decimal("0.10"),
        reason="restaurant",
    )
    decided = automation_decision(good, policy=policy, automation=automation)
    assert decided.method is CategorisationMethod.MODEL
    assert decided.category == "dining"

    low_confidence = good.model_copy(update={"confidence": Decimal("0.80")})
    assert (
        automation_decision(low_confidence, policy=policy, automation=automation).method
        is CategorisationMethod.ABSTAIN
    )

    close_margin = good.model_copy(update={"runner_up_confidence": Decimal("0.90")})
    assert (
        automation_decision(close_margin, policy=policy, automation=automation).method
        is CategorisationMethod.ABSTAIN
    )

    unseen = good.model_copy(update={"category": "groceries"})
    assert (
        automation_decision(unseen, policy=policy, automation=automation).method
        is CategorisationMethod.ABSTAIN
    )


def test_split_by_identity_keeps_identity_in_one_bucket() -> None:
    examples = tuple(
        CalibrationExample(
            feature_hash="a" * 64,
            normalized_merchant=f"merchant {index}",
            approved_category="dining",
            source_identity=identity,
        )
        for index, identity in enumerate(f"{value:064x}" for value in range(1, 4))
    )
    split = split_by_identity(examples)
    assert len(split.train) + len(split.calibration) + len(split.holdout) == 3
    again = split_by_identity(examples)
    assert again == split
