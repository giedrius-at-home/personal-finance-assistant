//! The owner's AI settings (`ai_settings`): which classifier runs, the
//! local Ollama URL and models, the TypeSafe model, research and automatic
//! approval. Python read these from `PFA_*` environment variables
//! (`CategorySuggestionSettings.from_environment`).

use finance_types::ai::AiSettings;
use rusqlite::{Connection, OptionalExtension};

use crate::writes::{Writer, invalid};
use crate::{Error, Result};

/// The hosts a local model may live on (`normalize_ollama_base_url`).
const LOCAL_HOSTS: [&str; 4] = ["127.0.0.1", "[::1]", "localhost", "host.docker.internal"];

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

/// A loopback `http(s)://host[:port]` URL without credentials, path, query
/// or fragment, with a trailing slash.
pub fn ollama_base_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let ok = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/"
        && LOCAL_HOSTS.contains(&host.as_str());
    ok.then(|| value.trim().trim_end_matches('/').to_owned() + "/")
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
    let url = ollama_base_url(&settings.ollama_url)
        .ok_or_else(|| invalid("Enter a local Ollama address such as http://127.0.0.1:11434."))?;
    settings.ollama_url = url.trim_end_matches('/').to_owned();
    settings.ollama_model = model_name(&settings.ollama_model, false)
        .ok_or_else(|| invalid("Enter an Ollama model."))?;
    settings.embedding_model = model_name(&settings.embedding_model, true)
        .ok_or_else(|| invalid("The embedding model name is too long."))?;
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
    fn only_local_ollama_addresses_are_accepted() {
        assert_eq!(
            ollama_base_url("http://127.0.0.1:11434").as_deref(),
            Some("http://127.0.0.1:11434/")
        );
        assert!(ollama_base_url("http://localhost:11434/").is_some());
        assert!(ollama_base_url("http://[::1]:11434").is_some());
        for bad in [
            "http://example.com:11434",
            "http://user:pw@127.0.0.1:11434",
            "http://127.0.0.1:11434/api",
            "http://127.0.0.1:11434/?x=1",
            "ftp://127.0.0.1",
            "127.0.0.1:11434",
        ] {
            assert!(ollama_base_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn defaults_are_off_and_validate() {
        let settings = AiSettings::default();
        assert_eq!(settings.classifier, Classifier::Off);
        assert_eq!(settings.ollama_model, "qwen3:8b");
        let checked = validate(settings).unwrap();
        assert_eq!(checked.ollama_url, "http://127.0.0.1:11434");
        assert!(
            validate(AiSettings {
                batch_size: 0,
                ..AiSettings::default()
            })
            .is_err()
        );
    }
}
