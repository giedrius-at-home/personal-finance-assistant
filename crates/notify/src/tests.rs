use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use finance_core::spending_email::{Copy, Risk};
use lettre::transport::stub::StubTransport;

use super::*;

fn smtp() -> Smtp {
    Smtp {
        host: "smtp.example.com".into(),
        port: 465,
        security: SmtpSecurity::Ssl,
        username: "me@example.com".into(),
        from: "me@example.com".into(),
        to: "you@example.com".into(),
    }
}

#[test]
fn sends_from_rafael_with_an_html_alternative() {
    let mailer = Mailer::new(smtp(), StubTransport::new_ok());
    let message = Message {
        subject: "Rafael: your spending forecast".into(),
        body: "Plain text".into(),
        html_body: Some("<p>Hi</p>".into()),
    };
    mailer.send(&message).unwrap();
    let sent = mailer.transport().messages();
    assert_eq!(sent.len(), 1);
    let (envelope, raw) = &sent[0];
    assert_eq!(
        envelope.from().map(ToString::to_string).as_deref(),
        Some("me@example.com")
    );
    assert_eq!(envelope.to()[0].to_string(), "you@example.com");
    assert!(raw.contains("From: Rafael <me@example.com>"));
    assert!(raw.contains("To: you@example.com"));
    assert!(raw.contains("Subject: Rafael: your spending forecast"));
    assert!(raw.contains("multipart/alternative"));
    assert!(raw.contains("Plain text"));
    assert!(raw.contains("<p>Hi</p>"));
}

#[test]
fn transport_errors_and_bad_addresses_are_reported() {
    let message = Message {
        subject: "S".into(),
        body: "B".into(),
        html_body: None,
    };
    let failing = Mailer::new(smtp(), StubTransport::new_error());
    assert!(failing.send(&message).is_err());
    let mut bad = smtp();
    bad.to = "nobody".into();
    let mailer = Mailer::new(bad, StubTransport::new_ok());
    assert!(mailer.send(&message).is_err());
    assert!(mailer.transport().messages().is_empty());
}

/// A fake Ollama: one canned answer on a loopback port. The handle returns
/// the request body it received.
fn fake_ollama(answer: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        std::io::Read::read_exact(&mut reader, &mut body).unwrap();
        let reply = json!({ "response": answer }).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{reply}",
            reply.len()
        )
        .unwrap();
        String::from_utf8(body).unwrap()
    });
    (url, handle)
}

#[test]
fn ollama_gets_the_persona_and_signals_only() {
    let (url, server) =
        fake_ollama(r#"{"subject":"Rafael: check in","body":"Take a look at your plan today."}"#);
    let writer = Ollama::new(&url, OLLAMA_MODEL).unwrap();
    let copy = writer.write(&Risk::default()).unwrap();
    assert_eq!(
        copy,
        Copy {
            subject: "Rafael: check in".into(),
            body: "Take a look at your plan today.".into()
        }
    );
    let request: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(request["model"], "qwen3:8b");
    assert_eq!(request["format"], "json");
    assert_eq!(request["stream"], false);
    assert_eq!(request["think"], false);
    assert_eq!(request["system"], WRITER_SYSTEM);
    assert_eq!(request["prompt"], writer_prompt(&Risk::default()));
}

#[test]
fn ollama_copy_with_figures_is_refused() {
    let (url, server) = fake_ollama(r#"{"subject":"Spent 40","body":"Ouch."}"#);
    let writer = Ollama::new(&url, OLLAMA_MODEL).unwrap();
    assert!(writer.write(&Risk::default()).is_err());
    server.join().unwrap();
}

#[test]
fn ollama_must_be_local() {
    assert!(Ollama::new("http://example.com:11434", OLLAMA_MODEL).is_err());
    assert!(Ollama::new("http://user:pw@127.0.0.1:11434", OLLAMA_MODEL).is_err());
    assert!(Ollama::local().is_ok());
}

/// Needs Ollama with qwen3:8b on this Mac: `cargo test -p finance-notify -- --ignored`.
#[test]
#[ignore = "needs the local Ollama model"]
fn local_qwen_writes_valid_copy() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../core/src/spending_email/python_golden.json"
    ))
    .unwrap();
    let risk: Risk = serde_json::from_value(golden["risk"].clone()).unwrap();
    let copy = Ollama::local().unwrap().write(&risk);
    println!("{copy:?}");
    assert!(copy.is_ok());
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

fn alert_settings(alerts: bool) -> EmailSettings {
    EmailSettings {
        alerts,
        recipient: "you@example.com".into(),
        username: "me@example.com".into(),
        ..EmailSettings::default()
    }
}

#[test]
fn alerts_are_emailed_to_the_recipient_when_turned_on() {
    let sent = std::sync::Mutex::new(Vec::new());
    let notifier = EmailAlerts::new(
        || Ok(alert_settings(true)),
        |smtp: &Smtp, m: &Message| {
            sent.lock().unwrap().push((smtp.clone(), m.clone()));
            Ok(())
        },
    );
    assert_eq!(notifier.send(&alert()), Outcome::Delivered);
    let sent = sent.into_inner().unwrap();
    assert_eq!(sent.len(), 1);
    let (smtp, message) = &sent[0];
    assert_eq!(smtp.to, "you@example.com");
    assert_eq!(smtp.from, "me@example.com");
    assert_eq!(smtp.host, "smtp.gmail.com");
    // In Rafael's voice: his title, the facts, his line and sign-off.
    assert_eq!(message.subject, "Rafael: Groceries needs a look 👀");
    assert!(message.body.starts_with("80% of the plan is used. "));
    assert!(message.body.contains("— Rafael\n\nOpen your plan: "));
    assert!(message.html_body.is_none());
}

#[test]
fn the_alert_email_goes_through_the_mailer() {
    let mailer = Mailer::new(smtp(), StubTransport::new_ok());
    let notifier = EmailAlerts::new(
        || Ok(alert_settings(true)),
        |_: &Smtp, m: &Message| mailer.send(m),
    );
    assert_eq!(notifier.send(&alert()), Outcome::Delivered);
    let sent = mailer.transport().messages();
    assert_eq!(sent.len(), 1);
    // The emoji is encoded (RFC 2047); the words stay readable.
    assert!(
        sent[0]
            .1
            .contains("Subject: Rafael: Groceries needs a look =?utf-8?")
    );
    assert!(sent[0].1.contains("To: you@example.com"));
}

#[test]
fn a_send_error_is_recorded_as_failed() {
    let failing = Mailer::new(smtp(), StubTransport::new_error());
    let notifier = EmailAlerts::new(
        || Ok(alert_settings(true)),
        |_: &Smtp, m: &Message| failing.send(m),
    );
    assert_eq!(notifier.send(&alert()), Outcome::Failed);
    let no_password = EmailAlerts::new(
        || Ok(alert_settings(true)),
        |_: &Smtp, _: &Message| Err("Save the SMTP password first.".to_owned()),
    );
    assert_eq!(no_password.send(&alert()), Outcome::Failed);
}

#[test]
fn nothing_is_sent_while_off_or_without_a_recipient() {
    let never = |_: &Smtp, _: &Message| -> Result<(), String> {
        panic!("nothing should be sent");
    };
    let off = EmailAlerts::new(|| Ok(alert_settings(false)), never);
    assert_eq!(off.send(&alert()), Outcome::Disabled);
    let unreadable = EmailAlerts::new(|| Err("no-data".to_owned()), never);
    assert_eq!(unreadable.send(&alert()), Outcome::Unavailable);
    let nobody = EmailAlerts::new(
        || {
            Ok(EmailSettings {
                recipient: String::new(),
                ..alert_settings(true)
            })
        },
        never,
    );
    assert_eq!(nobody.send(&alert()), Outcome::Unavailable);
}

struct Fixed(Outcome);

impl Notifier for Fixed {
    fn send(&self, _: &Notification) -> Outcome {
        self.0
    }
}

#[test]
fn fanout_keeps_the_best_outcome() {
    let fan = |outcomes: &[Outcome]| {
        Fanout(
            outcomes
                .iter()
                .map(|o| Box::new(Fixed(*o)) as Box<dyn Notifier + Send + Sync>)
                .collect(),
        )
        .send(&alert())
    };
    use Outcome::*;
    assert_eq!(fan(&[Disabled, Delivered]), Delivered);
    assert_eq!(fan(&[Failed, Unavailable]), Failed);
    assert_eq!(fan(&[Disabled, Unavailable]), Unavailable);
    assert_eq!(fan(&[Disabled, Disabled]), Disabled);
    assert_eq!(fan(&[]), Unavailable);
}
