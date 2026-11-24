//! Trust in the owner's Cloudflare Access login, for the phone: requests that
//! come through the tunnel carry `Cf-Access-Jwt-Assertion`, a token signed
//! by the Access team's key. A request is the owner's when that signature
//! verifies, the token is for this application (its AUD tag), from this
//! team, in date, and names the owner's email. Nothing else in the request
//! is believed: anyone on the private network can send headers, but only
//! Cloudflare can sign.
//!
//! Configured by `FINANCE_ACCESS_TEAM` (`<name>.cloudflareaccess.com`),
//! `FINANCE_ACCESS_AUD` and `FINANCE_ACCESS_EMAIL`; all three or none.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents, UnparsedPublicKey};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

pub const HEADER: &str = "cf-access-jwt-assertion";
/// Unknown key ids trigger a fetch of the team's keys at most this often.
const REFETCH: Duration = Duration::from_secs(60);
/// Clocks may differ by this much.
const SKEW: i64 = 60;

/// One of the team's signing keys.
#[derive(Clone, Debug)]
pub enum Key {
    /// Modulus and exponent, as the team's JWKS gives them.
    Components { n: Vec<u8>, e: Vec<u8> },
    /// A DER `RSAPublicKey` (tests).
    Der(Vec<u8>),
}

impl Key {
    fn verifies(&self, message: &[u8], signature: &[u8]) -> bool {
        match self {
            Self::Components { n, e } => RsaPublicKeyComponents { n, e }
                .verify(&RSA_PKCS1_2048_8192_SHA256, message, signature)
                .is_ok(),
            Self::Der(der) => UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, der)
                .verify(message, signature)
                .is_ok(),
        }
    }
}

/// What a token must say to be the owner's.
#[derive(Clone, Debug)]
pub struct Expect {
    /// `https://<team>.cloudflareaccess.com`.
    pub issuer: String,
    pub aud: String,
    pub email: String,
}

fn part(text: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(text).ok()?).ok()
}

/// The key id a token says it was signed with (RS256 only).
fn key_id(token: &str) -> Option<String> {
    let header = part(token.split('.').next()?)?;
    (header.get("alg")?.as_str()? == "RS256")
        .then(|| header.get("kid")?.as_str().map(str::to_owned))?
}

/// `token` is the owner's, at `now` (Unix seconds). The reason when not.
pub fn verify(
    token: &str,
    keys: &[(String, Key)],
    expect: &Expect,
    now: i64,
) -> Result<(), &'static str> {
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("not a signed token");
    };
    let kid = key_id(token).ok_or("not an RS256 token")?;
    let (_, key) = keys
        .iter()
        .find(|(id, _)| *id == kid)
        .ok_or("unknown signing key")?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "bad signature encoding")?;
    // The signature covers the first two parts exactly as sent.
    let signed = &token.as_bytes()[..head.len() + 1 + body.len()];
    if !key.verifies(signed, &signature) {
        return Err("signature does not verify");
    }
    // Only now is the body believed.
    let claims = part(body).ok_or("unreadable claims")?;
    let for_app = match claims.get("aud") {
        Some(Value::String(aud)) => *aud == expect.aud,
        Some(Value::Array(list)) => list.iter().any(|a| a.as_str() == Some(&expect.aud)),
        _ => false,
    };
    if !for_app {
        return Err("for another application");
    }
    if claims.get("iss").and_then(Value::as_str) != Some(&expect.issuer) {
        return Err("from another team");
    }
    let exp = claims
        .get("exp")
        .and_then(Value::as_i64)
        .ok_or("no expiry")?;
    if now >= exp {
        return Err("expired");
    }
    if claims
        .get("nbf")
        .and_then(Value::as_i64)
        .is_some_and(|nbf| now + SKEW < nbf)
    {
        return Err("not valid yet");
    }
    let email = claims.get("email").and_then(Value::as_str).unwrap_or("");
    if email.is_empty() || !email.eq_ignore_ascii_case(&expect.email) {
        return Err("another person");
    }
    Ok(())
}

#[derive(Debug, Default)]
struct Keys {
    keys: Vec<(String, Key)>,
    fetched: Option<Instant>,
}

#[derive(Debug)]
pub struct Access {
    expect: Expect,
    certs_url: String,
    http: reqwest::blocking::Client,
    keys: Mutex<Keys>,
}

impl Access {
    /// From the environment; `None` when Access is not configured.
    pub fn from_env() -> Result<Option<Self>, String> {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let (team, aud, email) = match (var("FINANCE_ACCESS_TEAM"), var("FINANCE_ACCESS_AUD"), var("FINANCE_ACCESS_EMAIL")) {
            (None, None, None) => return Ok(None),
            (Some(team), Some(aud), Some(email)) => (team, aud, email),
            _ => return Err("set all of FINANCE_ACCESS_TEAM, FINANCE_ACCESS_AUD and FINANCE_ACCESS_EMAIL, or none".into()),
        };
        // Keys are fetched from this host, so it must be Cloudflare's.
        let name = team.strip_suffix(".cloudflareaccess.com").unwrap_or("");
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("FINANCE_ACCESS_TEAM must be <name>.cloudflareaccess.com".into());
        }
        if !email.contains('@') {
            return Err("FINANCE_ACCESS_EMAIL is not an email address".into());
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Some(Self {
            expect: Expect {
                issuer: format!("https://{team}"),
                aud,
                email,
            },
            certs_url: format!("https://{team}/cdn-cgi/access/certs"),
            http,
            keys: Mutex::new(Keys::default()),
        }))
    }

    fn fetch(&self) -> Option<Vec<(String, Key)>> {
        let body: Value = self.http.get(&self.certs_url).send().ok()?.json().ok()?;
        let keys = body.get("keys")?.as_array()?;
        Some(
            keys.iter()
                .filter_map(|k| {
                    let bytes = |name: &str| URL_SAFE_NO_PAD.decode(k.get(name)?.as_str()?).ok();
                    Some((
                        k.get("kid")?.as_str()?.to_owned(),
                        Key::Components {
                            n: bytes("n")?,
                            e: bytes("e")?,
                        },
                    ))
                })
                .collect(),
        )
    }

    /// `token` (the header's value) is the owner's Access login. A refusal's
    /// reason goes to the log, never the token.
    pub fn allows(&self, token: &str) -> bool {
        let Some(kid) = key_id(token) else {
            eprintln!("access login refused: not an RS256 token");
            return false;
        };
        let Ok(mut held) = self.keys.lock() else {
            return false;
        };
        let known = held.keys.iter().any(|(id, _)| *id == kid);
        if !known && held.fetched.is_none_or(|at| at.elapsed() >= REFETCH) {
            held.fetched = Some(Instant::now());
            if let Some(keys) = self.fetch() {
                held.keys = keys;
            }
        }
        match verify(
            token,
            &held.keys,
            &self.expect,
            jiff::Timestamp::now().as_second(),
        ) {
            Ok(()) => true,
            Err(reason) => {
                eprintln!("access login refused: {reason}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::rsa::{KeyPair, KeySize};
    use aws_lc_rs::signature::{KeyPair as _, RSA_PKCS1_SHA256};
    use serde_json::json;

    const NOW: i64 = 1_800_000_000;

    fn expect() -> Expect {
        Expect {
            issuer: "https://team.cloudflareaccess.com".into(),
            aud: "aud-of-this-app".into(),
            email: "owner@example.com".into(),
        }
    }

    fn token(key: &KeyPair, kid: &str, claims: &Value) -> String {
        let enc = |v: &Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let signed = format!(
            "{}.{}",
            enc(&json!({"alg": "RS256", "kid": kid, "typ": "JWT"})),
            enc(claims)
        );
        let mut signature = vec![0; key.public_modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signed.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    fn claims() -> Value {
        json!({"aud": ["aud-of-this-app"], "iss": "https://team.cloudflareaccess.com",
               "exp": NOW + 600, "nbf": NOW - 10, "email": "Owner@Example.com", "type": "app"})
    }

    #[test]
    fn only_the_owners_signed_login_for_this_app() {
        let key = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let other = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let keys = vec![(
            "k1".to_owned(),
            Key::Der(key.public_key().as_ref().to_vec()),
        )];
        let check = |t: &str| verify(t, &keys, &expect(), NOW);

        assert_eq!(check(&token(&key, "k1", &claims())), Ok(()));

        // Signed by someone else, under the known key id or another.
        assert_eq!(
            check(&token(&other, "k1", &claims())),
            Err("signature does not verify")
        );
        assert_eq!(
            check(&token(&key, "k2", &claims())),
            Err("unknown signing key")
        );

        // A valid signature over other claims.
        let with = |name: &str, value: Value| {
            let mut c = claims();
            c[name] = value;
            token(&key, "k1", &c)
        };
        assert_eq!(
            check(&with("aud", json!(["another-app"]))),
            Err("for another application")
        );
        assert_eq!(
            check(&with("iss", json!("https://evil.cloudflareaccess.com"))),
            Err("from another team")
        );
        assert_eq!(check(&with("exp", json!(NOW))), Err("expired"));
        assert_eq!(check(&with("nbf", json!(NOW + 3600))), Err("not valid yet"));
        assert_eq!(
            check(&with("email", json!("someone@example.com"))),
            Err("another person")
        );
        assert_eq!(check(&with("email", Value::Null)), Err("another person"));

        // Claims swapped after signing, an unsigned token, and junk.
        let good = token(&key, "k1", &claims());
        let mut parts: Vec<&str> = good.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(json!({"aud": ["aud-of-this-app"], "iss": "https://team.cloudflareaccess.com", "exp": NOW + 600, "email": "owner@example.com"}).to_string());
        parts[1] = &forged;
        assert_eq!(check(&parts.join(".")), Err("signature does not verify"));
        let none = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"none","kid":"k1"}"#),
            parts[1]
        );
        assert_eq!(check(&none), Err("not an RS256 token"));
        assert_eq!(check("not-a-token"), Err("not a signed token"));
    }

    /// The JWKS form (modulus and exponent), which is what production gets.
    #[test]
    fn verifies_with_jwks_components() {
        let key = KeyPair::generate(KeySize::Rsa2048).unwrap();
        // DER RSAPublicKey of a 2048-bit key with e = 65537:
        // 30 82 01 0a | 02 82 01 01 00 <n: 256 bytes> | 02 03 01 00 01
        let der = key.public_key().as_ref();
        assert_eq!(
            (&der[..9], &der[265..]),
            (
                &[0x30, 0x82, 1, 0x0a, 2, 0x82, 1, 1, 0][..],
                &[2, 3, 1, 0, 1][..]
            )
        );
        let components = Key::Components {
            n: der[9..265].to_vec(),
            e: vec![1, 0, 1],
        };
        let keys = vec![("k1".to_owned(), components)];
        assert_eq!(
            verify(&token(&key, "k1", &claims()), &keys, &expect(), NOW),
            Ok(())
        );
        let other = KeyPair::generate(KeySize::Rsa2048).unwrap();
        assert_eq!(
            verify(&token(&other, "k1", &claims()), &keys, &expect(), NOW),
            Err("signature does not verify")
        );
    }
}
