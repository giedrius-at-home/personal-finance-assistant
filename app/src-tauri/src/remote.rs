//! Server mode: when `Finance/server-url` exists, the app is a window onto
//! the Finance server (`app/server`). Every command but the app's own
//! (updates) is posted to the server with the token from the secret store,
//! and the app runs no bank sync, email, suggestions or forecasts itself.
//! Delete the file and restart to go back to the local ledger.

use std::net::IpAddr;
use std::time::Duration;

use finance_secrets::Item;
use serde_json::Value;
use tauri::ipc::{Invoke, InvokeBody};

/// These stay in the app: it updates itself, the server does not.
const LOCAL: [&str; 5] = [
    "update_status",
    "check_for_updates",
    "install_update",
    "set_release_token",
    "forget_release_token",
];

#[derive(Debug)]
pub struct Remote {
    /// The server's address without a trailing slash.
    url: String,
    http: reqwest::blocking::Client,
}

/// A server address the token may be sent to: `https://…` anywhere, plain
/// `http://` only on this Mac or a Tailscale address (100.64.0.0/10), where
/// WireGuard encrypts it.
fn checked(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "server-url is not an address")?;
    let host = parsed.host_str().unwrap_or("");
    let private = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback() || (ip.octets()[0] == 100 && ip.octets()[1] & 0xC0 == 64)
        }
        Ok(IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => host == "localhost" || host.ends_with(".ts.net"),
    };
    match parsed.scheme() {
        "https" => {}
        "http" if private => {}
        _ => return Err("server-url must be https, or http on a Tailscale address".into()),
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

impl Remote {
    /// The server to use, when `Finance/server-url` names one. An address
    /// that cannot be used is an error, never a silent fall back to the
    /// local ledger (the two would drift apart).
    pub fn configured() -> Result<Option<Self>, String> {
        let path = finance_storage::data_dir()
            .map_err(|e| e.to_string())?
            .join("server-url");
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let http = reqwest::blocking::Client::builder()
            .user_agent("Finance.app")
            .connect_timeout(Duration::from_secs(10))
            // A bank sync or a research call can take a while.
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Some(Self {
            url: checked(&text)?,
            http,
        }))
    }

    /// Run `cmd` on the server; its answer or its error, as the UI expects.
    fn call(&self, cmd: &str, args: &Value) -> Result<Value, Value> {
        let fail = |message: &str| Value::String(message.to_owned());
        let token = finance_secrets::get(Item::ServerToken)
            .map_err(|_| fail("The server token could not be read."))?
            .ok_or_else(|| fail("Save the server token first (secrets/server-token)."))?;
        let response = self
            .http
            .post(format!("{}/invoke/{cmd}", self.url))
            .bearer_auth(token.expose().trim())
            .json(args)
            .send()
            .map_err(|_| fail("The Finance server could not be reached."))?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|_| fail("The Finance server gave an unreadable answer."))?;
        if status.is_success() {
            Ok(body)
        } else if status.as_u16() == 401 {
            Err(fail("The Finance server refused the token."))
        } else {
            Err(body)
        }
    }

    /// `cmd` is the server's to answer (all but the app's own).
    pub fn takes(&self, cmd: &str) -> bool {
        !LOCAL.contains(&cmd)
    }

    /// Answer `invoke` from the server. The call runs on its own thread
    /// (blocking HTTP), never on the async runtime.
    pub fn forward(&'static self, invoke: Invoke) {
        let cmd = invoke.message.command().to_owned();
        let args = match invoke.message.payload() {
            InvokeBody::Json(value) => value.clone(),
            InvokeBody::Raw(_) => Value::Null,
        };
        let resolver = invoke.resolver;
        std::thread::spawn(move || match self.call(&cmd, &args) {
            Ok(value) => {
                // The server has no browser: open the bank's approval page here.
                if cmd == "bank_link_start"
                    && let Some(address) = value.as_str().filter(|a| a.starts_with("https://"))
                {
                    let _ = std::process::Command::new("/usr/bin/open")
                        .arg(address)
                        .status();
                }
                resolver.resolve(value);
            }
            Err(error) => resolver.reject(error),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::checked;

    #[test]
    fn token_only_over_tls_or_the_tailnet() {
        assert_eq!(
            checked("http://100.83.10.51:8443/\n").unwrap(),
            "http://100.83.10.51:8443"
        );
        assert!(checked("http://127.0.0.1:18443").is_ok());
        assert!(checked("http://finance.tail1234.ts.net:8443").is_ok());
        assert!(checked("https://money.example.com").is_ok());
        assert!(checked("http://money.example.com").is_err());
        assert!(checked("http://100.128.0.1:8443").is_err());
        assert!(checked("http://84.235.169.99:8443").is_err());
        assert!(checked("ftp://100.83.10.51").is_err());
        assert!(checked("finance").is_err());
    }
}
