use finance_core::spending_email::{Copy, Risk};
use lettre::transport::stub::StubTransport;

use serde_json::Value;

use super::*;

fn smtp() -> Smtp {
    Smtp {
        host: "smtp.example.com".into(),
        port: 465,
        security: SmtpSecurity::Ssl,
        username: "me@example.com".into(),
        from: "me@example.com".into(),
        to: "you@example.com".into(),
        name: "Gisel".into(),
    }
}

#[test]
fn sends_from_the_persona_with_an_html_alternative() {
    let mailer = Mailer::new(smtp(), StubTransport::new_ok());
    let message = Message {
        subject: "Gisel: your spending forecast".into(),
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
    assert!(raw.contains("From: Gisel <me@example.com>"));
    assert!(raw.contains("To: you@example.com"));
    assert!(raw.contains("Subject: Gisel: your spending forecast"));
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

fn writer(url: &str) -> MistralWriter {
    let client = Mistral::local(
        Secret::new("k".into()),
        url,
        std::time::Duration::from_secs(5),
    )
    .unwrap()
    .attempts(1);
    MistralWriter {
        client: Some(client),
        model: "mistral-small-latest".into(),
        name: "Rafael".into(),
    }
}

#[test]
fn mistral_gets_the_persona_and_signals_only() {
    let (url, server) = finance_mistral::fake::serve(vec![(
        200,
        finance_mistral::fake::chat(
            r#"{"subject":"Gisel: check in","body":"Take a look at your plan today."}"#,
        ),
    )]);
    let copy = writer(&url).write(&Risk::default()).unwrap();
    assert_eq!(
        copy,
        Copy {
            subject: "Gisel: check in".into(),
            body: "Take a look at your plan today.".into()
        }
    );
    let request: Value = serde_json::from_str(&server.join().unwrap()[0].body).unwrap();
    assert_eq!(request["model"], "mistral-small-latest");
    assert_eq!(request["temperature"], 0.0);
    assert_eq!(
        request["messages"][0]["content"],
        finance_core::spending_email::writer_system("Rafael")
    );
    assert_eq!(
        request["messages"][1]["content"],
        writer_prompt(&Risk::default())
    );
}

#[test]
fn copy_with_figures_is_refused() {
    let (url, server) = finance_mistral::fake::serve(vec![(
        200,
        finance_mistral::fake::chat(r#"{"subject":"Spent 40","body":"Ouch."}"#),
    )]);
    assert!(writer(&url).write(&Risk::default()).is_err());
    server.join().unwrap();
}

#[test]
fn without_a_key_the_writer_fails_so_fixed_copy_stands_in() {
    let writer = MistralWriter {
        client: None,
        model: "mistral-small-latest".into(),
        name: "Gisel".into(),
    };
    assert!(writer.write(&Risk::default()).is_err());
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

#[test]
fn the_brief_retries_once_when_a_number_is_invented_then_speaks() {
    use finance_core::brief::{BriefWriter, Speaker};
    use finance_types::notify::{BriefFacts, BriefLeft, BriefSettings, Persona};
    let facts = BriefFacts {
        left: vec![BriefLeft {
            left: "€160".into(),
            per_day: "€27".into(),
            days_left: 6,
        }],
        ..BriefFacts::default()
    };
    let script = "Hola, Rafael here. You have €160 left for the next 6 days, about €27 a day. \
        Nothing is near its limit and no bills are due, so this is a calm one. Your habits look \
        steady, which is fine, but steady is not the same as better. Today, try one meal you \
        cook yourself; the best budget is the one you actually look at. Venga.";
    let bad = serde_json::json!({"title": "€999 left", "script": script}).to_string();
    let good = serde_json::json!({"title": "€160 left", "script": script, "gif": "Slow  clap"})
        .to_string();
    let audio = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"ID3");
    let (url, server) = finance_mistral::fake::serve(vec![
        (200, finance_mistral::fake::chat(&bad)),
        (200, finance_mistral::fake::chat(&good)),
        (200, serde_json::json!({ "audio_data": audio }).to_string()),
    ]);
    let brief = MistralBrief {
        client: Some(
            Mistral::local(
                Secret::new("k".into()),
                &url,
                std::time::Duration::from_secs(5),
            )
            .unwrap()
            .attempts(1),
        ),
        model: "mistral-small-latest".into(),
    };
    let settings = BriefSettings {
        persona: Persona::Rafael,
        style: "More jokes.".into(),
        ..BriefSettings::default()
    };
    let written = brief.write(&facts, &settings).unwrap();
    assert_eq!(written.title, "€160 left");
    assert_eq!(written.gif, "slow clap");
    assert_eq!(brief.speak(&written.script, "v1", 1.4).unwrap(), b"ID3");
    let seen = server.join().unwrap();
    let prompt: Value = serde_json::from_str(&seen[0].body).unwrap();
    let system = prompt["messages"][0]["content"].as_str().unwrap();
    assert!(system.starts_with("You are Rafael, from Spain"));
    assert!(system.contains("start with \"Hola, Rafael here.\""));
    assert!(system.contains("70 to 140 words"));
    assert!(system.ends_with("More jokes."));
    let sent: BriefFacts =
        serde_json::from_str(prompt["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(sent, facts);
    let speech: Value = serde_json::from_str(&seen[2].body).unwrap();
    assert_eq!(speech["voice_id"], "v1");
    assert_eq!(speech["expressiveness"], 1.4);
}

#[test]
fn the_sender_is_the_persona() {
    let smtp = Smtp {
        name: "Rafael".into(),
        ..smtp()
    };
    let mailer = Mailer::new(smtp, StubTransport::new_ok());
    let message = Message {
        subject: "Hi".into(),
        body: "Body".into(),
        html_body: None,
    };
    mailer.send(&message).unwrap();
    assert!(
        mailer.transport().messages()[0]
            .1
            .contains("From: Rafael <me@example.com>")
    );
}
