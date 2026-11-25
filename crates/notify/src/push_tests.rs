use std::cell::RefCell;
use std::sync::Mutex;

use super::*;

fn key(b64url: &str) -> Vec<u8> {
    unb64(b64url).unwrap()
}

/// RFC 8291 Appendix A: the example message, keys and salt.
#[test]
fn encrypts_the_rfc_8291_example() {
    let as_private = PrivateKey::from_private_key(
        &ECDH_P256,
        &key("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"),
    )
    .unwrap();
    let ua_public = key(
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
    );
    let auth = key("BTBZMqHH6r4Tts7J_aSIgg");
    let salt: [u8; 16] = key("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
    let body = encrypt_with(
        b"When I grow up, I want to be a watermelon",
        &ua_public,
        &auth,
        &as_private,
        &salt,
    )
    .unwrap();
    assert_eq!(
        b64(&body),
        "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
    );
    let ua_private = PrivateKey::from_private_key(
        &ECDH_P256,
        &key("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"),
    )
    .unwrap();
    assert_eq!(
        decrypt(&body, &ua_private, &auth).unwrap(),
        b"When I grow up, I want to be a watermelon"
    );
}

/// A device: its private key and the subscription the browser would send.
fn device() -> (PrivateKey, PushSubscription) {
    let private = PrivateKey::generate(&ECDH_P256).unwrap();
    let public = private.compute_public_key().unwrap();
    let subscription = PushSubscription {
        endpoint: "https://web.push.apple.com/QGuQyavXutnMH".into(),
        p256dh: b64(public.as_ref()),
        auth: b64(&[7; 16]),
        origin: "https://finance.example.com".into(),
    };
    (private, subscription)
}

#[test]
fn a_fresh_message_decrypts_on_the_device() {
    let (private, s) = device();
    let body = encrypt(b"{\"title\":\"hi\"}", &s).unwrap();
    assert_eq!(&body[16..20], &4096_u32.to_be_bytes());
    assert_eq!(body[20], 65);
    assert_eq!(
        decrypt(&body, &private, &[7; 16]).unwrap(),
        b"{\"title\":\"hi\"}"
    );
    assert!(encrypt(&[b'x'; 4000], &s).is_err());
}

#[test]
fn the_vapid_header_is_a_signed_es256_token_for_the_push_origin() {
    let (vapid, stored) = Vapid::generate().unwrap();
    assert_eq!(
        Vapid::from_pkcs8(&stored).unwrap().public_key(),
        vapid.public_key()
    );
    assert_eq!(key(&vapid.public_key()).len(), 65);
    let header = vapid
        .authorization(
            "https://web.push.apple.com/QGuQyavXutnMH",
            "https://finance.example.com",
            1_000,
        )
        .unwrap();
    let rest = header.strip_prefix("vapid t=").unwrap();
    let (token, k) = rest.split_once(", k=").unwrap();
    assert_eq!(k, vapid.public_key());
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let claims: serde_json::Value = serde_json::from_slice(&key(parts[1])).unwrap();
    assert_eq!(claims["aud"], "https://web.push.apple.com");
    assert_eq!(claims["exp"], 1_000 + 12 * 3600);
    assert_eq!(claims["sub"], "https://finance.example.com");
    let public = aws_lc_rs::signature::UnparsedPublicKey::new(
        &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED,
        key(k),
    );
    public
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &key(parts[2]),
        )
        .unwrap();
}

#[test]
fn only_known_push_services_are_accepted() {
    assert!(valid_endpoint("https://web.push.apple.com/abc"));
    assert!(valid_endpoint("https://fcm.googleapis.com/fcm/send/abc"));
    assert!(valid_endpoint(
        "https://wns2-db5p.notify.windows.com/w/?token=1"
    ));
    assert!(!valid_endpoint("http://web.push.apple.com/abc"));
    assert!(!valid_endpoint(
        "https://web.push.apple.com.evil.example/abc"
    ));
    assert!(!valid_endpoint("https://127.0.0.1/abc"));
    assert!(!valid_endpoint("https://web.push.apple.com:8443/abc"));
    let (_, s) = device();
    assert!(check_subscription(&s).is_ok());
    let short = PushSubscription {
        auth: b64(&[1; 8]),
        ..s.clone()
    };
    assert!(check_subscription(&short).is_err());
    let no_origin = PushSubscription {
        origin: String::new(),
        ..s
    };
    assert!(check_subscription(&no_origin).is_err());
}

/// Records each request and answers with the next status.
/// One request: URL, headers, body.
type Request = (String, Vec<(String, String)>, Vec<u8>);

struct Stub {
    statuses: Mutex<Vec<u16>>,
    seen: Mutex<Vec<Request>>,
}

impl Stub {
    fn answering(statuses: &[u16]) -> Self {
        Self {
            statuses: Mutex::new(statuses.iter().rev().copied().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Post for Stub {
    fn post(&self, url: &str, headers: &[(&str, String)], body: Vec<u8>) -> Result<u16, String> {
        self.seen.lock().unwrap().push((
            url.to_owned(),
            headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            body,
        ));
        self.statuses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| "offline".to_owned())
    }
}

fn alert() -> Notification {
    Notification {
        title: "Personal finance: warning".into(),
        body: "Groceries: 80% of the plan is used".into(),
        severity: finance_types::report::reporting::AlertSeverity::HeadsUp,
        kind: finance_core::alerts::SubjectKind::Budget,
        name: "Groceries".into(),
    }
}

#[test]
fn an_alert_reaches_the_device_encrypted_with_the_vapid_headers() {
    let (private, s) = device();
    let pusher = Pusher::new(Vapid::generate().unwrap().0, Stub::answering(&[201]));
    let message = PushMessage {
        title: "T".into(),
        body: "B".into(),
        url: "/".into(),
        tag: "t".into(),
        urgent: true,
    };
    assert_eq!(pusher.push(&s, &message, 0), Delivery::Sent);
    let seen = pusher.transport().seen.lock().unwrap();
    let (url, headers, body) = &seen[0];
    assert_eq!(url, &s.endpoint);
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    assert!(header("Authorization").unwrap().starts_with("vapid t="));
    assert_eq!(header("Content-Encoding"), Some("aes128gcm"));
    assert_eq!(header("Urgency"), Some("high"));
    assert_eq!(header("TTL"), Some("86400"));
    let payload: serde_json::Value =
        serde_json::from_slice(&decrypt(body, &private, &[7; 16]).unwrap()).unwrap();
    assert_eq!(payload["title"], "T");
    assert_eq!(payload["url"], "/");
}

#[test]
fn outcomes_follow_the_devices_and_gone_ones_are_forgotten() {
    let (_, a) = device();
    let (_, b) = device();
    let forgotten = RefCell::new(Vec::new());
    let gone = |s: &PushSubscription| forgotten.borrow_mut().push(s.p256dh.clone());
    let notifier = |statuses: &[u16], subs: Vec<PushSubscription>| {
        PushAlerts::new(
            Arc::new(Pusher::new(
                Vapid::generate().unwrap().0,
                Stub::answering(statuses),
            )),
            move || Ok(subs.clone()),
            gone,
        )
    };
    // One device took it, the other is gone.
    assert_eq!(
        notifier(&[410, 201], vec![a.clone(), b.clone()]).send(&alert()),
        Outcome::Delivered
    );
    assert_eq!(
        forgotten.borrow().as_slice(),
        std::slice::from_ref(&a.p256dh)
    );
    assert_eq!(
        notifier(&[500], vec![a.clone()]).send(&alert()),
        Outcome::Failed
    );
    assert_eq!(notifier(&[], vec![a]).send(&alert()), Outcome::Failed);
    assert_eq!(
        notifier(&[], Vec::new()).send(&alert()),
        Outcome::Unavailable
    );
    let unreadable = PushAlerts::new(
        Arc::new(Pusher::new(
            Vapid::generate().unwrap().0,
            Stub::answering(&[]),
        )),
        || Err("no-data".to_owned()),
        |_: &PushSubscription| {},
    );
    assert_eq!(unreadable.send(&alert()), Outcome::Unavailable);
}
