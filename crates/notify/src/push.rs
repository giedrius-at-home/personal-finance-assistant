//! Web Push to the installed web app on the phone: the VAPID key that signs
//! for the server (RFC 8292), the message encryption every push service
//! requires (RFC 8291, `aes128gcm` of RFC 8188), and the POST to the
//! device's push service. Blocking, like the mailer: callers run it on
//! their own thread.

use std::sync::Arc;
use std::time::Duration;

use aws_lc_rs::agreement::{self, ECDH_P256, PrivateKey, UnparsedPublicKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use aws_lc_rs::{aead, hkdf};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use finance_core::alerts::{Notification, Notifier, Outcome};
use finance_types::notify::PushSubscription;
use serde_json::json;

/// One record holds the whole message (RFC 8291 §4: 4096 octets).
const RECORD_SIZE: u32 = 4096;
/// What fits in one record after the padding delimiter and the GCM tag.
const MAX_PLAINTEXT: usize = 3993;
/// How long a push service keeps a message for a phone that is offline.
const TTL_SECONDS: u32 = 24 * 3600;
/// The push services browsers subscribe with; any other endpoint is refused,
/// so a stored subscription cannot point the server anywhere else.
const PUSH_HOSTS: [&str; 5] = [
    "web.push.apple.com",
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "notify.windows.com",
    "push.services.mozilla.com",
];

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(text.trim().trim_end_matches('='))
        .map_err(|_| "The subscription keys are not base64url.".to_owned())
}

/// Whether `endpoint` is an https URL on a known push service.
pub fn valid_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && PUSH_HOSTS
            .iter()
            .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Check a subscription the web app sent before it is stored.
pub fn check_subscription(s: &PushSubscription) -> Result<(), String> {
    if s.endpoint.len() > 2048 || !valid_endpoint(&s.endpoint) {
        return Err("That push service is not one this server sends to.".into());
    }
    if unb64(&s.p256dh)?.len() != 65 || unb64(&s.auth)?.len() != 16 {
        return Err("The subscription keys are not valid.".into());
    }
    if !(s.origin.starts_with("https://") || s.origin.starts_with("http://localhost"))
        || s.origin.len() > 256
    {
        return Err("The subscription has no page origin.".into());
    }
    Ok(())
}

/// The server's VAPID key pair (ECDSA P-256).
#[derive(Debug)]
pub struct Vapid {
    key: EcdsaKeyPair,
}

impl Vapid {
    /// A new key and its PKCS#8 form in base64url, for the secret store.
    pub fn generate() -> Result<(Self, String), String> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| "Could not make a push key.".to_owned())?;
        let encoded = b64(pkcs8.as_ref());
        Ok((Self::from_pkcs8(&encoded)?, encoded))
    }

    pub fn from_pkcs8(encoded: &str) -> Result<Self, String> {
        let der = unb64(encoded).map_err(|_| "The stored push key is not valid.".to_owned())?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der)
            .map_err(|_| "The stored push key is not valid.".to_owned())?;
        Ok(Self { key })
    }

    /// The uncompressed public key, base64url: the web app's
    /// `applicationServerKey` and the `k` of the Authorization header.
    pub fn public_key(&self) -> String {
        b64(self.key.public_key().as_ref())
    }

    /// The `Authorization: vapid t=…, k=…` header for `endpoint`, valid for
    /// 12 hours from `now` (Unix seconds).
    pub fn authorization(&self, endpoint: &str, subject: &str, now: i64) -> Result<String, String> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "Bad push endpoint.".to_owned())?;
        let audience = url.origin().ascii_serialization();
        let header = b64(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = b64(
            json!({"aud": audience, "exp": now + 12 * 3600, "sub": subject})
                .to_string()
                .as_bytes(),
        );
        let signing_input = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .map_err(|_| "Could not sign the push request.".to_owned())?;
        Ok(format!(
            "vapid t={signing_input}.{}, k={}",
            b64(signature.as_ref()),
            self.public_key()
        ))
    }
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[u8], len: usize) -> Result<Vec<u8>, String> {
    let fail = || "Push encryption failed.".to_owned();
    let info = [info];
    let okm = prk.expand(&info, Len(len)).map_err(|_| fail())?;
    let mut out = vec![0; len];
    okm.fill(&mut out).map_err(|_| fail())?;
    Ok(out)
}

/// The content key and nonce of RFC 8291 §3.4 for one message.
fn keys(
    ecdh_secret: &[u8],
    auth: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(ecdh_secret);
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let ikm = expand(&prk_key, &key_info, 32)?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    Ok((
        expand(&prk, b"Content-Encoding: aes128gcm\0", 16)?,
        expand(&prk, b"Content-Encoding: nonce\0", 12)?,
    ))
}

fn ecdh(private: &PrivateKey, peer: &[u8]) -> Result<Vec<u8>, String> {
    agreement::agree(
        private,
        UnparsedPublicKey::new(&ECDH_P256, peer),
        "The device's push key is not valid.".to_owned(),
        |secret| Ok(secret.to_vec()),
    )
}

/// RFC 8291 with a given sender key and salt (the RFC's example uses fixed
/// ones; real messages get fresh ones from [`encrypt`]).
fn encrypt_with(
    plaintext: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    as_private: &PrivateKey,
    salt: &[u8; 16],
) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err("The push message is too long.".into());
    }
    let fail = || "Push encryption failed.".to_owned();
    let as_public = as_private.compute_public_key().map_err(|_| fail())?;
    let as_public = as_public.as_ref();
    let secret = ecdh(as_private, ua_public)?;
    let (cek, nonce) = keys(&secret, auth, ua_public, as_public, salt)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| fail())?,
    );
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| fail())?;
    let mut record = plaintext.to_vec();
    // The last (and only) record's padding delimiter.
    record.push(2);
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut record)
        .map_err(|_| fail())?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public.len() + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(u8::try_from(as_public.len()).map_err(|_| fail())?);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// Encrypt `plaintext` for one subscription, with a fresh key and salt.
pub fn encrypt(plaintext: &[u8], s: &PushSubscription) -> Result<Vec<u8>, String> {
    let fail = || "Push encryption failed.".to_owned();
    let ua_public = unb64(&s.p256dh)?;
    let auth = unb64(&s.auth)?;
    let private = PrivateKey::generate(&ECDH_P256).map_err(|_| fail())?;
    let mut salt = [0; 16];
    SystemRandom::new().fill(&mut salt).map_err(|_| fail())?;
    encrypt_with(plaintext, &ua_public, &auth, &private, &salt)
}

/// Decrypt a single-record `aes128gcm` body with the device's private key
/// (what the browser does; here for tests).
pub fn decrypt(body: &[u8], ua_private: &PrivateKey, auth: &[u8]) -> Result<Vec<u8>, String> {
    let fail = || "Push decryption failed.".to_owned();
    if body.len() < 21 {
        return Err(fail());
    }
    let salt = &body[..16];
    let id_len = usize::from(body[20]);
    let as_public = body.get(21..21 + id_len).ok_or_else(fail)?;
    let mut record = body.get(21 + id_len..).ok_or_else(fail)?.to_vec();
    let ua_public = ua_private.compute_public_key().map_err(|_| fail())?;
    let secret = ecdh(ua_private, as_public)?;
    let (cek, nonce) = keys(&secret, auth, ua_public.as_ref(), as_public, salt)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| fail())?,
    );
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| fail())?;
    let plain = key
        .open_in_place(nonce, aead::Aad::empty(), &mut record)
        .map_err(|_| fail())?;
    let end = plain.iter().rposition(|b| *b != 0).ok_or_else(fail)?;
    if plain[end] != 2 {
        return Err(fail());
    }
    Ok(plain[..end].to_vec())
}

/// What a device's push service said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// The subscription is gone (404 or 410): forget it.
    Gone,
    Failed(String),
}

/// POSTs one encrypted message; reqwest in the server, a stub in tests.
pub trait Post {
    fn post(&self, url: &str, headers: &[(&str, String)], body: Vec<u8>) -> Result<u16, String>;
}

/// The real push services, over HTTPS, 15 seconds, no redirects.
#[derive(Debug)]
pub struct Http(reqwest::blocking::Client);

impl Http {
    pub fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

impl Post for Http {
    fn post(&self, url: &str, headers: &[(&str, String)], body: Vec<u8>) -> Result<u16, String> {
        let mut request = self.0.post(url).body(body);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        request
            .send()
            .map(|r| r.status().as_u16())
            .map_err(|_| "The push service could not be reached.".to_owned())
    }
}

/// What the service worker shows: title, body, the page a tap opens, and a
/// tag so a newer message on the same subject replaces the older one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushMessage {
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
    /// `high` for alerts (shown at once), `normal` for the summary.
    pub urgent: bool,
}

impl PushMessage {
    fn payload(&self) -> Vec<u8> {
        let body: String = self.body.chars().take(1000).collect();
        json!({"title": self.title, "body": body, "url": self.url, "tag": self.tag})
            .to_string()
            .into_bytes()
    }
}

/// Signs, encrypts and posts messages for one server key.
#[derive(Debug)]
pub struct Pusher<T> {
    vapid: Vapid,
    transport: T,
}

impl<T: Post> Pusher<T> {
    pub fn new(vapid: Vapid, transport: T) -> Self {
        Self { vapid, transport }
    }

    pub fn public_key(&self) -> String {
        self.vapid.public_key()
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Deliver `message` to one device at `now` (Unix seconds).
    pub fn push(&self, s: &PushSubscription, message: &PushMessage, now: i64) -> Delivery {
        if !valid_endpoint(&s.endpoint) {
            return Delivery::Gone;
        }
        let attempt = || -> Result<u16, String> {
            let body = encrypt(&message.payload(), s)?;
            let headers = [
                (
                    "Authorization",
                    self.vapid.authorization(&s.endpoint, &s.origin, now)?,
                ),
                ("Content-Encoding", "aes128gcm".to_owned()),
                ("Content-Type", "application/octet-stream".to_owned()),
                ("TTL", TTL_SECONDS.to_string()),
                (
                    "Urgency",
                    if message.urgent { "high" } else { "normal" }.to_owned(),
                ),
            ];
            self.transport.post(&s.endpoint, &headers, body)
        };
        match attempt() {
            Ok(200..=202) => Delivery::Sent,
            Ok(404 | 410) => Delivery::Gone,
            Ok(status) => Delivery::Failed(format!("The push service answered {status}.")),
            Err(e) => Delivery::Failed(e),
        }
    }

    /// Deliver to every device; `gone` hears of each subscription to forget.
    /// The outcome is `delivered` when one device took it, `unavailable`
    /// without devices, `failed` otherwise.
    pub fn push_all(
        &self,
        subscriptions: &[PushSubscription],
        message: &PushMessage,
        now: i64,
        gone: &dyn Fn(&PushSubscription),
    ) -> Outcome {
        let mut outcome = Outcome::Unavailable;
        for s in subscriptions {
            match self.push(s, message, now) {
                Delivery::Sent => outcome = Outcome::Delivered,
                Delivery::Gone => gone(s),
                Delivery::Failed(_) if outcome != Outcome::Delivered => outcome = Outcome::Failed,
                Delivery::Failed(_) => {}
            }
        }
        outcome
    }
}

/// Goal and budget alerts as push notifications on every subscribed device.
pub struct PushAlerts<T, L, G> {
    pusher: Arc<Pusher<T>>,
    subscriptions: L,
    gone: G,
}

impl<T, L, G> PushAlerts<T, L, G>
where
    T: Post,
    L: Fn() -> Result<Vec<PushSubscription>, String>,
    G: Fn(&PushSubscription),
{
    pub fn new(pusher: Arc<Pusher<T>>, subscriptions: L, gone: G) -> Self {
        Self {
            pusher,
            subscriptions,
            gone,
        }
    }
}

impl<T, L, G> std::fmt::Debug for PushAlerts<T, L, G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushAlerts")
    }
}

impl<T, L, G> Notifier for PushAlerts<T, L, G>
where
    T: Post,
    L: Fn() -> Result<Vec<PushSubscription>, String>,
    G: Fn(&PushSubscription),
{
    fn send(&self, n: &Notification) -> Outcome {
        let Ok(subscriptions) = (self.subscriptions)() else {
            return Outcome::Unavailable;
        };
        let now = jiff::Timestamp::now();
        let (title, body) = finance_core::rafael::alert(
            n,
            finance_core::rafael::seed(&n.name, now.to_zoned(jiff::tz::TimeZone::UTC).date()),
        );
        let message = PushMessage {
            title,
            body,
            url: "/".into(),
            tag: format!("alert:{}", n.name),
            urgent: true,
        };
        let now = now.as_second();
        self.pusher
            .push_all(&subscriptions, &message, now, &self.gone)
    }
}

#[cfg(test)]
#[path = "push_tests.rs"]
mod tests;
