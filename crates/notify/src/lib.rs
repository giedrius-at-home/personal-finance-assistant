//! Delivery by email: the alert notifier the server registers (goal and
//! budget alerts reach the phone as email), the SMTP sender (Python
//! `SmtpSpendingEmailSender`) and Gisel's writer on Mistral (Python
//! `QwenSpendingMessageWriter`). The content and every decision come from
//! `finance_core::spending_email`; this crate only talks to the mail server
//! and to Mistral. Both block, so callers run them on their own thread,
//! never on the async runtime.

pub mod push;
pub mod telegram;

use std::time::Duration;

use finance_core::alerts::{Notification, Notifier, Outcome};
use finance_core::brief::{BriefWriter, Speaker, parse_brief, system_prompt};
use finance_core::spending_email::{
    Copy, Message, Risk, Sender, Smtp, Writer, parse_writer_reply, writer_prompt, writer_system,
};
use finance_mistral::Mistral;
use finance_secrets::Secret;
use finance_types::notify::{BriefFacts, BriefSettings, EmailSettings, SmtpSecurity};
use lettre::message::{Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};

/// The email as the mail server receives it: from the persona, plain text with
/// an HTML alternative when there is one.
pub fn email(message: &Message, smtp: &Smtp) -> Result<lettre::Message, String> {
    let name: String = smtp
        .name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();
    let from: Mailbox = format!("{} <{}>", name.trim(), smtp.from)
        .parse()
        .map_err(|_| "The sender is not an email address.".to_owned())?;
    let to: Mailbox = smtp
        .to
        .parse()
        .map_err(|_| "The recipient is not an email address.".to_owned())?;
    let builder = lettre::Message::builder()
        .from(from)
        .to(to)
        .subject(message.subject.clone());
    match &message.html_body {
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(
            message.body.clone(),
            html.clone(),
        )),
        None => builder.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(message.body.clone()),
        ),
    }
    .map_err(|e| format!("The email could not be built: {e}"))
}

/// Sends through any lettre transport: SMTP in the app, a stub in tests.
#[derive(Debug)]
pub struct Mailer<T> {
    smtp: Smtp,
    transport: T,
}

impl<T> Mailer<T> {
    pub fn new(smtp: Smtp, transport: T) -> Self {
        Self { smtp, transport }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

impl Mailer<SmtpTransport> {
    /// The owner's SMTP server: TLS from the start (`ssl`, port 465) or
    /// upgraded with STARTTLS, signing in with the secret store password; 20
    /// second timeout as in Python.
    pub fn smtp(smtp: Smtp, password: &Secret) -> Result<Self, String> {
        let builder = match smtp.security {
            SmtpSecurity::Ssl => SmtpTransport::relay(&smtp.host),
            SmtpSecurity::Starttls => SmtpTransport::starttls_relay(&smtp.host),
        }
        .map_err(|_| "The SMTP host is not valid.".to_owned())?;
        let transport = builder
            .port(smtp.port)
            .credentials(Credentials::new(
                smtp.username.clone(),
                password.expose().to_owned(),
            ))
            .timeout(Some(Duration::from_secs(20)))
            .build();
        Ok(Self::new(smtp, transport))
    }
}

impl<T> Sender for Mailer<T>
where
    T: Transport,
    T::Error: std::fmt::Display,
{
    fn send(&self, message: &Message) -> Result<(), String> {
        let email = email(message, &self.smtp)?;
        self.transport
            .send(&email)
            .map(|_| ())
            .map_err(|e| format!("Sending failed: {e}"))
    }
}

/// The email an alert becomes, in Rafael's voice (as the daily email):
/// his title as the subject, the facts and his line as the body.
pub fn alert_email(n: &Notification) -> Message {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let (subject, body) =
        finance_core::rafael::alert(n, finance_core::rafael::seed(&n.name, today));
    Message {
        subject,
        body: format!(
            "{body}\n\nOpen your plan: {}\n",
            finance_core::spending_email::PLAN_LINK
        ),
        html_body: None,
    }
}

/// Goal and budget alerts by email, for the server, where there is no
/// desktop to show them; the phone shows the email as it arrives. The email
/// settings are read at every alert, so turning "Email alerts" on or off
/// takes effect at once, and `send` gets the mail server to use with the
/// email (the server reads the SMTP password from its secret store there).
///
/// Outcomes as the alert record keeps them: `disabled` while the owner has
/// the setting off, `unavailable` when the settings cannot be read or have no
/// recipient, `failed` when sending did not work, `delivered` when the mail
/// server took it.
pub struct EmailAlerts<L, S> {
    settings: L,
    send: S,
}

impl<L, S> EmailAlerts<L, S>
where
    L: Fn() -> Result<EmailSettings, String>,
    S: Fn(&Smtp, &Message) -> Result<(), String>,
{
    pub fn new(settings: L, send: S) -> Self {
        Self { settings, send }
    }
}

impl<L, S> std::fmt::Debug for EmailAlerts<L, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EmailAlerts")
    }
}

impl<L, S> Notifier for EmailAlerts<L, S>
where
    L: Fn() -> Result<EmailSettings, String>,
    S: Fn(&Smtp, &Message) -> Result<(), String>,
{
    fn send(&self, message: &Notification) -> Outcome {
        let Ok(settings) = (self.settings)() else {
            return Outcome::Unavailable;
        };
        if !settings.alerts {
            return Outcome::Disabled;
        }
        let Ok(settings) = finance_core::spending_email::validate(settings) else {
            return Outcome::Unavailable;
        };
        if settings.recipient.is_empty() {
            return Outcome::Unavailable;
        }
        match (self.send)(&Smtp::from_settings(&settings), &alert_email(message)) {
            Ok(()) => Outcome::Delivered,
            Err(_) => Outcome::Failed,
        }
    }
}

/// Every alert to each notifier (email and push on the server); the record
/// keeps the best outcome: `delivered` if any delivered, then `failed`,
/// `unavailable`, and `disabled` only when every channel is off.
pub struct Fanout(pub Vec<Box<dyn Notifier + Send + Sync>>);

impl std::fmt::Debug for Fanout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Fanout({})", self.0.len())
    }
}

impl Notifier for Fanout {
    fn send(&self, message: &Notification) -> Outcome {
        let rank = |o: Outcome| match o {
            Outcome::Delivered => 3,
            Outcome::Failed => 2,
            Outcome::Unavailable => 1,
            Outcome::Disabled => 0,
        };
        self.0
            .iter()
            .map(|n| n.send(message))
            .max_by_key(|o| rank(*o))
            .unwrap_or(Outcome::Unavailable)
    }
}

/// Python `QwenSpendingMessageWriter` on Mistral: the persona prompt,
/// qualitative signals only, JSON out, temperature 0. Without a key (or
/// when Mistral fails) the email keeps its fixed lines.
#[derive(Debug)]
pub struct MistralWriter {
    pub client: Option<Mistral>,
    pub model: String,
    /// The persona's name ("Gisel", "Rafael").
    pub name: String,
}

impl Writer for MistralWriter {
    fn write(&self, risk: &Risk) -> Result<Copy, String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "No Mistral key.".to_owned())?;
        let reply = client
            .chat_json(
                &self.model,
                Some(&writer_system(&self.name)),
                &writer_prompt(risk),
                0.0,
            )
            .map_err(|e| finance_mistral::message(&e))?;
        parse_writer_reply(&reply)
    }
}

/// The daily brief on Mistral: the script from the facts (two tries,
/// every number checked against them) and Voxtral's voice.
#[derive(Debug)]
pub struct MistralBrief {
    pub client: Option<Mistral>,
    pub model: String,
}

impl MistralBrief {
    fn client(&self) -> Result<&Mistral, String> {
        self.client
            .as_ref()
            .ok_or_else(|| "No Mistral key.".to_owned())
    }
}

impl BriefWriter for MistralBrief {
    fn write(
        &self,
        facts: &BriefFacts,
        settings: &BriefSettings,
    ) -> Result<(String, String), String> {
        let client = self.client()?;
        let prompt = serde_json::to_string(facts).map_err(|e| e.to_string())?;
        let system = system_prompt(settings);
        let mut last = String::new();
        for _ in 0..2 {
            let reply = client
                .chat_json(&self.model, Some(&system), &prompt, 0.7)
                .map_err(|e| finance_mistral::message(&e))?;
            match parse_brief(&reply, facts, settings.length) {
                Ok(brief) => return Ok(brief),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

impl Speaker for MistralBrief {
    fn speak(&self, script: &str, voice: &str, expressiveness: f64) -> Result<Vec<u8>, String> {
        self.client()?
            .speech(script, Some(voice), Some(expressiveness))
            .map_err(|e| finance_mistral::message(&e))
    }
}

#[cfg(test)]
mod tests;
