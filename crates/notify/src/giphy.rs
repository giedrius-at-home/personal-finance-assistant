//! The reaction GIF after the brief on Telegram: one search on GIPHY
//! (rating R, the owner is an adult) with the words the model chose, and
//! one of the first hits at random so the same search does not always give
//! the same GIF. Blocking, like the rest of this crate. The key is part of
//! the URL, so no error text here ever carries one.

use std::io::Read;
use std::time::Duration;

use finance_secrets::Secret;
use serde_json::Value;

/// Hits to choose from.
const HITS: usize = 8;
/// A search reply is well under this.
const MAX_REPLY: usize = 1024 * 1024;

/// Whether `key` looks like a GIPHY API key: 20 to 64 letters and digits.
pub fn valid_key(key: &str) -> bool {
    (20..=64).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The GIFs in a search reply, as URLs Telegram can fetch: the MP4 when
/// there is one (smaller, plays inline), else the GIF. Only GIPHY's own
/// media hosts over HTTPS.
pub fn urls(reply: &Value) -> Vec<String> {
    let Some(hits) = reply.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    hits.iter()
        .filter_map(|h| {
            let original = h.get("images")?.get("original")?;
            ["mp4", "url"]
                .iter()
                .filter_map(|k| original.get(*k).and_then(Value::as_str))
                .find(|u| on_giphy(u))
                .map(str::to_owned)
        })
        .take(HITS)
        .collect()
}

fn on_giphy(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    host == "giphy.com" || host.ends_with(".giphy.com")
}

/// `api.giphy.com`.
#[derive(Debug)]
pub struct Giphy(reqwest::blocking::Client);

impl Giphy {
    pub fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map(Self)
            .map_err(|e| format!("GIPHY client: {}", e.without_url()))
    }

    /// The URLs of the first GIFs for `query`.
    pub fn search(&self, key: &Secret, query: &str) -> Result<Vec<String>, String> {
        let url = reqwest::Url::parse_with_params(
            "https://api.giphy.com/v1/gifs/search",
            &[
                ("api_key", key.expose()),
                ("q", query),
                ("limit", &HITS.to_string()),
                ("rating", "r"),
                ("lang", "en"),
            ],
        )
        .map_err(|_| "GIPHY search could not be built.".to_owned())?;
        let response = self
            .0
            .get(url)
            .send()
            .map_err(|e| format!("GIPHY could not be reached: {}", e.without_url()))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err("GIPHY did not accept the key.".into());
        }
        if !status.is_success() {
            return Err(format!("GIPHY answered {status}."));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_REPLY as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "GIPHY's reply could not be read.".to_owned())?;
        if bytes.len() > MAX_REPLY {
            return Err("GIPHY's reply was too long.".into());
        }
        let reply: Value =
            serde_json::from_slice(&bytes).map_err(|_| "GIPHY's reply was not JSON.".to_owned())?;
        Ok(urls(&reply))
    }

    /// One of the first GIFs for `query`, picked by `seed`; `None` when
    /// nothing was found.
    pub fn pick(&self, key: &Secret, query: &str, seed: u64) -> Result<Option<String>, String> {
        let found = self.search(key, query)?;
        if found.is_empty() {
            return Ok(None);
        }
        let i = usize::try_from(seed % found.len() as u64).unwrap_or_default();
        Ok(found.into_iter().nth(i))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_and_urls() {
        assert!(valid_key("dc6zaTOxFJmzCdc6zaTOxFJmzC"));
        assert!(!valid_key("short"));
        assert!(!valid_key("dc6zaTOxFJmzC/dc6zaTOxFJmzC"));
        let reply = json!({ "data": [
            { "images": { "original": { "mp4": "https://media2.giphy.com/media/a/giphy.mp4", "url": "https://media2.giphy.com/media/a/giphy.gif" } } },
            { "images": { "original": { "url": "https://media.giphy.com/media/b/giphy.gif" } } },
            { "images": { "original": { "url": "https://evil.example/giphy.com/x.gif" } } },
            { "images": { "original": { "url": "http://media.giphy.com/media/c/giphy.gif" } } },
            { "nothing": true },
        ] });
        assert_eq!(
            urls(&reply),
            [
                "https://media2.giphy.com/media/a/giphy.mp4",
                "https://media.giphy.com/media/b/giphy.gif"
            ]
        );
        assert!(urls(&json!({})).is_empty());
    }
}
