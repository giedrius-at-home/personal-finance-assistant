//! The owner's AI settings (`ai_settings`): which classifier runs, the
//! Mistral model, the TypeSafe model, research and automatic
//! approval. Python read these from `PFA_*` environment variables
//! (`CategorySuggestionSettings.from_environment`).

use finance_types::ai::AiSettings;
use rusqlite::{Connection, OptionalExtension};

use crate::writes::{Writer, invalid};
use crate::{Error, Result};

/// The stored settings, or the defaults (everything off).
pub fn load(conn: &Connection) -> Result<AiSettings> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'ai_settings')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(AiSettings::default());
    }
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM ai_settings WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    payload.map_or_else(
        || Ok(AiSettings::default()),
        |p| serde_json::from_str(&p).map_err(|e| Error::Invalid(format!("ai settings: {e}"))),
    )
}

fn model_name(value: &str, optional: bool) -> Option<String> {
    let value = value.trim();
    let valid = value.chars().count() <= 128
        && (optional || !value.is_empty())
        && !value.chars().any(char::is_control);
    valid.then(|| value.to_owned())
}

/// Check and normalise settings from the form.
pub fn validate(mut settings: AiSettings) -> Result<AiSettings> {
    settings.mistral_model = model_name(&settings.mistral_model, false)
        .filter(|m| !m.contains(char::is_whitespace))
        .ok_or_else(|| invalid("Enter a Mistral model such as mistral-small-latest."))?;
    settings.typesafe_model = model_name(&settings.typesafe_model, false)
        .ok_or_else(|| invalid("Enter a TypeSafe model."))?;
    if !(1..=100).contains(&settings.batch_size) {
        return Err(invalid("The batch size must be between 1 and 100."));
    }
    Ok(settings)
}

pub fn save(w: &Writer<'_>, settings: AiSettings) -> Result<AiSettings> {
    let settings = validate(settings)?;
    w.put_json(
        "INSERT INTO ai_settings (singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        &settings,
        &[],
    )?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::ai::Classifier;

    #[test]
    fn settings_saved_for_ollama_read_as_mistral() {
        let old: AiSettings = serde_json::from_str(
            r#"{"classifier":"ollama","ollama_url":"http://127.0.0.1:11434","ollama_model":"qwen3:8b","embedding_model":"nomic"}"#,
        )
        .unwrap();
        assert_eq!(old.classifier, Classifier::Mistral);
        assert_eq!(old.mistral_model, "mistral-small-latest");
        assert!(!old.embeddings);
        assert!(
            validate(AiSettings {
                mistral_model: "two words".into(),
                ..old
            })
            .is_err()
        );
    }

    #[test]
    fn defaults_are_off_and_validate() {
        let settings = AiSettings::default();
        assert_eq!(settings.classifier, Classifier::Off);
        assert_eq!(settings.mistral_model, "mistral-small-latest");
        assert!(validate(settings).is_ok());
        assert!(
            validate(AiSettings {
                batch_size: 0,
                ..AiSettings::default()
            })
            .is_err()
        );
    }
}
