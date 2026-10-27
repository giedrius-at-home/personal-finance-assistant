//! The app side of bank sync: the Enable Banking application credentials and
//! each bank's consent in the Keychain, an authenticated HTTP client, and
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
    #[error("Keychain: {0}")]
    Keychain(#[from] finance_secrets::Error),
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

    /// The stored credentials, if both parts are in the Keychain.
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

impl Api for Client {
    fn get(&mut self, path: &str) -> Result<Value, ApiError> {
        let token = self.bearer()?;
        let response = self
            .http
            .get(format!("{API}/{path}"))
            .bearer_auth(token)
            .header("Accept", "application/json")
            .send()
            .map_err(|_| ApiError::Unavailable)?;
        let status = response.status();
        let body = response.text().map_err(|_| ApiError::Unavailable)?;
        if status.as_u16() == 429 || body.contains("ASPSP_RATE_LIMIT_EXCEEDED") {
            return Err(ApiError::RateLimited);
        }
        if !status.is_success() {
            return Err(ApiError::Unavailable);
        }
        serde_json::from_str(&body).map_err(|_| ApiError::Unavailable)
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

/// One sync at a time, for the commands and the scheduler.
#[derive(Debug, Default)]
pub struct Syncer {
    running: Mutex<Option<String>>,
    last_check: Mutex<Option<Timestamp>>,
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
        let mut results = Vec::new();
        for (bank, _) in BANKS {
            let due = runs.get(bank).is_none_or(|run| bank_sync::is_due(run, now));
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
}
