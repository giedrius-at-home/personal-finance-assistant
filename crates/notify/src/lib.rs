//! Delivery by email: the alert notifier the server registers (goal and
//! budget alerts reach the phone as email), the SMTP sender (Python
//! `SmtpSpendingEmailSender`) and the local Qwen writer (Python
//! `QwenSpendingMessageWriter` over `OllamaClient`). The content and every
//! decision come from `finance_core::spending_email`; this crate only talks
//! to the mail server and to Ollama. Both block, so callers run them on
//! their own thread, never on the async runtime.

pub mod push;

use std::io::Read;
use std::time::Duration;

use finance_core::alerts::{Notification, Notifier, Outcome};
use finance_core::spending_email::{
    Copy, Message, Risk, Sender, Smtp, WRITER_SYSTEM, Writer, parse_writer_reply, writer_prompt,
};
use finance_secrets::Secret;
use finance_types::notify::{EmailSettings, SmtpSecurity};
use lettre::message::{Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use serde_json::{Value, json};

/// The Ollama the Python service used, on this Mac.
pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const OLLAMA_MODEL: &str = "qwen3:8b";
/// Python `MAX_OLLAMA_RESPONSE_BYTES`.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
/// Python's status codes worth one more try.
const RETRYABLE: [u16; 4] = [429, 502, 503, 504];

/// The email as the mail server receives it: from "Rafael", plain text with
/// an HTML alternative when there is one.
pub fn email(message: &Message, smtp: &Smtp) -> Result<lettre::Message, String> {
    let from: Mailbox = format!("Rafael <{}>", smtp.from)
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

/// The email an alert becomes: the notification's title as the subject,
/// its line as the body.
pub fn alert_email(n: &Notification) -> Message {
    Message {
        subject: n.title.clone(),
        body: format!(
            "{}\n\nOpen Finance to see the plan and what to do now.\n",
            n.body
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

/// Python `QwenSpendingMessageWriter`: the persona prompt, qualitative
/// signals only, JSON out, temperature 0, two attempts, 90 second read.
#[derive(Debug)]
pub struct Ollama {
    base_url: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl Ollama {
    pub fn local() -> Result<Self, String> {
        Self::new(OLLAMA_URL, OLLAMA_MODEL)
    }

    /// Only a loopback address, as Python's `normalize_ollama_base_url`
    /// allowed.
    pub fn new(base_url: &str, model: &str) -> Result<Self, String> {
        let url = reqwest::Url::parse(base_url).map_err(|_| "Invalid Ollama URL.".to_owned())?;
        let local = matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        );
        if !local || !url.username().is_empty() || url.password().is_some() {
            return Err("Ollama must run on this Mac.".into());
        }
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: model.trim().to_owned(),
            client,
        })
    }

    /// The request body Python's `OllamaClient.generate` posted.
    pub fn payload(&self, prompt: &str) -> Value {
        json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": {"temperature": 0},
            "think": false,
            "system": WRITER_SYSTEM,
        })
    }

    fn generate(&self, prompt: &str) -> Result<String, String> {
        let unavailable = || "The local model is not available.".to_owned();
        let url = format!("{}/api/generate", self.base_url);
        let mut last = unavailable();
        for _ in 0..2 {
            let response = match self.client.post(&url).json(&self.payload(prompt)).send() {
                Ok(r) => r,
                Err(_) => {
                    last = unavailable();
                    continue;
                }
            };
            let status = response.status().as_u16();
            if RETRYABLE.contains(&status) {
                last = unavailable();
                continue;
            }
            if status != 200 {
                return Err("The local model refused the request.".into());
            }
            let mut body = Vec::new();
            response
                .take(MAX_RESPONSE_BYTES + 1)
                .read_to_end(&mut body)
                .map_err(|_| unavailable())?;
            if body.len() as u64 > MAX_RESPONSE_BYTES {
                return Err("The local model answered too much.".into());
            }
            let document: Value =
                serde_json::from_slice(&body).map_err(|_| "Unreadable model answer.".to_owned())?;
            return document
                .get("response")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "Unreadable model answer.".to_owned());
        }
        Err(last)
    }
}

impl Writer for Ollama {
    fn write(&self, risk: &Risk) -> Result<Copy, String> {
        parse_writer_reply(&self.generate(&writer_prompt(risk))?)
    }
}

#[cfg(test)]
mod tests;
