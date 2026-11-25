//! The app side of bank sync: the Enable Banking application credentials and
//! each bank's consent in the secret store, an authenticated HTTP client, and
//! [`Syncer`], which runs one sync at a time for the commands and the daily
//! scheduler. The sync itself is `finance_core::bank_sync`.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use finance_core::bank_sync::link::{self, Session};
use finance_core::bank_sync::{self, Api, ApiError, Connection, Context};
use finance_core::service::Service;
use finance_secrets::{Bank, Item, Secret};
use finance_types::bank::{
    BANKS, BankConsent, BankCredentials, BankSyncReport, BankSyncResult, SyncStatus, bank_name,
};
use jiff::Timestamp;
use serde_json::Value;

const API: &str = "https://api.enablebanking.com";
/// Enable Banking accepts application tokens for at most a day; the
/// importer used an hour.
const TOKEN_SECONDS: i64 = 3600;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The private key is not a PEM RSA key")]
    Key,
    #[error("The application id is empty")]
    AppId,
    #[error("Secret store: {0}")]
    Secrets(#[from] finance_secrets::Error),
}

/// The Enable Banking application: its id and private key.
pub struct Credentials {
    app_id: String,
    key: RsaKeyPair,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials(<redacted>)")
    }
}

/// The DER bytes of the first PEM block and whether it is PKCS#1
/// (`RSA PRIVATE KEY`) rather than PKCS#8 (`PRIVATE KEY`).
fn pem(text: &str) -> Option<(Vec<u8>, bool)> {
    let begin = text.find("-----BEGIN ")?;
    let rest = &text[begin + 11..];
    let label_end = rest.find("-----")?;
    let label = &rest[..label_end];
    let body = &rest[label_end + 5..];
    let end = body.find("-----END ")?;
    let base64: String = body[..end].chars().filter(|c| !c.is_whitespace()).collect();
    let der = STANDARD.decode(base64).ok()?;
    match label {
        "PRIVATE KEY" => Some((der, false)),
        "RSA PRIVATE KEY" => Some((der, true)),
        _ => None,
    }
}

impl Credentials {
    pub fn parse(app_id: &str, private_key: &str) -> Result<Self, Error> {
        let app_id = app_id.trim();
        if app_id.is_empty() {
            return Err(Error::AppId);
        }
        let (der, pkcs1) = pem(private_key).ok_or(Error::Key)?;
        let key = if pkcs1 {
            RsaKeyPair::from_der(&der)
        } else {
            RsaKeyPair::from_pkcs8(&der)
        }
        .map_err(|_| Error::Key)?;
        Ok(Self {
            app_id: app_id.to_owned(),
            key,
        })
    }

    /// The stored credentials, if both parts are stored.
    pub fn load() -> Result<Option<Self>, Error> {
        let (Some(app_id), Some(key)) = (
            finance_secrets::get(Item::EnableBankingAppId)?,
            finance_secrets::get(Item::EnableBankingKey)?,
        ) else {
            return Ok(None);
        };
        Self::parse(app_id.expose(), key.expose()).map(Some)
    }

    /// Check and store the owner's credentials. Nothing is kept when the
    /// key does not parse.
    pub fn store(credentials: &BankCredentials) -> Result<(), Error> {
        let key = Secret::new(credentials.private_key.trim().to_owned());
        Self::parse(&credentials.app_id, key.expose())?;
        finance_secrets::set(
            Item::EnableBankingAppId,
            &Secret::new(credentials.app_id.trim().to_owned()),
        )?;
        finance_secrets::set(Item::EnableBankingKey, &key)?;
        Ok(())
    }

    pub fn forget() -> Result<(), Error> {
        finance_secrets::delete(Item::EnableBankingAppId)?;
        finance_secrets::delete(Item::EnableBankingKey)?;
        Ok(())
    }

    /// An RS256 application token (`JWTManager::generateToken`): the key id
    /// is the application id.
    pub fn token(&self, now: i64) -> Result<String, Error> {
        let header = serde_json::json!({"typ": "JWT", "alg": "RS256", "kid": self.app_id});
        let claims = serde_json::json!({
            "iss": "enablebanking.com",
            "aud": "api.enablebanking.com",
            "iat": now,
            "exp": now + TOKEN_SECONDS,
        });
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let mut signature = vec![0; self.key.public_modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| Error::Key)?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

/// Authenticated `GET`s against the Enable Banking API.
#[derive(Debug)]
pub struct Client {
    credentials: Credentials,
    http: reqwest::blocking::Client,
    token: Option<(String, i64)>,
    /// What the bank answered to the last failed request.
    failure: Option<String>,
}

/// The kind of request `path` is, for failure messages (never the ids).
fn request_kind(path: &str) -> &'static str {
    let path = path.split('?').next().unwrap_or("");
    if path.starts_with("sessions/") {
        "consent check"
    } else if path == "sessions" {
        "opening the session"
    } else if path == "auth" {
        "authorisation"
    } else if path.starts_with("aspsps") {
        "bank list"
    } else if path.ends_with("/transactions") {
        "transactions"
    } else if path.ends_with("/balances") {
        "balances"
    } else if path.ends_with("/details") {
        "account details"
    } else {
        "request"
    }
}

/// A short, data-free description of an Enable Banking error answer: the
/// HTTP status, the error code and the bank's message.
fn describe_failure(kind: &str, status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .map(|s| s.chars().take(160).collect::<String>())
    };
    let mut text = format!("{kind}: HTTP {status}");
    if let Some(code) = field("error") {
        text.push(' ');
        text.push_str(&code);
    }
    if let Some(message) = field("message") {
        text.push_str(" — ");
        text.push_str(&message);
    }
    text
}

impl Client {
    pub fn new(credentials: Credentials) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .https_only(true)
            .build()
            .map_err(|_| "The HTTP client could not start".to_owned())?;
        Ok(Self {
            credentials,
            http,
            token: None,
            failure: None,
        })
    }

    fn bearer(&mut self) -> Result<String, ApiError> {
        let now = Timestamp::now().as_second();
        match &self.token {
            Some((token, expires)) if *expires - 60 > now => Ok(token.clone()),
            _ => {
                let token = self
                    .credentials
                    .token(now)
                    .map_err(|_| ApiError::Unavailable)?;
                self.token = Some((token.clone(), now + TOKEN_SECONDS));
                Ok(token)
            }
        }
    }
}

impl Client {
    fn send(
        &mut self,
        path: &str,
        request: reqwest::blocking::RequestBuilder,
    ) -> Result<Value, ApiError> {
        let kind = request_kind(path);
        let token = self.bearer()?;
        let response = match request
            .bearer_auth(token)
            .header("Accept", "application/json")
            .send()
        {
            Ok(response) => response,
            Err(e) => {
                let what = if e.is_timeout() {
                    "timed out"
                } else {
                    "network error"
                };
                self.failure = Some(format!("{kind}: {what}"));
                return Err(ApiError::Unavailable);
            }
        };
        let status = response.status();
        let body = response.text().map_err(|_| ApiError::Unavailable)?;
        if status.as_u16() == 429 || body.contains("ASPSP_RATE_LIMIT_EXCEEDED") {
            self.failure = Some(describe_failure(kind, status.as_u16(), &body));
            return Err(ApiError::RateLimited);
        }
        if !status.is_success() {
            self.failure = Some(describe_failure(kind, status.as_u16(), &body));
            return Err(ApiError::Unavailable);
        }
        serde_json::from_str(&body).map_err(|_| {
            self.failure = Some(format!("{kind}: unreadable answer"));
            ApiError::Unavailable
        })
    }
}

impl Api for Client {
    fn get(&mut self, path: &str) -> Result<Value, ApiError> {
        let request = self.http.get(format!("{API}/{path}"));
        self.send(path, request)
    }

    fn post(&mut self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let request = self.http.post(format!("{API}/{path}")).json(body);
        self.send(path, request)
    }

    fn last_failure(&self) -> Option<String> {
        self.failure.clone()
    }
}

fn keychain_bank(bank: &str) -> Result<Bank, String> {
    Bank::from_id(bank).ok_or_else(|| "Unknown bank".to_owned())
}

/// The stored consent of `bank`.
pub fn consent(bank: &str) -> Result<Option<Connection>, String> {
    let item = Item::BankConsent(keychain_bank(bank)?);
    let Some(secret) = finance_secrets::get(item).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(serde_json::from_str(secret.expose()).ok())
}

fn store_consent(bank: &str, connection: &Connection) -> Result<(), String> {
    let item = Item::BankConsent(keychain_bank(bank)?);
    let json = serde_json::to_string(connection).map_err(|e| e.to_string())?;
    finance_secrets::set(item, &Secret::new(json)).map_err(|e| e.to_string())
}

/// Where the old importer kept its consents and `status.json`.
fn legacy_dir() -> Option<PathBuf> {
    dirs_home().map(|home| {
        home.join("Library/Application Support/Personal Finance Assistant/Bank Imports/data")
    })
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn legacy_file(bank: &str) -> Option<PathBuf> {
    legacy_dir()
        .map(|d| d.join(format!("{bank}.json")))
        .filter(|p| p.is_file())
}

/// A connection the owner started and has not finished.
#[derive(Debug)]
struct Link {
    bank: String,
    state: String,
    session: Option<Session>,
}

/// One sync at a time, for the commands and the scheduler.
#[derive(Debug, Default)]
pub struct Syncer {
    running: Mutex<Option<String>>,
    last_check: Mutex<Option<Timestamp>>,
    link: Mutex<Option<Link>>,
}

fn random_state() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|_| "No randomness available".to_owned())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn refused(e: ApiError) -> String {
    match e {
        ApiError::RateLimited => {
            "Enable Banking is rate limiting requests. Try again later.".into()
        }
        ApiError::Unavailable => {
            "Enable Banking refused the request or could not be reached.".into()
        }
    }
}

/// Clears the running bank when a sync ends, however it ends.
struct Running<'a>(&'a Mutex<Option<String>>);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if let Ok(mut running) = self.0.lock() {
            *running = None;
        }
    }
}

impl Syncer {
    fn begin(&self, bank: &str) -> Result<Running<'_>, String> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| "sync-poisoned".to_owned())?;
        if let Some(other) = running.as_deref() {
            return Err(format!(
                "{} is syncing. Try again when it finishes.",
                bank_name(other)
            ));
        }
        *running = Some(bank.to_owned());
        Ok(Running(&self.running))
    }

    /// The Banks screen.
    pub fn report(&self, service: &Service) -> Result<BankSyncReport, String> {
        let mut connected = HashMap::new();
        for (id, _) in BANKS {
            if let Ok(Some(c)) = consent(id) {
                connected.insert(id.to_owned(), c.targets());
            }
        }
        let credentials = matches!(
            (
                finance_secrets::get(Item::EnableBankingAppId),
                finance_secrets::get(Item::EnableBankingKey)
            ),
            (Ok(Some(_)), Ok(Some(_)))
        );
        let legacy_consents = BANKS
            .iter()
            .any(|(id, _)| !connected.contains_key(*id) && legacy_file(id).is_some());
        service.bank_sync_report(Context {
            connected,
            credentials,
            running: self.running.lock().ok().and_then(|r| r.clone()),
            legacy_consents,
            last_check: self.last_check.lock().ok().and_then(|c| *c),
        })
    }

    fn client() -> Result<Client, SyncStatus> {
        match Credentials::load() {
            Ok(Some(credentials)) => {
                Client::new(credentials).map_err(|_| SyncStatus::RuntimeUnavailable)
            }
            _ => Err(SyncStatus::MissingCredentials),
        }
    }

    /// Sync `bank` now (the owner asked). A rate limit still holding is
    /// refused; anything else is recorded as the bank's latest run.
    pub fn sync(&self, service: &Service, bank: &str) -> Result<BankSyncResult, String> {
        keychain_bank(bank)?;
        let now = Timestamp::now();
        let run = service.bank_runs()?.remove(bank).unwrap_or_default();
        if bank_sync::retry_after(&run, now).is_some() {
            return Err(format!(
                "{} is rate limited. Try again later.",
                bank_name(bank)
            ));
        }
        let _running = self.begin(bank)?;
        self.attempt(service, bank, now)
    }

    fn attempt(
        &self,
        service: &Service,
        bank: &str,
        now: Timestamp,
    ) -> Result<BankSyncResult, String> {
        let failed = |status: SyncStatus| -> Result<BankSyncResult, String> {
            let previous = service.bank_runs()?.remove(bank).unwrap_or_default();
            service.save_bank_run(bank, &bank_sync::record(&previous, status, None, now))?;
            Ok(BankSyncResult {
                bank: bank.to_owned(),
                status,
                created: None,
                checked: None,
            })
        };
        let Some(connection) = consent(bank)? else {
            return failed(SyncStatus::MissingConfig);
        };
        let mut client = match Self::client() {
            Ok(client) => client,
            Err(status) => return failed(status),
        };
        service.sync_bank(bank, &connection, &mut client, now)
    }

    /// The scheduler's tick: sync every connected bank that is due, in
    /// order. Without credentials nothing runs.
    pub fn sync_due(&self, service: &Service) -> Vec<BankSyncResult> {
        let now = Timestamp::now();
        if let Ok(mut last) = self.last_check.lock() {
            *last = Some(now);
        }
        if !matches!(Credentials::load(), Ok(Some(_))) {
            return Vec::new();
        }
        let Ok(runs) = service.bank_runs() else {
            return Vec::new();
        };
        // The extra syncs after the owner's usual buying hours.
        let slots = service.sync_slots(now).unwrap_or_default();
        let mut results = Vec::new();
        for (bank, _) in BANKS {
            let due = runs
                .get(bank)
                .is_none_or(|run| finance_core::timing::is_due_adaptive(run, now, &slots));
            if !due || !matches!(consent(bank), Ok(Some(_))) {
                continue;
            }
            let Ok(_running) = self.begin(bank) else {
                continue;
            };
            if let Ok(result) = self.attempt(service, bank, now) {
                results.push(result);
            }
        }
        results
    }

    /// Replace `bank`'s consent with a renewed importer configuration: it
    /// must keep every ledger account mapped and pass the bank's checks,
    /// then the bank syncs.
    pub fn renew(
        &self,
        service: &Service,
        request: &BankConsent,
    ) -> Result<BankSyncResult, String> {
        let bank = request.bank.as_str();
        keychain_bank(bank)?;
        if request.configuration.len() > 64_000 {
            return Err("Configuration exceeds 64 KB.".to_owned());
        }
        let _running = self.begin(bank)?;
        let now = Timestamp::now();
        let old = consent(bank)?;
        let candidate = Connection::renew(old.as_ref(), &request.configuration)
            .map_err(|status| format!("Consent not renewed: {}.", status.reason()))?;
        let mut client = Self::client().map_err(|status| status.reason().to_owned())?;
        service
            .verify_bank(&candidate, &mut client, now)
            .map_err(|status| format!("Consent not renewed: {}.", status.reason()))?;
        store_consent(bank, &candidate)?;
        service.sync_bank(bank, &candidate, &mut client, now)
    }

    /// Start connecting `start.bank`: returns the Enable Banking address
    /// where the owner approves access at the bank.
    pub fn link_start(&self, start: &finance_types::bank::BankLinkStart) -> Result<String, String> {
        let (_, country) = finance_types::bank::aspsp(&start.bank).ok_or("Unknown bank")?;
        let mut client = Self::client().map_err(|s| s.reason().to_owned())?;
        let aspsps = client.get(&format!("aspsps?country={country}")).ok();
        let state = random_state()?;
        let body = link::auth_body(
            &start.bank,
            start.redirect_url.trim(),
            &state,
            Timestamp::now(),
            aspsps.as_ref(),
        )?;
        let answer = client.post("auth", &body).map_err(refused)?;
        let url = answer
            .get("url")
            .and_then(Value::as_str)
            .filter(|u| u.starts_with("https://"))
            .ok_or("Enable Banking returned no approval address.")?
            .to_owned();
        *self.link.lock().map_err(|_| "link-poisoned".to_owned())? = Some(Link {
            bank: start.bank.clone(),
            state,
            session: None,
        });
        Ok(url)
    }

    /// Finish the approval: `address` is the page the bank redirected to.
    /// Opens the session and proposes how its accounts map.
    pub fn link_finish(
        &self,
        service: &Service,
        bank: &str,
        address: &str,
    ) -> Result<finance_types::bank::BankLinkProposal, String> {
        let mut guard = self.link.lock().map_err(|_| "link-poisoned".to_owned())?;
        let pending = guard
            .as_mut()
            .filter(|l| l.bank == bank)
            .ok_or("Start the connection first.")?;
        let code = link::callback_code(address.trim(), &pending.state)?;
        let mut client = Self::client().map_err(|s| s.reason().to_owned())?;
        let answer = client
            .post("sessions", &serde_json::json!({"code": code}))
            .map_err(refused)?;
        let session = Session::parse(&answer)?;
        let own = service.bank_own_accounts()?;
        let old = consent(bank)?;
        let proposal = link::proposal(bank, old.as_ref(), &session, &own);
        pending.session = Some(session);
        Ok(proposal)
    }

    /// Store the owner's mapping as `bank`'s consent once the bank accepts
    /// it, then sync.
    pub fn link_confirm(
        &self,
        service: &Service,
        request: &finance_types::bank::BankLinkConfirm,
    ) -> Result<BankSyncResult, String> {
        let bank = request.bank.as_str();
        keychain_bank(bank)?;
        let session = self
            .link
            .lock()
            .map_err(|_| "link-poisoned".to_owned())?
            .as_ref()
            .filter(|l| l.bank == bank)
            .and_then(|l| l.session.clone())
            .ok_or("Approve access at the bank first.")?;
        let _running = self.begin(bank)?;
        let now = Timestamp::now();
        let own = service.bank_own_accounts()?;
        let old = consent(bank)?;
        let connection = link::confirm(old.as_ref(), &session, &request.choices, &own)?;
        let mut client = Self::client().map_err(|s| s.reason().to_owned())?;
        service
            .verify_bank(&connection, &mut client, now)
            .map_err(|status| format!("Not connected: {}.", status.reason()))?;
        store_consent(bank, &connection)?;
        if let Ok(mut link) = self.link.lock() {
            *link = None;
        }
        service.sync_bank(bank, &connection, &mut client, now)
    }

    /// Take over the consents the old importer saved on this Mac, with its
    /// run history so the daily cadence continues. Banks that already have
    /// a consent here are left alone. Returns the banks taken over.
    pub fn adopt_legacy(&self, service: &Service) -> Result<Vec<String>, String> {
        let runs = legacy_dir()
            .and_then(|d| std::fs::read_to_string(d.join("status.json")).ok())
            .map(|raw| bank_sync::legacy_runs(&raw))
            .unwrap_or_default();
        let stored = service.bank_runs()?;
        let mut adopted = Vec::new();
        for (bank, _) in BANKS {
            if consent(bank)?.is_some() {
                continue;
            }
            let Some(path) = legacy_file(bank) else {
                continue;
            };
            let raw = Secret::new(std::fs::read_to_string(path).map_err(|e| e.to_string())?);
            let connection = Connection::from_importer(raw.expose())
                .map_err(|status| format!("{}: {}.", bank_name(bank), status.reason()))?;
            store_consent(bank, &connection)?;
            if let (Some(run), false) = (runs.get(bank), stored.contains_key(bank)) {
                service.save_bank_run(bank, run)?;
            }
            adopted.push(bank.to_owned());
        }
        Ok(adopted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_keys_that_are_not_rsa_pem() {
        assert!(matches!(
            Credentials::parse("app", "not a key"),
            Err(Error::Key)
        ));
        assert!(matches!(Credentials::parse(" ", "x"), Err(Error::AppId)));
        assert!(matches!(
            Credentials::parse(
                "app",
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----"
            ),
            Err(Error::Key)
        ));
    }

    #[test]
    fn signs_an_rs256_token_the_api_accepts() {
        use aws_lc_rs::encoding::AsDer;
        use aws_lc_rs::rsa::{KeyPair, KeySize};
        use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};

        let generated = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let der = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der<'_>>::as_der(&generated).unwrap();
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            STANDARD.encode(der.as_ref())
        );
        let credentials = Credentials::parse("app-id", &pem).unwrap();
        let token = credentials.token(1_700_000_000).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["kid"], "app-id");
        assert_eq!(header["alg"], "RS256");
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "api.enablebanking.com");
        assert_eq!(claims["exp"], 1_700_003_600);
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, generated.public_key().as_ref())
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn failures_name_the_request_and_the_bank_code() {
        assert_eq!(
            request_kind("accounts/abc/transactions?date_from=x"),
            "transactions"
        );
        assert_eq!(request_kind("sessions/abc"), "consent check");
        assert_eq!(
            describe_failure(
                "transactions",
                422,
                r#"{"code":422,"error":"WRONG_TRANSACTIONS_PERIOD","message":"Wrong transactions period requested"}"#
            ),
            "transactions: HTTP 422 WRONG_TRANSACTIONS_PERIOD — Wrong transactions period requested"
        );
        assert_eq!(
            describe_failure("balances", 500, "<html>"),
            "balances: HTTP 500"
        );
    }
}
