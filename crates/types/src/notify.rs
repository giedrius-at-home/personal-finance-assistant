//! The daily spending email's settings and status (Python
//! `SpendingEmailSettings`, configured there through `PFA_*` variables).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::report::Currency;

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
/// the secret store and never reaches the UI.
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
    /// Email each new goal or budget alert (80% used, over plan) as it
    /// happens: how the phone hears of them when the app runs as the server.
    pub alerts: bool,
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
            alerts: false,
        }
    }
}

/// Manage → Settings → Daily email.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailStatus {
    pub settings: EmailSettings,
    /// Whether a password is stored in the secret store.
    pub password_set: bool,
    /// When the last risk email and the last forecast digest went out (UTC).
    pub last_alert: Option<String>,
    pub last_digest: Option<String>,
    /// When the daily slot last completed (UTC).
    pub last_run: Option<String>,
}

/// One device's Web Push subscription, as the browser's `PushSubscription`
/// gives it (`endpoint`, `keys.p256dh`, `keys.auth`, both base64url), plus
/// the page origin it was made on (the VAPID `sub`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)]
    pub origin: String,
}

/// Manage → Settings → Phone notifications.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushStatus {
    /// The server's VAPID public key (base64url), for `pushManager.subscribe`;
    /// `None` where push is not offered (the Mac app).
    pub public_key: Option<String>,
    /// The push endpoints subscribed, by their service's host only.
    pub devices: Vec<String>,
}

/// One plan group's day: what went out today against what the rest of the
/// month allows per day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayGroup {
    pub group: String,
    pub spent: Decimal,
    pub allowance: Decimal,
}

/// Today in one currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTotal {
    pub currency: Currency,
    /// Spent today in groups with a plan limit.
    pub spent: Decimal,
    /// What those limits allow today: what is left of each, spread over the
    /// days left in the month (today included).
    pub allowance: Decimal,
    /// `spent - allowance` when positive.
    pub over_by: Decimal,
    /// The groups that went over today, most first.
    pub groups: Vec<DayGroup>,
}

/// A purchase today in a group the owner marks avoidable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvoidablePurchase {
    pub merchant: String,
    pub group: String,
    pub amount: Decimal,
    pub currency: Currency,
    /// Local `HH:MM`, when the card memo gave it.
    pub time: Option<String>,
    /// What could have been done instead ("cooking at home").
    pub instead: String,
}

/// The evening's overspend summary (Overview → Today, the phone, the email).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DaySummary {
    /// Local date.
    pub date: String,
    pub totals: Vec<DayTotal>,
    /// Largest first.
    pub avoidable: Vec<AvoidablePurchase>,
}

impl DaySummary {
    /// Whether any currency went over today's allowance.
    pub fn over(&self) -> bool {
        self.totals.iter().any(|t| t.over_by > Decimal::ZERO)
    }
}

/// A spending group and whether its purchases count as avoidable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvoidableGroup {
    pub group: String,
    pub avoidable: bool,
    /// Whether that is the default from the group's name (no tick saved).
    pub default: bool,
}
