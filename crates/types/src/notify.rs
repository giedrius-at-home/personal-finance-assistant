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
    /// Let Mistral write Gisel's opening lines (without figures); off, or
    /// when the model fails, fixed copy is used. Saved as `ollama` before.
    #[serde(alias = "ollama")]
    pub ai_writer: bool,
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
            ai_writer: true,
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

// ---------------------------------------------------------------------------
// Gisel's daily voice brief

/// Who talks: both are laid-back Spaniards; only the name and the default
/// voice differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    /// A woman from Spain; Mistral's Elena (Spanish).
    #[default]
    Gisel,
    /// A guy from Spain; Mistral's Jordi (Catalan).
    Rafael,
}

impl Persona {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gisel => "Gisel",
            Self::Rafael => "Rafael",
        }
    }

    /// The Mistral voice (slug) used unless the owner picks another.
    pub fn default_voice(self) -> &'static str {
        match self {
            Self::Gisel => "voice_483",
            Self::Rafael => "voice_3143",
        }
    }

    /// The default voice's name, for the picker.
    pub fn default_voice_name(self) -> &'static str {
        match self {
            Self::Gisel => "Elena",
            Self::Rafael => "Jordi",
        }
    }
}

/// How long the brief runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefLength {
    /// About 45 seconds.
    Short,
    #[default]
    Normal,
    Long,
}

impl BriefLength {
    /// The words the prompt asks for.
    pub fn words(self) -> (usize, usize) {
        match self {
            Self::Short => (45, 85),
            Self::Normal => (70, 140),
            Self::Long => (120, 200),
        }
    }
}

/// Expressiveness the speech starts at: 2.0, in tenths.
pub const DEFAULT_EXPRESSIVENESS: u8 = 20;

/// The range Voxtral takes, in tenths: 1.0 (calm) to 5.0 (most expressive).
pub const EXPRESSIVENESS_RANGE: std::ops::RangeInclusive<u8> = 10..=50;

/// Manage → Settings → Daily voice brief. Every field has a default, so
/// settings saved by older builds still load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BriefSettings {
    /// Off until the owner turns it on.
    pub enabled: bool,
    /// The local hour after which the day's brief goes out.
    pub hour: u8,
    /// Speak it (Mistral Voxtral); off sends the text only.
    pub voice: bool,
    pub persona: Persona,
    /// A Mistral voice id or slug; empty is the persona's own voice.
    pub voice_id: String,
    /// How lively the voice is, in tenths: 10 (calm) to 50 (most expressive).
    pub expressiveness: u8,
    pub length: BriefLength,
    /// Switch to the voice's mood version that fits the day (Paul, Oliver
    /// and Marie have them; Elena and Jordi do not).
    pub match_mood: bool,
    /// The owner's own words on how the persona talks (at most 300
    /// characters, no digits); added to the prompt.
    pub style: String,
    /// Also send the brief on Telegram, from the speaking persona's own bot.
    pub telegram: bool,
    /// The owner's Telegram chat id; empty until it is found from the first
    /// message they send a bot (in a private chat it is the same for both).
    pub telegram_chat: String,
    /// What the persona calls the owner; filled from their Telegram first
    /// name when the chat is found, editable. Empty: no name.
    pub owner_name: String,
    /// Send a GIF that fits the day after the Telegram brief (GIPHY).
    pub gif: bool,
}

impl Default for BriefSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hour: 8,
            voice: true,
            persona: Persona::default(),
            voice_id: String::new(),
            expressiveness: DEFAULT_EXPRESSIVENESS,
            length: BriefLength::default(),
            match_mood: true,
            style: String::new(),
            telegram: false,
            telegram_chat: String::new(),
            owner_name: String::new(),
            gif: true,
        }
    }
}

impl BriefSettings {
    /// The voice chosen: the owner's pick, else the persona's own.
    pub fn chosen_voice(&self) -> &str {
        Some(self.voice_id.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or(self.persona.default_voice())
    }

    pub fn expressiveness(&self) -> f64 {
        let r = EXPRESSIVENESS_RANGE;
        f64::from(self.expressiveness.clamp(*r.start(), *r.end())) / 10.0
    }
}

/// The emotions Voxtral TTS 3 takes inline, as `[emotion=warm]`.
pub const EMOTION_TAGS: [&str; 34] = [
    "amused",
    "angry",
    "anxious",
    "apologetic",
    "appreciative",
    "assertive",
    "bored",
    "calm",
    "confident",
    "confused",
    "critical",
    "curious",
    "determined",
    "disappointed",
    "disgusted",
    "excited",
    "frustrated",
    "grateful",
    "happy",
    "hopeful",
    "playful",
    "proud",
    "reflective",
    "resigned",
    "sad",
    "sarcastic",
    "skeptical",
    "somber",
    "surprised",
    "tense",
    "tired",
    "urgent",
    "warm",
    "wistful",
];

/// The speeds Voxtral TTS 3 takes inline, as `[pace=slow]`.
pub const PACE_TAGS: [&str; 5] = ["very-slow", "slow", "moderate", "fast", "very-fast"];

/// The sounds a brief may make, as `[chuckle]`.
pub const SOUND_TAGS: [&str; 7] = ["um", "uh", "laugh", "breath", "sigh", "chuckle", "gasp"];

/// Whether `inner` (what is between the brackets) is a tag Voxtral knows.
pub fn known_tag(inner: &str) -> bool {
    match inner.split_once('=') {
        Some(("emotion", v)) => EMOTION_TAGS.contains(&v),
        Some(("pace", v)) => PACE_TAGS.contains(&v),
        Some(_) => false,
        None => SOUND_TAGS.contains(&inner),
    }
}

/// The script as words to read: every `[...]` tag taken out and the spaces
/// tidied.
pub fn plain_script(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    let mut depth = 0usize;
    for c in script.chars() {
        match c {
            '[' => {
                depth += 1;
                out.push(' ');
            }
            ']' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
    [" ,", " .", " …", " ?", " !"]
        .iter()
        .fold(joined, |t, p| t.replace(p, &p[1..]))
}

/// `"Paul - Happy"` → `("Paul", Some("happy"))`; `"gb_oliver_sad"` →
/// `("gb_oliver", Some("sad"))` when `moods` names it. The base without a
/// mood otherwise.
pub fn split_mood<'a>(name: &'a str, moods: &[&str]) -> (&'a str, Option<String>) {
    if let Some((base, mood)) = name.rsplit_once(" - ") {
        return (base.trim(), Some(mood.trim().to_lowercase()));
    }
    if let Some((base, mood)) = name.rsplit_once('_')
        && moods.contains(&mood.to_lowercase().as_str())
    {
        return (base, Some(mood.to_lowercase()));
    }
    (name.trim(), None)
}

/// Every mood a voice version may carry.
pub const MOODS: [&str; 11] = [
    "neutral",
    "happy",
    "cheerful",
    "excited",
    "confident",
    "curious",
    "sad",
    "frustrated",
    "angry",
    "calm",
    "serious",
];

/// The mood versions of `voice` (by id or slug) in `voices`: empty when it
/// has none.
pub fn mood_versions<'a>(voice: &str, voices: &'a [VoiceOption]) -> Vec<(&'a VoiceOption, String)> {
    let Some(chosen) = voices.iter().find(|v| v.id == voice || v.slug == voice) else {
        return Vec::new();
    };
    let base_of = |v: &VoiceOption| {
        let (b, m) = split_mood(&v.name, &MOODS);
        if m.is_some() {
            return (b.to_owned(), m);
        }
        let (b, m) = split_mood(&v.slug, &MOODS);
        (b.to_owned(), m)
    };
    let (base, mood) = base_of(chosen);
    if mood.is_none() {
        return Vec::new();
    }
    voices
        .iter()
        .filter_map(|v| {
            let (b, m) = base_of(v);
            (b == base).then_some((v, m?))
        })
        .collect()
}

/// How much is left in the plan in one currency, as Gisel says it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefLeft {
    /// What the plan groups still allow this month ("€412").
    pub left: String,
    /// Spread over the days left ("€68").
    pub per_day: String,
    pub days_left: i64,
}

/// A plan group close to or over its limit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefGroup {
    pub group: String,
    pub used_percent: i64,
    /// What is left ("€20"; "€0" when over).
    pub left: String,
}

/// A bill due in the next week.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefBill {
    pub name: String,
    /// "today", "tomorrow" or the weekday ("Friday").
    pub due: String,
    pub amount: String,
}

/// One habit: the last seven days against the four weeks before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefHabit {
    pub group: String,
    pub last_week: String,
    pub usual_week: String,
    /// Positive: more than usual.
    pub change_percent: i64,
    /// "up", "down" or "steady".
    pub direction: String,
    /// Weeks in a row the spending moved this way (1 = just this week).
    pub weeks_in_a_row: i64,
}

/// Everything Gisel may talk about today. Amounts are already words to
/// say; she may use no number that is not here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BriefFacts {
    pub date: String,
    pub weekday: String,
    pub left: Vec<BriefLeft>,
    pub over_limit: Vec<BriefGroup>,
    pub near_limit: Vec<BriefGroup>,
    pub bills_soon: Vec<BriefBill>,
    /// What went out yesterday, per currency ("€34").
    pub spent_yesterday: Vec<String>,
    /// The groups whose week changed most, largest change first.
    pub habits: Vec<BriefHabit>,
    /// Avoidable spending as a whole (eating out, coffee, shopping...).
    pub avoidable: Option<BriefHabit>,
}

/// One day's brief, without the audio.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    /// Local date.
    pub date: String,
    /// UTC, RFC 3339.
    pub created_at: String,
    pub title: String,
    pub script: String,
    /// "mistral" or "fixed" (Mistral unavailable).
    pub source: String,
    pub has_audio: bool,
    #[serde(default)]
    pub facts: BriefFacts,
    /// The voice that spoke it (empty when there is no audio).
    #[serde(default)]
    pub voice_id: String,
    /// Who spoke: "Gisel" or "Rafael".
    #[serde(default)]
    pub persona: String,
    /// A short GIPHY search for the reaction GIF that fits the day
    /// ("slow clap"); empty for none.
    #[serde(default)]
    pub gif: String,
}

/// "Preview" in the settings card: a brief from the form as it stands,
/// neither saved nor sent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefPreview {
    pub brief: Brief,
    /// The MP3, base64.
    pub audio: Option<String>,
}

/// One of Mistral's voices (preset or the owner's own), for the picker.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceOption {
    pub id: String,
    /// Mistral's short name ("voice_483", "gb_oliver_sad"); either works as
    /// the voice id.
    #[serde(default)]
    pub slug: String,
    pub name: String,
    pub languages: Vec<String>,
    #[serde(default)]
    pub gender: Option<String>,
    /// A voice the owner cloned in Mistral Studio (not a preset).
    #[serde(default)]
    pub custom: bool,
}

/// The brief's settings card and player.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefStatus {
    pub settings: BriefSettings,
    pub latest: Option<Brief>,
    /// Empty when Mistral has no key or could not be reached.
    #[serde(default)]
    pub voices: Vec<VoiceOption>,
    /// Whether Gisel's and Rafael's Telegram bot tokens are stored (the
    /// tokens never reach the UI).
    #[serde(default)]
    pub telegram_gisel: bool,
    #[serde(default)]
    pub telegram_rafael: bool,
    /// Whether the GIPHY key is stored.
    #[serde(default)]
    pub giphy: bool,
}
