//! Mistral AI (Paris; the API runs in the EU): the one model provider the
//! app uses. Chat writes category suggestions, Gisel's email lines and her
//! daily script; embeddings find similar past transactions; Voxtral turns
//! the script into speech.
//!
//! Every call blocks, so callers run it on their own thread. The API key
//! stays in [`Mistral`] (wiped on drop) and errors never carry prompts,
//! answers or the key.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use finance_core::ai::retrieval::{check_embedding_inputs, parse_embeddings};
use finance_core::ai::{BackendError, Embed, Generate};
use finance_secrets::Secret;
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use serde_json::{Value, json};

pub const API: &str = "https://api.mistral.ai";
/// Small, cheap and good at short JSON answers.
pub const DEFAULT_CHAT_MODEL: &str = "mistral-small-latest";
pub const EMBED_MODEL: &str = "mistral-embed";
/// Voxtral Mini TTS 3, pinned: the `-latest` alias still served the older
/// model, which refuses the Voxtral 3 voices (HTTP 400, code 3051).
pub const SPEECH_MODEL: &str = "voxtral-mini-tts-3-0";

const RETRYABLE: [u16; 4] = [429, 502, 503, 504];
const MAX_CHAT_BYTES: usize = 256 * 1024;
const MAX_EMBED_BYTES: usize = 8 * 1024 * 1024;
/// A minute of MP3 is well under a megabyte; base64 adds a third.
const MAX_SPEECH_BYTES: usize = 16 * 1024 * 1024;
/// Text sent to speech, at most (a brief is about 500).
pub const MAX_SPEECH_CHARS: usize = 2_000;

/// A key Mistral could accept: 1–512 ASCII characters, no whitespace.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 512 && key.is_ascii() && !key.chars().any(char::is_whitespace)
}

/// A model id: 1–128 characters, no spaces or control characters.
pub fn valid_model(model: &str) -> bool {
    let model = model.trim();
    !model.is_empty()
        && model.chars().count() <= 128
        && !model.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// One of Mistral's voices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    pub gender: Option<String>,
    /// Mistral's short name ("voice_483"); also accepted as the voice id.
    pub slug: String,
    /// Cloned by the owner in Mistral Studio, not a preset.
    pub custom: bool,
}

pub struct Mistral {
    key: Secret,
    base: String,
    http: Client,
    attempts: u32,
}

impl std::fmt::Debug for Mistral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mistral(<redacted>)")
    }
}

fn read_limited(response: Response, max: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    response.take(max as u64 + 1).read_to_end(&mut body).ok()?;
    (body.len() <= max).then_some(body)
}

impl Mistral {
    /// The public API over HTTPS. `timeout` bounds each request.
    pub fn new(key: Secret, timeout: Duration) -> Result<Self, BackendError> {
        Self::build(key, API, timeout, true)
    }

    /// A stand-in server on this machine (tests).
    pub fn local(key: Secret, base: &str, timeout: Duration) -> Result<Self, BackendError> {
        let url = reqwest::Url::parse(base).map_err(|_| BackendError::Invalid("bad base URL"))?;
        if url.host_str() != Some("127.0.0.1") {
            return Err(BackendError::Invalid("a test server must be on 127.0.0.1"));
        }
        Self::build(key, base, timeout, false)
    }

    fn build(
        key: Secret,
        base: &str,
        timeout: Duration,
        https: bool,
    ) -> Result<Self, BackendError> {
        if !valid_key(key.expose()) {
            return Err(BackendError::Invalid("The Mistral API key is not valid."));
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout)
            .redirect(Policy::none())
            .no_proxy()
            .https_only(https)
            .build()
            .map_err(|_| BackendError::Unavailable)?;
        Ok(Self {
            key,
            base: base.trim_end_matches('/').to_owned(),
            http,
            attempts: 3,
        })
    }

    /// How many times a request is tried (transport errors, 429, 5xx).
    #[must_use]
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.clamp(1, 5);
        self
    }

    fn send(&self, path: &str, body: Option<&Value>) -> Result<Response, BackendError> {
        let url = format!("{}{path}", self.base);
        let body = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| BackendError::Protocol)?;
        let mut response = None;
        for attempt in 1..=self.attempts {
            let request = match &body {
                Some(b) => self
                    .http
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(b.clone()),
                None => self.http.get(&url),
            };
            response = request
                .bearer_auth(self.key.expose())
                .header("Accept", "application/json")
                .send()
                .ok();
            if response
                .as_ref()
                .is_some_and(|r| !RETRYABLE.contains(&r.status().as_u16()))
            {
                break;
            }
            if attempt < self.attempts {
                std::thread::sleep(Duration::from_millis(250 * (1 << (attempt - 1))));
            }
        }
        let response = response.ok_or(BackendError::Unavailable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(BackendError::Status(status.as_u16()));
        }
        Ok(response)
    }

    fn json(&self, path: &str, body: Option<&Value>, max: usize) -> Result<Value, BackendError> {
        let response = self.send(path, body)?;
        let body = read_limited(response, max).ok_or(BackendError::Protocol)?;
        match serde_json::from_slice(&body) {
            Ok(value @ Value::Object(_)) => Ok(value),
            _ => Err(BackendError::Protocol),
        }
    }

    /// One chat turn that must answer a JSON object; the answer's text.
    pub fn chat_json(
        &self,
        model: &str,
        system: Option<&str>,
        prompt: &str,
        temperature: f64,
    ) -> Result<String, BackendError> {
        if !valid_model(model) {
            return Err(BackendError::Invalid("Enter a Mistral model."));
        }
        let mut messages = Vec::new();
        if let Some(system) = system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.push(json!({"role": "user", "content": prompt}));
        let document = self.json(
            "/v1/chat/completions",
            Some(&json!({
                "model": model.trim(),
                "messages": messages,
                "temperature": temperature,
                "response_format": {"type": "json_object"},
            })),
            MAX_CHAT_BYTES,
        )?;
        document
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(BackendError::Protocol)
    }

    /// `mistral-embed` vectors, in the order of `inputs`.
    pub fn embeddings(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>, BackendError> {
        check_embedding_inputs(inputs)?;
        let document = self.json(
            "/v1/embeddings",
            Some(&json!({"model": EMBED_MODEL, "input": inputs})),
            MAX_EMBED_BYTES,
        )?;
        let mut rows: Vec<(u64, Value)> = document
            .get("data")
            .and_then(Value::as_array)
            .ok_or(BackendError::Protocol)?
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let index = row.get("index").and_then(Value::as_u64).unwrap_or(i as u64);
                (index, row.get("embedding").cloned().unwrap_or(Value::Null))
            })
            .collect();
        rows.sort_by_key(|(i, _)| *i);
        let embeddings: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
        parse_embeddings(&json!({ "embeddings": embeddings }), inputs.len())
    }

    /// `text` spoken by `voice` (a preset or custom voice id or slug;
    /// `None` lets Mistral choose) at `expressiveness` (1.0 calm to 2.0
    /// very lively), as MP3 bytes. Should Mistral refuse the
    /// expressiveness, the speech is made once more without it.
    pub fn speech(
        &self,
        text: &str,
        voice: Option<&str>,
        expressiveness: Option<f64>,
    ) -> Result<Vec<u8>, BackendError> {
        match self.speech_once(text, voice, expressiveness) {
            Err(BackendError::Status(400 | 422)) if expressiveness.is_some() => {
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    eprintln!("mistral: speech refused expressiveness; speaking without it");
                }
                self.speech_once(text, voice, None)
            }
            other => other,
        }
    }

    fn speech_once(
        &self,
        text: &str,
        voice: Option<&str>,
        expressiveness: Option<f64>,
    ) -> Result<Vec<u8>, BackendError> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_SPEECH_CHARS {
            return Err(BackendError::Invalid(
                "The spoken text is empty or too long.",
            ));
        }
        let mut body = json!({
            "model": SPEECH_MODEL,
            "input": text,
            "response_format": "mp3",
        });
        if let Some(voice) = voice.map(str::trim).filter(|v| !v.is_empty()) {
            if voice.len() > 128 || voice.chars().any(char::is_control) {
                return Err(BackendError::Invalid("The voice id is not valid."));
            }
            body["voice_id"] = Value::String(voice.to_owned());
        }
        if let Some(e) = expressiveness {
            body["expressiveness"] = json!((e.clamp(1.0, 2.0) * 10.0).round() / 10.0);
        }
        let document = self.json("/v1/audio/speech", Some(&body), MAX_SPEECH_BYTES)?;
        let audio = document
            .get("audio_data")
            .and_then(Value::as_str)
            .ok_or(BackendError::Protocol)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio)
            .map_err(|_| BackendError::Protocol)?;
        if bytes.is_empty() {
            return Err(BackendError::Protocol);
        }
        Ok(bytes)
    }

    /// The owner's own voices first, then the presets (first 100).
    pub fn voices(&self) -> Result<Vec<Voice>, BackendError> {
        let document = self.json(
            "/v2/audio/voices?type=all&page_size=100",
            None,
            MAX_CHAT_BYTES,
        )?;
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let mut voices: Vec<Voice> = document
            .get("data")
            .and_then(Value::as_array)
            .ok_or(BackendError::Protocol)?
            .iter()
            .filter_map(|v| {
                Some(Voice {
                    custom: v.get("type").and_then(Value::as_str) == Some("custom"),
                    id: v.get("id")?.as_str()?.to_owned(),
                    name: v
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Voice")
                        .to_owned(),
                    languages: strings(v.get("languages")),
                    gender: v.get("gender").and_then(Value::as_str).map(str::to_owned),
                    slug: v
                        .get("slug")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect();
        voices.sort_by_key(|v| !v.custom);
        Ok(voices)
    }
}

/// [`Mistral`] with a chat model: what the category runner calls.
#[derive(Debug)]
pub struct Chat<'a> {
    pub client: &'a Mistral,
    pub model: &'a str,
}

impl Generate for Chat<'_> {
    fn generate(&self, prompt: &str, system: Option<&str>) -> Result<String, BackendError> {
        self.client.chat_json(self.model, system, prompt, 0.0)
    }
}

impl Embed for Mistral {
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>, BackendError> {
        self.embeddings(inputs)
    }
}

/// A short, data-free reason for the owner.
pub fn message(error: &BackendError) -> String {
    match error {
        BackendError::Unavailable => "Mistral could not be reached.".into(),
        BackendError::Status(401 | 403) => "Mistral rejected the API key.".into(),
        BackendError::Status(429) => "Mistral is rate limiting; it is tried again later.".into(),
        BackendError::Status(code) => format!("Mistral returned HTTP {code}."),
        BackendError::Protocol => "Mistral returned an invalid answer.".into(),
        BackendError::Invalid(reason) => (*reason).to_owned(),
    }
}

#[cfg(any(test, feature = "fake"))]
pub mod fake;

#[cfg(test)]
mod tests;
