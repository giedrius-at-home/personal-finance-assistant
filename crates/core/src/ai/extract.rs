//! Validation of model JSON: the pydantic models the Python service checked
//! model output against (`ModelCategorisation`, `ExtraSuggestions`,
//! `CountrySuggestions`, `CheckResponse`), with their lax coercions.

use std::str::FromStr;

use finance_types::report::review::{
    CountrySuggestion, CountrySuggestions, DetailSuggestion, ExtraSuggestions, TagSuggestion,
};
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use crate::text::is_country;

/// `REVIEW_POLICY.enrichment` limits.
const DETAIL_LENGTH_LIMIT: usize = 200;
const TAG_LENGTH_LIMIT: usize = 80;
const SUGGESTED_TAGS_LIMIT: usize = 6;
/// The enabled detail fields, in policy order.
const EXTRA_FIELDS: [&str; 3] = ["merchant_name", "merchant_location", "payment_method"];

/// A pydantic `Decimal` in lax mode: a JSON number or a numeric string,
/// never a boolean.
pub fn decimal(value: &Value) -> Option<Decimal> {
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_owned(),
        _ => return None,
    };
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .ok()
}

/// A probability: a decimal in `[0, 1]`.
pub fn unit(value: &Value) -> Option<Decimal> {
    decimal(value).filter(|d| (Decimal::ZERO..=Decimal::ONE).contains(d))
}

/// A string of `min..=max` characters (pydantic never coerces to `str`).
pub fn text(value: &Value, min: usize, max: usize) -> Option<String> {
    let s = value.as_str()?;
    (min..=max)
        .contains(&s.chars().count())
        .then(|| s.to_owned())
}

/// Only the `allowed` keys (`extra="forbid"`).
fn only(map: &Map<String, Value>, allowed: &[&str]) -> bool {
    map.keys().all(|k| allowed.contains(&k.as_str()))
}

/// `" ".join(text.casefold().split())`.
pub fn fold(text: &str) -> String {
    crate::text::casefold(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Evidence: 1–3 excerpts of 1–200 characters.
fn evidence(value: Option<&Value>) -> Option<Vec<String>> {
    let items = value?.as_array()?;
    if !(1..=3).contains(&items.len()) {
        return None;
    }
    items.iter().map(|e| text(e, 1, 200)).collect()
}

/// Every excerpt, whitespace-folded, occurs in the folded description.
fn supported(excerpts: &[String], description: &str) -> bool {
    excerpts.iter().all(|e| {
        let e = fold(e);
        !e.is_empty() && description.contains(&e)
    })
}

/// `ModelCategorisation`: the classifier's structured answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCategorisation {
    pub category: String,
    pub confidence: Decimal,
    pub runner_up: Option<String>,
    pub runner_up_confidence: Decimal,
    pub reason: String,
}

pub fn model_categorisation(map: &Map<String, Value>) -> Option<ModelCategorisation> {
    if !only(
        map,
        &[
            "category",
            "confidence",
            "runner_up",
            "runner_up_confidence",
            "reason",
        ],
    ) {
        return None;
    }
    let category = text(map.get("category")?, 1, 128)?;
    let confidence = unit(map.get("confidence")?)?;
    let runner_up = match map.get("runner_up") {
        None | Some(Value::Null) => None,
        Some(v) => Some(text(v, 1, 128)?),
    };
    let runner_up_confidence = match map.get("runner_up_confidence") {
        None => Decimal::ZERO,
        Some(v) => unit(v)?,
    };
    let reason = text(map.get("reason")?, 1, 256)?;
    if (runner_up.is_none() && !runner_up_confidence.is_zero())
        || runner_up.as_deref() == Some(category.as_str())
        || runner_up_confidence > confidence
    {
        return None;
    }
    Some(ModelCategorisation {
        category,
        confidence,
        runner_up,
        runner_up_confidence,
        reason,
    })
}

fn detail(value: Option<&Value>) -> Option<DetailSuggestion> {
    let map = value?.as_object()?;
    if !only(map, &["value", "evidence"]) {
        return None;
    }
    Some(DetailSuggestion {
        value: text(map.get("value")?, 1, 200)?,
        evidence: evidence(map.get("evidence"))?,
    })
}

fn tag(value: &Value) -> Option<TagSuggestion> {
    let map = value.as_object()?;
    if !only(map, &["label", "parent", "evidence"]) {
        return None;
    }
    Some(TagSuggestion {
        label: text(map.get("label")?, 1, 80)?,
        parent: match map.get("parent") {
            None => String::new(),
            Some(v) => text(v, 0, 80)?,
        },
        evidence: evidence(map.get("evidence"))?,
    })
}

/// `parse_extras`: details and tags whose evidence is in `description`.
pub fn extras(value: Option<&Value>, description: &str) -> ExtraSuggestions {
    let Some(map) = value.and_then(Value::as_object) else {
        return ExtraSuggestions::default();
    };
    let folded = fold(description);
    let mut found = ExtraSuggestions::default();
    for key in EXTRA_FIELDS {
        let Some(suggestion) = detail(map.get(key)) else {
            continue;
        };
        if suggestion.value.chars().count() > DETAIL_LENGTH_LIMIT
            || !supported(&suggestion.evidence, &folded)
        {
            continue;
        }
        let slot = match key {
            "merchant_name" => &mut found.merchant_name,
            "merchant_location" => &mut found.merchant_location,
            _ => &mut found.payment_method,
        };
        *slot = Some(suggestion);
    }
    if let Some(tags) = map.get("tags").and_then(Value::as_array) {
        for raw in tags.iter().take(SUGGESTED_TAGS_LIMIT) {
            let Some(tag) = tag(raw) else {
                continue;
            };
            if tag.label.chars().count().max(tag.parent.chars().count()) <= TAG_LENGTH_LIMIT
                && supported(&tag.evidence, &folded)
            {
                found.tags.push(tag);
            }
        }
    }
    found
}

fn country(value: &Value) -> Option<CountrySuggestion> {
    let map = value.as_object()?;
    if !only(map, &["code", "reason", "evidence"]) {
        return None;
    }
    let code = map.get("code")?.as_str()?;
    if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) || !is_country(code) {
        return None;
    }
    Some(CountrySuggestion {
        code: code.to_owned(),
        reason: text(map.get("reason")?, 1, 400)?,
        evidence: evidence(map.get("evidence"))?,
    })
}

/// `parse_country_suggestions`: each side on its own, never one the bank
/// already supplied.
pub fn countries(
    value: Option<&Value>,
    description: &str,
    known_from: Option<&str>,
    known_to: Option<&str>,
) -> CountrySuggestions {
    let Some(map) = value.and_then(Value::as_object) else {
        return CountrySuggestions::default();
    };
    let folded = fold(description);
    let side = |name: &str, known: Option<&str>| {
        if known.is_some() {
            return None;
        }
        let suggestion = country(map.get(name).filter(|v| !v.is_null())?)?;
        supported(&suggestion.evidence, &folded).then_some(suggestion)
    };
    CountrySuggestions {
        from_country: side("from_country", known_from),
        to_country: side("to_country", known_to),
    }
}

/// `CheckResponse`: the second reviewer's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResponse {
    pub supported: bool,
    pub reason: String,
    pub evidence: Vec<String>,
}

pub fn check_response(map: &Map<String, Value>) -> Option<CheckResponse> {
    if !only(map, &["verdict", "reason", "evidence"]) {
        return None;
    }
    let supported = match map.get("verdict")?.as_str()? {
        "supported" => true,
        "review" => false,
        _ => return None,
    };
    let items = map.get("evidence")?.as_array()?;
    if items.len() > 3 {
        return None;
    }
    let evidence = items
        .iter()
        .map(|e| e.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(CheckResponse {
        supported,
        reason: text(map.get("reason")?, 1, 500)?,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_model_categorisation_like_pydantic() {
        let ok = json!({"category": "Groceries", "confidence": 0.9, "runner_up": "Dining",
            "runner_up_confidence": "0.05", "reason": "supermarket"});
        let parsed = model_categorisation(ok.as_object().unwrap()).unwrap();
        assert_eq!(parsed.confidence, Decimal::from_str("0.9").unwrap());
        assert_eq!(
            parsed.runner_up_confidence,
            Decimal::from_str("0.05").unwrap()
        );
        for bad in [
            json!({"category": "A", "confidence": true, "reason": "r"}),
            json!({"category": "A", "confidence": 1.5, "reason": "r"}),
            json!({"category": "A", "confidence": 0.5, "reason": "r", "extra": 1}),
            json!({"category": "A", "confidence": 0.5, "reason": "r", "runner_up": "A", "runner_up_confidence": 0.1}),
            json!({"category": "A", "confidence": 0.5, "reason": "r", "runner_up_confidence": 0.1}),
            json!({"category": "A", "confidence": 0.5, "reason": "r", "runner_up": "B", "runner_up_confidence": 0.6}),
            json!({"category": 5, "confidence": 0.5, "reason": "r"}),
        ] {
            assert!(
                model_categorisation(bad.as_object().unwrap()).is_none(),
                "{bad}"
            );
        }
        let minimal = json!({"category": "A", "confidence": 1, "runner_up": null, "runner_up_confidence": 0.0, "reason": "r"});
        assert!(model_categorisation(minimal.as_object().unwrap()).is_some());
    }

    #[test]
    fn keeps_only_evidenced_extras_and_countries() {
        let description = "bea, betaalpas cafe de klos amsterdam";
        let raw = json!({
            "merchant_name": {"value": "Cafe de Klos", "evidence": ["Cafe  de KLOS"]},
            "merchant_location": {"value": "Amsterdam", "evidence": ["Rotterdam"]},
            "payment_method": null,
            "tags": [{"label": "Beer", "parent": "Drinks", "evidence": ["cafe"]},
                     {"label": "Wine", "evidence": ["wine"]}]
        });
        let parsed = extras(Some(&raw), description);
        assert_eq!(parsed.merchant_name.unwrap().value, "Cafe de Klos");
        assert!(parsed.merchant_location.is_none());
        assert_eq!(parsed.tags.len(), 1);
        assert_eq!(parsed.tags[0].parent, "Drinks");

        let raw = json!({
            "from_country": {"code": "NL", "reason": "memo", "evidence": ["amsterdam"]},
            "to_country": {"code": "XX", "reason": "?", "evidence": ["amsterdam"]}
        });
        let parsed = countries(Some(&raw), description, None, None);
        assert_eq!(parsed.from_country.unwrap().code, "NL");
        assert!(parsed.to_country.is_none());
        assert!(
            countries(Some(&raw), description, Some("NL"), None)
                .from_country
                .is_none()
        );
    }
}
