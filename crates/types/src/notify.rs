//! The daily spending email's settings and status (Python
//! `SpendingEmailSettings`, configured there through `PFA_*` variables).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    /// TLS from the first byte (port 465).
    #[default]
    Ssl,
    /// Plain connection upgraded with STARTTLS (port 587).
    Starttls,
}

/// `email_settings.payload_json`. The SMTP password is not here: it lives in
/// the Keychain and never reaches the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailSettings {
    /// `PFA_SPENDING_EMAIL_ENABLED` (off until the owner turns it on).
    pub enabled: bool,
    /// `PFA_SPENDING_EMAIL_TO`.
    pub recipient: String,
    /// `PFA_SPENDING_EMAIL_FROM`; empty sends from the username.
    pub sender: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    pub username: String,
    /// `PFA_SPENDING_EMAIL_HOUR`: the local hour after which the day's email
    /// may go out.
    pub hour: u8,
    /// `PFA_SPENDING_FORECAST_DIGEST`: on quiet days, send the forecast.
    pub daily_digest: bool,
    /// Let the local Qwen model (Ollama) write the opening lines; off, or when
    /// the model fails, fixed copy is used.
    pub ollama: bool,
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            recipient: String::new(),
            sender: String::new(),
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            smtp_security: SmtpSecurity::Ssl,
            username: String::new(),
            hour: 20,
            daily_digest: true,
            ollama: true,
        }
    }
}

/// Manage → Settings → Daily email.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailStatus {
    pub settings: EmailSettings,
    /// Whether a password is stored in the Keychain.
    pub password_set: bool,
    /// When the last risk email and the last forecast digest went out (UTC).
    pub last_alert: Option<String>,
    pub last_digest: Option<String>,
    /// When the daily slot last completed (UTC).
    pub last_run: Option<String>,
}
