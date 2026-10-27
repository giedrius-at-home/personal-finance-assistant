//! TypeSafe System One Choice questions (Python `integrations/typesafe.py`):
//! the request body, the pinned response contract and the category criteria.

use rust_decimal::Decimal;
use serde_json::Value;

use super::BackendError;
use super::extract::decimal;
use super::pyjson::J;

pub const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";
/// The option that means "none of these".
pub const ABSTAIN: &str = "__abstain__";
pub const PROMPT_VERSION: &str = "ts-choice-v6-payment-methods";
pub const MAX_CHOICE_OPTIONS: usize = 255;
pub const MAX_CONTEXT_EXAMPLES: usize = 12;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Closed-choice definitions that separate money movement from purchase
/// purpose (`CATEGORY_CRITERIA`), keyed by case-folded category.
pub const CATEGORY_CRITERIA: [(&str, &str); 17] = [
    (
        "google play",
        "An explicit Google Play app-store merchant charge. Google Pay is a payment method and is not Google Play. A shop paid with Google Pay belongs to the shop's category.",
    ),
    (
        "google one",
        "An explicit Google One storage-service charge. Google Pay wallet payments and Google Play alone do not identify Google One.",
    ),
    (
        "transport",
        "Public transport, fares and returned transit-card balances, including OVpay and OV-chipkaart. Not internet bills or travel-retail shopping.",
    ),
    (
        "vue movie pass",
        "Explicit Vue moviepas/movie-pass recurring invoice. Not an ordinary Vue cinema merchant charge without pass evidence.",
    ),
    (
        "other internet bill",
        "Payment for internet access from a telecom provider. A URL, online purchase, app store or marketplace alone is not evidence.",
    ),
    (
        "internet",
        "An explicit internet-access bill or a shared internet-bill repayment.",
    ),
    (
        "other phone bill",
        "Mobile/telephone service from a provider such as Lebara or Pildyk. An incoming bank-card top-up or returned transit-card balance is not phone service.",
    ),
    (
        "revpoints",
        "Purchase of Revolut loyalty points, including RevPoints Spare Change. Not an internal transfer or a savings deposit.",
    ),
    (
        "revolut",
        "Revolut account funding or bank movement. Not RevPoints purchases; use a more specific purpose when stated.",
    ),
    (
        "money transfer",
        "Explicit account funding or money movement without an identified purchase purpose. Not ordinary retail spending or a named shared purchase.",
    ),
    (
        "account top-up",
        "Incoming funding of a bank/payment account by a payment card. Not a phone recharge, salary or a purchase.",
    ),
    (
        "cash",
        "Actual cash withdrawal or cash account movement. Not a fallback for a named shop purchase or a repayment.",
    ),
    (
        "credit card",
        "Payment or settlement of a credit-card account; not the purpose of the underlying purchases.",
    ),
    (
        "klarna",
        "Klarna payment with no more specific purchase purpose stated. If the memo names airline tickets, use Flights/Travel instead.",
    ),
    (
        "investments",
        "Investment purchases or movement into/out of an investment account. Not consumption spending or drinks.",
    ),
    (
        "subscriptions",
        "An explicitly recurring service. A merchant name alone does not establish recurrence; prefer a specific supported merchant/purpose.",
    ),
    (
        "income",
        "Income earned or received, excluding account funding, refunds, returned balances, reimbursements and savings movements.",
    ),
];

/// The criterion of one category option.
pub fn criterion(category: &str) -> String {
    let key = crate::text::casefold(category);
    CATEGORY_CRITERIA
        .iter()
        .find(|(k, _)| *k == key)
        .map_or_else(
            || {
                format!(
                    "The named merchant or explicit purchase purpose is {category}. \
                     Do not infer the item purchased solely from a general retailer."
                )
            },
            |(_, v)| (*v).to_owned(),
        )
}

/// One Choice question. Python always asked it as `transaction_category`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceRequest {
    pub state: J,
    pub instructions: String,
    /// Options in order, with their definitions.
    pub criteria: Vec<(String, String)>,
}

impl ChoiceRequest {
    /// Python `choose`'s argument checks.
    pub fn validate(&self) -> Result<(), BackendError> {
        if !(2..=MAX_CHOICE_OPTIONS).contains(&self.criteria.len()) {
            return Err(BackendError::Invalid(
                "TypeSafe Choice must contain between 2 and 255 options",
            ));
        }
        if self
            .criteria
            .iter()
            .any(|(o, _)| o.is_empty() || o.chars().count() > 128)
        {
            return Err(BackendError::Invalid("TypeSafe Choice option is invalid"));
        }
        if self.instructions.is_empty() || self.instructions.chars().count() > 2000 {
            return Err(BackendError::Invalid("TypeSafe instructions are invalid"));
        }
        Ok(())
    }

    /// The request body for `model`.
    pub fn body(&self, model: &str) -> J {
        J::obj([
            ("state", self.state.clone()),
            ("model", J::str(model)),
            (
                "questions",
                J::obj([(
                    "transaction_category",
                    J::obj([
                        ("type", J::str("choice")),
                        ("instructions", J::str(&self.instructions)),
                        (
                            "criteria",
                            J::Obj(
                                self.criteria
                                    .iter()
                                    .map(|(k, v)| (k.clone(), J::str(v)))
                                    .collect(),
                            ),
                        ),
                    ]),
                )]),
            ),
        ])
    }
}

/// A validated Choice answer (`TypeSafeChoiceResult`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceResult {
    pub model: String,
    pub choice: String,
    /// Every option's probability, in response order.
    pub probabilities: Vec<(String, Decimal)>,
    pub confidence: Decimal,
}

impl ChoiceResult {
    pub fn probability(&self, option: &str) -> Decimal {
        self.probabilities
            .iter()
            .find(|(o, _)| o == option)
            .map_or(Decimal::ZERO, |(_, p)| *p)
    }

    /// Options other than abstention, most likely first (ties by case-folded name).
    pub fn ranked(&self) -> Vec<(String, Decimal)> {
        let mut ranked: Vec<(String, Decimal)> = self
            .probabilities
            .iter()
            .filter(|(o, _)| o != ABSTAIN)
            .cloned()
            .collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| crate::text::casefold(&a.0).cmp(&crate::text::casefold(&b.0)))
        });
        ranked
    }
}

fn number(value: &Value) -> Option<Decimal> {
    matches!(value, Value::Number(_))
        .then(|| decimal(value))
        .flatten()
}

/// `_parse_choice`: the pinned response schema and a sane distribution.
pub fn parse_choice(document: &Value, options: &[String]) -> Result<ChoiceResult, BackendError> {
    let schema = BackendError::Protocol;
    let map = document.as_object().ok_or(schema.clone())?;
    let model = map
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty() && m.chars().count() <= 128)
        .ok_or(schema.clone())?;
    let answer = map
        .get("answers")
        .and_then(Value::as_object)
        .and_then(|a| a.get("transaction_category"))
        .and_then(Value::as_object)
        .filter(|a| a.get("type").and_then(Value::as_str) == Some("choice"))
        .ok_or(schema.clone())?;
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .filter(|c| options.iter().any(|o| o == c))
        .ok_or(schema.clone())?;
    let raw = answer
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or(schema.clone())?;
    if raw.len() != options.len() || !options.iter().all(|o| raw.contains_key(o)) {
        return Err(schema);
    }
    let confidence = answer
        .get("confidence")
        .and_then(number)
        .ok_or(schema.clone())?;
    let mut probabilities = Vec::with_capacity(raw.len());
    for option in options {
        let p = raw.get(option).and_then(number).ok_or(schema.clone())?;
        if !(Decimal::ZERO..=Decimal::ONE).contains(&p) {
            return Err(schema);
        }
        probabilities.push((option.clone(), p));
    }
    let total: Decimal = probabilities.iter().map(|(_, p)| *p).sum();
    let max = probabilities
        .iter()
        .map(|(_, p)| *p)
        .max()
        .unwrap_or_default();
    let chosen = probabilities
        .iter()
        .find(|(o, _)| o == choice)
        .map_or(Decimal::ZERO, |(_, p)| *p);
    if !(Decimal::ZERO..=Decimal::ONE).contains(&confidence)
        || (total - Decimal::ONE).abs() > Decimal::new(2, 2)
        || chosen != max
    {
        return Err(schema);
    }
    Ok(ChoiceResult {
        model: model.to_owned(),
        choice: choice.to_owned(),
        probabilities,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::str::FromStr;

    fn options() -> Vec<String> {
        vec!["Groceries".into(), "Dining".into(), ABSTAIN.into()]
    }

    #[test]
    fn parses_a_captured_choice_answer() {
        let captured = json!({"model": "jev-latest", "answers": {"transaction_category": {
            "type": "choice", "choice": "Groceries", "confidence": 0.81,
            "probabilities": {"Groceries": 0.81, "Dining": 0.09999999999999999, "__abstain__": 0.09}}}});
        let r = parse_choice(&captured, &options()).unwrap();
        assert_eq!(r.choice, "Groceries");
        assert_eq!(
            r.probability("Dining"),
            Decimal::from_str("0.09999999999999999").unwrap()
        );
        assert_eq!(r.ranked()[0].0, "Groceries");
        assert_eq!(r.ranked().len(), 2);
    }

    #[test]
    fn rejects_broken_distributions() {
        for bad in [
            json!({"model": "m", "answers": {"transaction_category": {"type": "choice", "choice": "Dining", "confidence": 0.5,
                "probabilities": {"Groceries": 0.6, "Dining": 0.3, "__abstain__": 0.1}}}}),
            json!({"model": "m", "answers": {"transaction_category": {"type": "choice", "choice": "Groceries", "confidence": 0.5,
                "probabilities": {"Groceries": 0.6, "Dining": 0.1, "__abstain__": 0.1}}}}),
            json!({"model": "m", "answers": {"transaction_category": {"type": "choice", "choice": "Groceries", "confidence": true,
                "probabilities": {"Groceries": 0.6, "Dining": 0.3, "__abstain__": 0.1}}}}),
            json!({"model": "m", "answers": {"transaction_category": {"type": "choice", "choice": "Groceries", "confidence": 0.6,
                "probabilities": {"Groceries": 0.6, "Dining": 0.4}}}}),
            json!({"answers": {}}),
        ] {
            assert!(parse_choice(&bad, &options()).is_err(), "{bad}");
        }
    }

    #[test]
    fn body_keeps_option_order() {
        let request = ChoiceRequest {
            state: J::obj([("x", J::Null)]),
            instructions: "Pick".into(),
            criteria: vec![("b".into(), "B".into()), ("a".into(), "A".into())],
        };
        assert!(request.validate().is_ok());
        let body = serde_json::to_string(&request.body("jev-latest")).unwrap();
        assert!(body.contains(r#""criteria":{"b":"B","a":"A"}"#));
        assert_eq!(
            criterion("Transport").split('.').next(),
            Some(
                "Public transport, fares and returned transit-card balances, including OVpay and OV-chipkaart"
            )
        );
    }
}
