//! Blocking HTTP clients for the backends `finance_core::ai` defines.
//! Every call runs on a plain thread, never on the app's async runtime.
//! Errors never carry prompts, responses or credentials.

use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use finance_core::ai::research::{self, Fetch, Fetched, Provider};
use finance_core::ai::retrieval::{check_embedding_inputs, parse_embeddings};
use finance_core::ai::typesafe::{self, ChoiceRequest, ChoiceResult};
use finance_core::ai::{BackendError, Choose, Embed, Generate, settings};
use finance_secrets::Secret;
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use serde_json::{Value, json};

const RETRYABLE: [u16; 4] = [429, 502, 503, 504];
const MAX_GENERATE_BYTES: usize = 64 * 1024;
const MAX_EMBED_BYTES: usize = 4 * 1024 * 1024;

/// Read at most `max` bytes; `None` when the body is larger.
fn read_limited(response: Response, max: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    response.take(max as u64 + 1).read_to_end(&mut body).ok()?;
    (body.len() <= max).then_some(body)
}

/// POST with bounded retries (transport errors and 429/502/503/504, with
/// 0.1 s, 0.2 s … back-off), then the JSON object of a 2xx answer.
fn post_json(
    http: &Client,
    url: &str,
    bearer: Option<&str>,
    body: Vec<u8>,
    attempts: u32,
    max_bytes: usize,
) -> Result<Value, BackendError> {
    let mut response = None;
    for attempt in 1..=attempts {
        let mut request = http
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.clone());
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        response = request.send().ok();
        if response
            .as_ref()
            .is_some_and(|r| !RETRYABLE.contains(&r.status().as_u16()))
        {
            break;
        }
        if attempt < attempts {
            std::thread::sleep(Duration::from_millis(100 * (1 << (attempt - 1))));
        }
    }
    let response = response.ok_or(BackendError::Unavailable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(BackendError::Status(status.as_u16()));
    }
    let json_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
    let body = read_limited(response, max_bytes).ok_or(BackendError::Protocol)?;
    if !json_type {
        return Err(BackendError::Protocol);
    }
    match serde_json::from_slice(&body) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => Err(BackendError::Protocol),
    }
}

/// A local Ollama model (`integrations/ollama.py`): loopback only, no
/// redirects, no proxies.
#[derive(Debug)]
pub struct Ollama {
    base: String,
    model: String,
    http: Client,
    attempts: u32,
}

impl Ollama {
    /// `read_timeout` is 120 s for suggestions and 45 s for group suggestions.
    pub fn new(
        base_url: &str,
        model: &str,
        read_timeout: Duration,
        attempts: u32,
    ) -> Result<Self, BackendError> {
        let base = settings::ollama_base_url(base_url).ok_or(BackendError::Invalid(
            "ollama base_url must use a local loopback host",
        ))?;
        let model = model.trim();
        if model.is_empty() || model.chars().count() > 128 {
            return Err(BackendError::Invalid(
                "model must be a non-empty bounded identifier",
            ));
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(read_timeout)
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| BackendError::Unavailable)?;
        Ok(Self {
            base,
            model: model.to_owned(),
            http,
            attempts: attempts.clamp(1, 5),
        })
    }

    /// The installed models, or `None` when Ollama does not answer.
    pub fn installed(base_url: &str) -> Option<Vec<String>> {
        let base = settings::ollama_base_url(base_url)?;
        let http = Client::builder()
            .connect_timeout(Duration::from_millis(800))
            .timeout(Duration::from_secs(2))
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .ok()?;
        let response = http.get(format!("{base}api/tags")).send().ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: Value = serde_json::from_slice(&read_limited(response, 1024 * 1024)?).ok()?;
        Some(
            body.get("models")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect(),
        )
    }
}

impl Generate for Ollama {
    fn generate(&self, prompt: &str, system: Option<&str>) -> Result<String, BackendError> {
        let mut payload = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": {"temperature": 0},
            "think": false,
        });
        if let Some(system) = system {
            payload["system"] = Value::String(system.to_owned());
        }
        let body = serde_json::to_vec(&payload).map_err(|_| BackendError::Protocol)?;
        let document = post_json(
            &self.http,
            &format!("{}api/generate", self.base),
            None,
            body,
            self.attempts,
            MAX_GENERATE_BYTES,
        )?;
        document
            .get("response")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(BackendError::Protocol)
    }
}

impl Embed for Ollama {
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>, BackendError> {
        check_embedding_inputs(inputs)?;
        let body =
            serde_json::to_vec(&json!({"model": self.model, "input": inputs, "truncate": false}))
                .map_err(|_| BackendError::Protocol)?;
        let document = post_json(
            &self.http,
            &format!("{}api/embed", self.base),
            None,
            body,
            self.attempts,
            MAX_EMBED_BYTES,
        )?;
        parse_embeddings(&document, inputs.len())
    }
}

/// TypeSafe System One (`TypeSafeClient`). The key stays in this struct
/// and is wiped when it is dropped.
pub struct TypeSafe {
    key: Secret,
    model: String,
    http: Client,
    attempts: u32,
}

impl std::fmt::Debug for TypeSafe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TypeSafe(<redacted>)")
    }
}

/// A key TypeSafe could accept: 1–512 ASCII characters, no whitespace.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 512 && key.is_ascii() && !key.chars().any(char::is_whitespace)
}

impl TypeSafe {
    pub fn new(key: Secret, model: &str, attempts: u32) -> Result<Self, BackendError> {
        if !valid_key(key.expose()) {
            return Err(BackendError::Invalid("TypeSafe API key is invalid"));
        }
        let model = model.trim();
        if model.is_empty() || model.chars().count() > 128 {
            return Err(BackendError::Invalid(
                "TypeSafe model must be a non-empty bounded identifier",
            ));
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::none())
            .no_proxy()
            .https_only(true)
            .build()
            .map_err(|_| BackendError::Unavailable)?;
        Ok(Self {
            key,
            model: model.to_owned(),
            http,
            attempts: attempts.clamp(1, 5),
        })
    }
}

impl Choose for TypeSafe {
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceResult, BackendError> {
        request.validate()?;
        let body =
            serde_json::to_vec(&request.body(&self.model)).map_err(|_| BackendError::Protocol)?;
        let document = post_json(
            &self.http,
            typesafe::TYPESAFE_URL,
            Some(self.key.expose()),
            body,
            self.attempts,
            typesafe::MAX_RESPONSE_BYTES,
        )?;
        let options: Vec<String> = request.criteria.iter().map(|(o, _)| o.clone()).collect();
        typesafe::parse_choice(&document, &options)
    }
}

/// Only publicly routable addresses (Python `ipaddress.is_global`).
pub fn is_global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..=19).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_global(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || first == 0x2001 && v6.segments()[1] == 0x0db8
                || v6.is_multicast())
        }
    }
}

/// Public research over HTTPS (`RestrictedResearchGateway._fetch`): every
/// hop is checked, resolved to public addresses and pinned to them.
#[derive(Debug, Default)]
pub struct Web;

impl Web {
    fn hop(url: &str, provider: Provider) -> Result<Response, String> {
        if !research::allowed(url, provider) {
            return Err("destination_rejected".into());
        }
        let host = provider.host();
        let addresses: Vec<SocketAddr> = (host, 443)
            .to_socket_addrs()
            .map_err(|_| "dns_unavailable".to_owned())?
            .collect();
        if addresses.is_empty() {
            return Err("dns_unavailable".into());
        }
        if addresses.iter().any(|a| !is_global(a.ip())) {
            return Err("non_public_destination".into());
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .no_proxy()
            .https_only(true)
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| "network_unavailable".to_owned())?;
        http.get(url)
            .header(
                "Accept",
                "application/json, application/atom+xml, text/html;q=0.5",
            )
            .header("User-Agent", research::USER_AGENT)
            .send()
            .map_err(|_| "network_unavailable".to_owned())
    }
}

impl Fetch for Web {
    fn get(&self, url: &str, provider: Provider) -> Result<Fetched, String> {
        let mut url = url.to_owned();
        for redirects in 0..=research::MAX_REDIRECTS {
            let response = Self::hop(&url, provider)?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if redirects == research::MAX_REDIRECTS {
                    return Err("redirect_limit".into());
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or("invalid_redirect")?;
                url = reqwest::Url::parse(&url)
                    .and_then(|base| base.join(location))
                    .map_err(|_| "invalid_redirect".to_owned())?
                    .to_string();
                continue;
            }
            if status != 200 {
                return Err("upstream_status".into());
            }
            if let Some(length) = response.headers().get("content-length") {
                let length: usize = length
                    .to_str()
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .ok_or("invalid_content_length")?;
                if length > research::MAX_RESEARCH_BYTES {
                    return Err("response_too_large".into());
                }
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let final_url = response.url().to_string();
            let body =
                read_limited(response, research::MAX_RESEARCH_BYTES).ok_or("response_too_large")?;
            return Ok(Fetched {
                url: final_url,
                content_type,
                body,
            });
        }
        Err("redirect_limit".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_global_addresses_are_public() {
        for private in [
            "10.0.0.1",
            "127.0.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_global(private.parse().unwrap()), "{private}");
        }
        for public in ["208.80.154.224", "2620:0:861:ed1a::1"] {
            assert!(is_global(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn rejects_bad_keys_and_remote_ollama() {
        assert!(!valid_key("has space"));
        assert!(!valid_key(""));
        assert!(valid_key("ts_live_abc123"));
        assert!(
            Ollama::new(
                "http://example.com:11434",
                "qwen3:8b",
                Duration::from_secs(1),
                1
            )
            .is_err()
        );
        assert!(Ollama::new("http://127.0.0.1:11434", " ", Duration::from_secs(1), 1).is_err());
    }

    /// Needs a local Ollama with `qwen3:8b` (or `OLLAMA_URL` / `OLLAMA_MODEL`):
    /// `cargo test -p finance-ai -- --ignored`. Runs the whole proposal path (prompt, generation, validation, check).
    #[test]
    #[ignore = "calls the local Ollama"]
    fn live_qwen_proposes_and_checks_a_category() {
        use finance_core::ai::categorise::{Qwen, Stamp, propose};
        use finance_core::ai::check::{self, MerchantIndex};
        use finance_core::ai::history::History;
        use finance_core::model::{Kind, Tx};
        use finance_types::report::review::CategorisationMethod;

        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:8b".into());
        assert!(
            Ollama::installed(&url).is_some_and(|m| m.contains(&model)),
            "{model} is not installed"
        );
        let ollama = Ollama::new(&url, &model, Duration::from_secs(120), 1).unwrap();
        let tx = Tx {
            transaction_id: "firefly:1".into(),
            ledger_id: 1,
            account_id: "firefly:2".into(),
            counterparty_account_id: None,
            kind: Kind::Outflow,
            amount: "4.20".parse().unwrap(),
            currency: finance_types::report::Currency {
                code: "EUR".into(),
                minor_unit: 2,
            },
            occurred_at: "2026-10-27T09:00:00+01:00[Europe/Amsterdam]"
                .parse()
                .unwrap(),
            purchased_on: None,
            description: Some("BEA, Betaalpas Albert Heijn 1234,PAS123 Amsterdam".into()),
            counterparty_name: None,
            group_id: None,
            category: None,
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        };
        let vocabulary = History {
            categories: vec!["Groceries".into(), "Dining".into(), "Transport".into()],
            rules: Default::default(),
            examples: Vec::new(),
            retrieval: Vec::new(),
            version: "v".into(),
        };
        let checker = check::Ollama { backend: &ollama };
        let stamp = Stamp {
            model_version: &model,
            prompt_version: "live",
            baseline: true,
            now: "2026-10-27T09:00:00.000000Z",
        };
        let p = propose(
            &stamp,
            &vocabulary,
            &Qwen { backend: &ollama },
            Some(&checker),
            &MerchantIndex::default(),
            &tx,
            &[],
        )
        .unwrap();
        // A model may answer outside the pinned schema (qwen3:14b sometimes
        // adds keys); that must abstain, exactly as the Python service did.
        match p.method {
            CategorisationMethod::Model => {
                assert_eq!(p.proposed_category.as_deref(), Some("Groceries"));
                assert!(p.confidence.is_some());
                assert_ne!(
                    p.sanity_check.status,
                    finance_types::report::review::CheckStatus::Unchecked
                );
            }
            _ => assert!(
                ["schema_violation", "malformed_output"].contains(&p.reasons[0].as_str()),
                "{:?}",
                p.reasons
            ),
        }
        assert_eq!(p.model_version.as_deref(), Some(model.as_str()));
    }
}
