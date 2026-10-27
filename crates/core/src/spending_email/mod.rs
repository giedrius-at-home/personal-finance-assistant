//! The daily, risk-gated spending email (Python `spending_email.py`,
//! `send_forecast.py` and the `DailySchedule` / `claim_evaluation` parts of
//! `scheduling.py` and `persistence/sqlite.py`).
//!
//! Everything here is computation and database state. Writing the opening
//! lines (Ollama) and sending (SMTP) are the [`Writer`] and [`Sender`] the
//! app passes in; the `finance-notify` crate implements them.

pub mod html;

use std::str::FromStr;
use std::sync::LazyLock;

use finance_types::gateway::Direction;
use finance_types::notify::{EmailSettings, EmailStatus, SmtpSecurity};
use finance_types::report::Currency;
use finance_types::report::Dashboard;
use finance_types::report::outlook::{DailySpendingForecast, ThreeDaySignal};
use finance_types::report::trends::SpendingRisk;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::groups::is_recurring_debit;
use crate::load::parse_instant;
use crate::model::Kind;
use crate::pyfmt::{decimal_str, isoformat_utc};
use crate::reporting::py_div;
use crate::service::Service;
use crate::{Ctx, Error, Result, Snapshot};

/// The scheduler's job name in `evaluation_schedule`, as Python named it.
pub const JOB: &str = "daily-spending-email";
/// A claimed slot that never completed may be retried after this long.
const LEASE_SECONDS: i64 = 900;
/// Python skipped the email when the last refresh was older than 36 hours.
const STALE_SECONDS: i64 = 129_600;
/// The receipts compared local days in Amsterdam, whatever the settings said.
const RECEIPT_ZONE: &str = "Europe/Amsterdam";
pub const PLAN_LINK: &str = "https://finance.giedriusathome.com/finance?view=spending";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtRiskGroup {
    pub group: String,
    pub currency: Currency,
    pub spent: Decimal,
    pub expected: Decimal,
    pub upper: Decimal,
    pub limit: Decimal,
    pub daily_room: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentPurchase {
    pub date: String,
    pub merchant: String,
    pub category: String,
    pub group: String,
    pub amount: Decimal,
    pub currency: Currency,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRisk {
    pub currency: Currency,
    pub signal: ThreeDaySignal,
    pub expected_savings: Decimal,
    pub planned_savings: Decimal,
    #[serde(default)]
    pub expected_final: Option<Decimal>,
    #[serde(default)]
    pub planned_final: Option<Decimal>,
}

/// Only material risks; a good day has none (Python `SpendingEmailRisk`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Risk {
    pub windows: Vec<WindowRisk>,
    /// (currency code, amount).
    pub salary_at_risk: Vec<(String, Decimal)>,
    pub next_month_shortfall: Vec<(String, Decimal)>,
    pub groups: Vec<AtRiskGroup>,
    pub forecasts: Vec<DailySpendingForecast>,
    pub recent_purchases: Vec<RecentPurchase>,
}

/// One email (Python `SpendingEmailMessage`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub subject: String,
    pub body: String,
    pub html_body: Option<String>,
}

/// The opening lines the model writes (or the fixed copy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub subject: String,
    pub body: String,
}

impl Copy {
    /// What Python sent when the model failed.
    pub fn fallback() -> Self {
        Self {
            subject: "Rafael: your spending picked up 👀".into(),
            body: "Quick budget check: a slower few days could help protect your savings goal."
                .into(),
        }
    }
}

/// Writes the opening lines from qualitative signals only.
pub trait Writer {
    fn write(&self, risk: &Risk) -> std::result::Result<Copy, String>;
}

/// Sends one email.
pub trait Sender {
    fn send(&self, message: &Message) -> std::result::Result<(), String>;
}

fn first_of_next_month(month: Date) -> Date {
    month
        .first_of_month()
        .checked_add(1.month())
        .unwrap_or(month)
        .first_of_month()
}

fn chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// Python `spending_email_risk`: allocation and forecast risks, with up to
/// eight recent everyday purchases; `None` on a good day.
pub fn spending_risk(report: &Dashboard, snap: &Snapshot, now: &Zoned) -> Option<Risk> {
    let allocation = report.monthly_allocation.as_ref()?;
    let ctx = Ctx::new(snap, now);
    let local_day = ctx.today();
    let this_month = ctx.current_month();
    let next_month = first_of_next_month(this_month);
    let days_left = i64::from((next_month - local_day).get_days()).max(1);
    let salary_risk: Vec<(String, Decimal)> = allocation
        .currencies
        .iter()
        .filter(|row| row.salary_at_risk > Decimal::ZERO)
        .map(|row| (row.currency.code.clone(), row.salary_at_risk))
        .collect();
    let shortfalls: Vec<(String, Decimal)> = allocation
        .currencies
        .iter()
        .filter(|row| row.salary_received_on.is_some() && row.next_shortfall > Decimal::ZERO)
        .map(|row| (row.currency.code.clone(), row.next_shortfall))
        .collect();
    let limit_of = |currency: &Currency, group: &str| {
        // A later row wins, as in Python's dict comprehension.
        allocation
            .currencies
            .iter()
            .flat_map(|row| row.groups.iter().map(move |g| (&row.currency, g)))
            .filter(|(c, g)| *c == currency && g.group == group)
            .map(|(_, g)| g.limit)
            .next_back()
    };
    let pace_risk = |currency: &Currency, group: &str| {
        report
            .spending_nudges
            .iter()
            .any(|n| n.risk != SpendingRisk::OnTrack && &n.currency == currency && n.group == group)
    };
    let groups: Vec<AtRiskGroup> = report
        .spending_forecasts
        .iter()
        .filter_map(|f| {
            let limit = limit_of(&f.currency, &f.group)?;
            (f.expected > limit || pace_risk(&f.currency, &f.group)).then(|| {
                let room = limit - f.spent;
                let room = if room > Decimal::ZERO {
                    room
                } else {
                    Decimal::ZERO
                };
                AtRiskGroup {
                    group: f.group.clone(),
                    currency: f.currency.clone(),
                    spent: f.spent,
                    expected: f.expected,
                    upper: f.upper,
                    limit,
                    daily_room: py_div(room, Decimal::from(days_left)),
                }
            })
        })
        .collect();
    let windows: Vec<WindowRisk> = report
        .savings_outlooks
        .iter()
        .filter(|o| o.data_fresh)
        .map(|o| WindowRisk {
            currency: o.currency.clone(),
            signal: o.signal.clone(),
            expected_savings: o.expected_savings,
            planned_savings: o.planned_savings,
            expected_final: Some(o.expected_final),
            planned_final: Some(o.planned_final),
        })
        .collect();
    if salary_risk.is_empty()
        && shortfalls.is_empty()
        && groups.is_empty()
        && !windows.iter().any(|w| w.signal.alert)
    {
        return None;
    }
    let group_of = |category: &str| {
        snap.group_mappings
            .iter()
            .filter(|m| m.category == category)
            .filter_map(|m| m.group.as_deref().filter(|g| !g.is_empty()))
            .next_back()
    };
    let risky: Vec<&str> = groups.iter().map(|g| g.group.as_str()).collect();
    let mut txs: Vec<_> = snap.raw_transactions.iter().collect();
    txs.sort_by_key(|t| std::cmp::Reverse(t.occurred_at.timestamp()));
    let mut purchases = Vec::new();
    for t in txs {
        let category = t.category.as_deref().unwrap_or("Uncategorised");
        let category = if category.is_empty() {
            "Uncategorised"
        } else {
            category
        };
        let group = group_of(category);
        if t.kind != Kind::Outflow
            || ctx.resolver.month_of(t) != this_month
            || t.subscription_id.is_some()
            || is_recurring_debit(t.description.as_deref())
            || (!risky.is_empty() && !group.is_some_and(|g| risky.contains(&g)))
        {
            continue;
        }
        let merchant = t
            .description
            .as_deref()
            .filter(|d| !d.is_empty())
            .unwrap_or(category);
        purchases.push(RecentPurchase {
            date: t
                .occurred_at
                .with_time_zone(snap.zone.clone())
                .date()
                .to_string(),
            merchant: chars(merchant, 120),
            category: category.to_owned(),
            group: group.unwrap_or("Other").to_owned(),
            amount: t.amount,
            currency: t.currency.clone(),
        });
        if purchases.len() == 8 {
            break;
        }
    }
    Some(Risk {
        windows,
        salary_at_risk: salary_risk,
        next_month_shortfall: shortfalls,
        groups,
        forecasts: report.daily_forecasts.clone(),
        recent_purchases: purchases,
    })
}

/// How bad the day is: the window spending, the forecast overruns and the
/// salary at risk, added up.
pub fn severity(risk: Option<&Risk>) -> Decimal {
    let Some(risk) = risk else {
        return Decimal::ZERO;
    };
    risk.windows
        .iter()
        .map(|w| w.signal.spending)
        .chain(risk.groups.iter().map(|g| g.expected - g.limit))
        .chain(risk.salary_at_risk.iter().map(|(_, v)| *v))
        .fold(Decimal::ZERO, |a, b| a + b)
}

/// A sent email: when, and how bad the day was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub sent_at: Timestamp,
    pub severity: Option<Decimal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptKind {
    /// A risk email (Python `.spending-email.json`).
    Alert,
    /// A forecast digest (Python `.spending-digest.json`).
    Digest,
}

impl ReceiptKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Digest => "digest",
        }
    }
}

pub fn receipt(conn: &Connection, kind: ReceiptKind) -> Result<Option<Receipt>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT sent_at, severity FROM email_receipts WHERE kind = ?1",
            [kind.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(sent, severity)| {
        Some(Receipt {
            sent_at: parse_instant(&sent)?,
            severity: severity.and_then(|s| Decimal::from_str(&s).ok()),
        })
    }))
}

pub fn save_receipt(
    conn: &Connection,
    kind: ReceiptKind,
    sent_at: Timestamp,
    severity: Option<Decimal>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO email_receipts(kind, sent_at, severity) VALUES (?1, ?2, ?3) \
         ON CONFLICT(kind) DO UPDATE SET sent_at = excluded.sent_at, severity = excluded.severity",
        params![
            kind.as_str(),
            isoformat_utc(sent_at),
            severity.map(decimal_str)
        ],
    )?;
    Ok(())
}

fn amsterdam_day(t: Timestamp) -> Date {
    let zone = TimeZone::get(RECEIPT_ZONE).unwrap_or(TimeZone::UTC);
    t.to_zoned(zone).date()
}

/// Python `_sent_today`.
pub fn sent_today(receipt: Option<&Receipt>, now: Timestamp) -> bool {
    receipt.is_some_and(|r| amsterdam_day(r.sent_at) == amsterdam_day(now))
}

/// Python `should_send`: once per local day; again after a week or when the
/// day got materially worse (at least 50 more and half as bad again).
pub fn should_send(receipt: Option<&Receipt>, now: Timestamp, severity: Decimal) -> bool {
    let Some(r) = receipt else {
        return true;
    };
    let Some(previous) = r.severity else {
        return true;
    };
    if amsterdam_day(now) == amsterdam_day(r.sent_at) {
        return false;
    }
    now.duration_since(r.sent_at) >= SignedDuration::from_hours(24 * 7)
        || (severity - previous >= Decimal::from(50)
            && severity >= previous * Decimal::from_str("1.5").unwrap_or(Decimal::ONE))
}

fn weekday(day: &str, format: &str) -> String {
    Date::from_str(day).map_or_else(|_| day.to_owned(), |d| d.strftime(format).to_string())
}

/// Python `forecast_summary`.
pub fn forecast_summary(forecasts: &[DailySpendingForecast], sign_off: bool) -> String {
    let mut lines = Vec::new();
    for f in forecasts {
        let code = &f.currency.code;
        let s = decimal_str;
        lines.push(format!(
            "Next 7 days ({code}): habits {code} {} (likely {}–{}) plus planned {code} {}.",
            s(f.next_week_habit),
            s(f.next_week_lower),
            s(f.next_week_upper),
            s(f.next_week_planned)
        ));
        if let Some(tomorrow) = f.days.get(1).or_else(|| f.days.first()) {
            lines.push(format!(
                "Tomorrow ({}): habits {code} {}, planned {code} {}.",
                weekday(&tomorrow.day, "%a"),
                s(tomorrow.habit_expected),
                s(tomorrow.planned)
            ));
        }
        lines.push(format!(
            "Month end: habit spending {code} {} (likely {}–{}); still planned {code} {}; \
             expected income {code} {}.",
            s(f.month_end_habit),
            s(f.month_end_lower),
            s(f.month_end_upper),
            s(f.month_remaining_planned),
            s(f.month_remaining_income)
        ));
        if let Some(h) = heaviest(f) {
            lines.push(format!(
                "Heaviest habit day: {}, about {code} {}.",
                h.weekday,
                s(h.average)
            ));
        }
        for flow in f.upcoming.iter().take(5) {
            lines.push(format!(
                "Planned {}: {} {}{code} {}.",
                weekday(&flow.day, "%d %b"),
                chars(&flow.name, 60),
                if flow.direction == Direction::Inflow {
                    "+"
                } else {
                    ""
                },
                s(flow.amount)
            ));
        }
        lines.push(format!("Model: {}.", f.model));
    }
    if sign_off {
        lines.push(format!("— Rafael\nOpen your plan: {PLAN_LINK}"));
    }
    lines.join("\n")
}

/// Python `max(f.habits, key=lambda h: h.average)`: the first of the largest.
pub(crate) fn heaviest(
    f: &DailySpendingForecast,
) -> Option<&finance_types::report::outlook::WeekdayHabit> {
    f.habits.iter().fold(None, |best, h| match best {
        Some(b) if h.average <= b.average => Some(b),
        _ => Some(h),
    })
}

/// Python `factual_summary`: every figure, written by code.
pub fn factual_summary(risk: &Risk) -> String {
    let s = decimal_str;
    let mut lines = Vec::new();
    for w in &risk.windows {
        let g = &w.signal;
        let code = &w.currency.code;
        lines.push(format!(
            "{}–{}: {code} {}; previous three days: {code} {}.",
            g.start,
            g.end,
            s(g.spending),
            s(g.previous_spending)
        ));
        lines.push(format!(
            "Unplanned: {code} {}. Three-day planned allowance: {code} {}.",
            s(g.unplanned),
            s(g.allowance)
        ));
        lines.push(format!(
            "Three-month savings: expected {code} {}; following plan {code} {}.",
            s(w.expected_savings),
            s(w.planned_savings)
        ));
    }
    for g in risk.groups.iter().take(3) {
        lines.push(format!(
            "{}: forecast {} {}; limit {}.",
            g.group,
            g.currency.code,
            s(g.expected),
            s(g.limit)
        ));
    }
    for (code, amount) in &risk.salary_at_risk {
        lines.push(format!(
            "Next month's reserved salary at risk: {code} {}.",
            s(*amount)
        ));
    }
    for (code, amount) in &risk.next_month_shortfall {
        lines.push(format!(
            "Next month funding shortfall: {code} {}.",
            s(*amount)
        ));
    }
    if !risk.forecasts.is_empty() {
        lines.push(forecast_summary(&risk.forecasts, false));
    }
    lines.push(format!("— Rafael\nOpen your plan: {PLAN_LINK}"));
    lines.join("\n")
}

/// The persona prompt Python gave Qwen.
pub const WRITER_SYSTEM: &str = "You are Rafael, a kind Gen Z budget coach. Return JSON with subject \
and body only. Body: two short sentences, under sixty words. Suggest one budget check. No numbers, \
amounts, products, names of merchants, or invented purchases. Exact figures are added by code. \
Example: {\"subject\":\"Rafael: quick budget check 👀\",\"body\":\"Dude, your savings goal deserves \
some love. Check your remaining budget before the next optional purchase.\"}";

/// The model sees only qualitative signals; code adds every figure.
pub fn writer_prompt(risk: &Risk) -> String {
    let any = |f: fn(&ThreeDaySignal) -> bool| risk.windows.iter().any(|w| f(&w.signal));
    format!(
        "{{\"recent_spending_above_plan\":{},\"spending_accelerating\":{},\
         \"unusual_unplanned_spending\":{},\"budget_group_at_risk\":{},\
         \"salary_budget_at_risk\":{},\"next_month_shortfall\":{}}}",
        any(|s| s.excessive_speed),
        any(|s| s.accelerating),
        any(|s| s.unusual_unplanned),
        !risk.groups.is_empty(),
        !risk.salary_at_risk.is_empty(),
        !risk.next_month_shortfall.is_empty(),
    )
}

static DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d").expect("digit pattern"));

/// Python's check of the model's reply: exactly a subject (1–100
/// characters) and a body (1–1800), no digits anywhere, at most 60 words.
pub fn parse_writer_reply(text: &str) -> std::result::Result<Copy, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reply {
        subject: String,
        body: String,
        #[serde(default)]
        #[allow(dead_code)]
        html_body: Option<String>,
    }
    let invalid = || "Qwen returned an invalid spending email".to_owned();
    let reply: Reply = serde_json::from_str(text).map_err(|_| invalid())?;
    let (subject_len, body_len) = (reply.subject.chars().count(), reply.body.chars().count());
    if !(1..=100).contains(&subject_len)
        || !(1..=1800).contains(&body_len)
        || reply
            .html_body
            .as_ref()
            .is_some_and(|h| h.chars().count() > 40_000)
        || DIGIT.is_match(&format!("{}{}", reply.subject, reply.body))
        || reply.body.split_whitespace().count() > 60
    {
        return Err(invalid());
    }
    Ok(Copy {
        subject: reply.subject,
        body: reply.body,
    })
}

/// The risk email: the model's (or fixed) lines, then every figure.
pub fn alert_message(risk: &Risk, copy: &Copy, sample: bool) -> Message {
    let opening = chars(&copy.body, 500);
    Message {
        subject: copy.subject.clone(),
        body: chars(&format!("{opening}\n\n{}", factual_summary(risk)), 1800),
        html_body: Some(html::render(risk, &opening, sample, false)),
    }
}

pub const DIGEST_INTRO: &str = "Here is how the next days look if your usual habits continue.";

/// The quiet-day forecast digest.
pub fn digest_message(forecasts: &[DailySpendingForecast], sample: bool) -> Message {
    let risk = Risk {
        forecasts: forecasts.to_vec(),
        ..Risk::default()
    };
    Message {
        subject: "Rafael: your spending forecast 🔮".into(),
        body: chars(
            &format!("{DIGEST_INTRO}\n\n{}", forecast_summary(forecasts, true)),
            1800,
        ),
        html_body: Some(html::render(&risk, DIGEST_INTRO, sample, true)),
    }
}

// Scheduling ----------------------------------------------------------------

/// Python `DailySchedule.latest_due`: today's `hour:00` local slot, or
/// yesterday's before it.
pub fn latest_due(now: Timestamp, zone: &TimeZone, hour: u8) -> Timestamp {
    let local = now.to_zoned(zone.clone());
    let slot = |day: Date| {
        day.to_datetime(Time::constant(hour.min(23) as i8, 0, 0, 0))
            .to_zoned(zone.clone())
            .map(|z| z.timestamp())
            .unwrap_or(now)
    };
    let due = slot(local.date());
    if now < due {
        local.date().yesterday().map(slot).unwrap_or(due)
    } else {
        due
    }
}

/// Python `claim_evaluation`: take the slot due at `due_after` unless it ran
/// already or another run holds an unexpired lease.
pub fn claim(conn: &Connection, job: &str, now: Timestamp, due_after: Timestamp) -> Result<bool> {
    if now < due_after {
        return Ok(false);
    }
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT last_success_at, lease_until FROM evaluation_schedule WHERE job_name = ?1",
            [job],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((success, lease)) = row {
        let at = |s: Option<String>| {
            s.as_deref()
                .filter(|s| !s.is_empty())
                .and_then(parse_instant)
        };
        if at(success).is_some_and(|t| t >= due_after) {
            return Ok(false);
        }
        if at(lease).is_some_and(|t| t > now) {
            return Ok(false);
        }
    }
    let lease = isoformat_utc(now + SignedDuration::from_secs(LEASE_SECONDS));
    conn.execute(
        "INSERT INTO evaluation_schedule(job_name, lease_until) VALUES (?1, ?2) \
         ON CONFLICT(job_name) DO UPDATE SET lease_until = excluded.lease_until",
        params![job, lease],
    )?;
    Ok(true)
}

/// Python `complete_evaluation`.
pub fn complete(conn: &Connection, job: &str, at: Timestamp) -> Result<()> {
    conn.execute(
        "INSERT INTO evaluation_schedule(job_name, last_success_at, lease_until) \
         VALUES (?1, ?2, NULL) ON CONFLICT(job_name) DO UPDATE SET \
         last_success_at = excluded.last_success_at, lease_until = NULL",
        params![job, isoformat_utc(at)],
    )?;
    Ok(())
}

// Settings --------------------------------------------------------------------

/// Python `_is_email`.
pub fn is_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && value.chars().count() <= 254
        && !value
            .chars()
            .any(|c| c.is_whitespace() || "<>,;".contains(c))
        && !domain.contains('@')
}

fn bounded_token(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_whitespace)
}

/// Trim and check settings as Python checked its environment. An enabled
/// email needs every field; a disabled one may be left half filled.
pub fn validate(mut s: EmailSettings) -> Result<EmailSettings> {
    let invalid = |m: &str| Error::Rejected(m.to_owned());
    s.recipient = s.recipient.trim().to_owned();
    s.sender = s.sender.trim().to_owned();
    s.username = s.username.trim().to_owned();
    s.smtp_host = s.smtp_host.trim().to_owned();
    if s.hour > 23 {
        return Err(invalid("The email hour must be between 0 and 23."));
    }
    if s.smtp_port == 0 {
        return Err(invalid("The SMTP port must be between 1 and 65535."));
    }
    let complete = s.enabled || !s.recipient.is_empty() || !s.username.is_empty();
    if !complete {
        return Ok(s);
    }
    let sender = if s.sender.is_empty() {
        &s.username
    } else {
        &s.sender
    };
    if !is_email(&s.recipient) || !is_email(sender) {
        return Err(invalid("The recipient and sender must be email addresses."));
    }
    if !bounded_token(&s.username, 254) {
        return Err(invalid("The SMTP username is required."));
    }
    if !bounded_token(&s.smtp_host, 253) {
        return Err(invalid("The SMTP host is required."));
    }
    Ok(s)
}

pub fn load_settings(conn: &Connection) -> Result<EmailSettings> {
    let text: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM email_settings WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .or_else(|e| match e {
            // A database the app never wrote has no table yet.
            rusqlite::Error::SqliteFailure(..) => Ok(None),
            other => Err(other),
        })?;
    text.map_or_else(
        || Ok(EmailSettings::default()),
        |t| serde_json::from_str(&t).map_err(|e| Error::Invalid(format!("email settings: {e}"))),
    )
}

pub fn save_settings(conn: &Connection, s: &EmailSettings) -> Result<()> {
    let json = serde_json::to_string(s).map_err(|e| Error::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO email_settings(singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        [json],
    )?;
    Ok(())
}

/// The SMTP account the sender connects with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: String,
    pub from: String,
    pub to: String,
}

impl Smtp {
    pub fn from_settings(s: &EmailSettings) -> Self {
        Self {
            host: s.smtp_host.clone(),
            port: s.smtp_port,
            security: s.smtp_security,
            username: s.username.clone(),
            from: if s.sender.is_empty() {
                s.username.clone()
            } else {
                s.sender.clone()
            },
            to: s.recipient.clone(),
        }
    }
}

/// What a run sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The risk email, and how bad the day was.
    Alert {
        severity: Decimal,
    },
    /// The forecast digest of a quiet (or already flagged) day.
    Digest,
    Quiet,
}

/// The runner's decision: a risk email once per local day (again only after
/// a week or a material worsening); otherwise, with the digest on, one
/// forecast digest per local day and never on a day a risk email went out.
pub fn plan(
    risk: Option<&Risk>,
    has_forecasts: bool,
    settings: &EmailSettings,
    last_alert: Option<&Receipt>,
    last_digest: Option<&Receipt>,
    now: Timestamp,
) -> Plan {
    let severity = severity(risk);
    if risk.is_some() && should_send(last_alert, now, severity) {
        return Plan::Alert { severity };
    }
    if settings.daily_digest
        && has_forecasts
        && !sent_today(last_digest, now)
        && !sent_today(last_alert, now)
    {
        return Plan::Digest;
    }
    Plan::Quiet
}

/// The email for `plan`. The model writes only the opening lines; when it
/// is off or fails, the fixed copy stands in, as in Python.
pub fn compose(
    plan: Plan,
    risk: Option<&Risk>,
    forecasts: &[DailySpendingForecast],
    settings: &EmailSettings,
    writer: &dyn Writer,
    sample: bool,
) -> Option<Message> {
    match (plan, risk) {
        (Plan::Alert { .. }, Some(risk)) => {
            let copy = if settings.ollama {
                writer.write(risk).unwrap_or_else(|_| Copy::fallback())
            } else {
                Copy::fallback()
            };
            Some(alert_message(risk, &copy, sample))
        }
        (Plan::Digest, _) => Some(digest_message(forecasts, sample)),
        _ => None,
    }
}

// The daily run ---------------------------------------------------------------

/// What one scheduled check did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// Not enabled, not due yet, or already done for this slot.
    NotDue,
    /// The ledger was not refreshed in the last 36 hours.
    Stale,
    /// A good day: nothing to send (digest off, sent already, or no forecast).
    Quiet,
    AlertSent,
    DigestSent,
}

/// Whether the ledger was refreshed successfully in the last 36 hours: the
/// latest bank sync or import, or the old Firefly sync.
fn fresh(snap: &Snapshot, now: Timestamp) -> bool {
    let (firefly, _, failed) = snap.sync_health;
    let firefly = firefly.filter(|_| !failed);
    [firefly, snap.last_import]
        .into_iter()
        .flatten()
        .max()
        .is_some_and(|t| now.duration_since(t) <= SignedDuration::from_secs(STALE_SECONDS))
}

impl Service {
    pub fn email_settings(&self) -> std::result::Result<EmailSettings, String> {
        self.read_store(load_settings)
    }

    pub fn save_email_settings(
        &self,
        settings: EmailSettings,
    ) -> std::result::Result<EmailSettings, String> {
        let settings = validate(settings).map_err(|e| e.to_string())?;
        self.write(|w, _| save_settings(&w.tx, &settings))?;
        Ok(settings)
    }

    /// The settings card: settings, receipts and the last completed slot.
    pub fn email_status(&self, password_set: bool) -> std::result::Result<EmailStatus, String> {
        self.read_store(|conn| {
            let has = |table: &str| -> Result<bool> {
                Ok(conn
                    .query_row(
                        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                        [table],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            };
            let mut status = EmailStatus {
                settings: load_settings(conn)?,
                password_set,
                ..EmailStatus::default()
            };
            if has("email_receipts")? {
                status.last_alert =
                    receipt(conn, ReceiptKind::Alert)?.map(|r| isoformat_utc(r.sent_at));
                status.last_digest =
                    receipt(conn, ReceiptKind::Digest)?.map(|r| isoformat_utc(r.sent_at));
            }
            if has("evaluation_schedule")? {
                status.last_run = conn
                    .query_row(
                        "SELECT last_success_at FROM evaluation_schedule WHERE job_name = ?1",
                        [JOB],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
            }
            Ok(status)
        })
    }

    /// Python `run_scheduled` with `DailySpendingEmailRunner`: at most one
    /// run per daily slot; a failed run keeps its lease and is retried after
    /// it expires.
    pub fn run_spending_email(
        &self,
        now: Timestamp,
        writer: &dyn Writer,
        sender: &dyn Sender,
    ) -> std::result::Result<RunOutcome, String> {
        let settings = self.email_settings()?;
        if !settings.enabled {
            return Ok(RunOutcome::NotDue);
        }
        let zone = self.read_snapshot(|s| s.zone.clone())?;
        let due = latest_due(now, &zone, settings.hour);
        if !self.with_connection(|c| claim(c, JOB, now, due))? {
            return Ok(RunOutcome::NotDue);
        }
        let outcome = self.spending_email_once(now, &settings, writer, sender)?;
        self.with_connection(|c| complete(c, JOB, now))?;
        Ok(outcome)
    }

    /// One evaluation (Python `DailySpendingEmailRunner.__call__`).
    fn spending_email_once(
        &self,
        now: Timestamp,
        settings: &EmailSettings,
        writer: &dyn Writer,
        sender: &dyn Sender,
    ) -> std::result::Result<RunOutcome, String> {
        let built = self.read_forecast_at(now, |store, snap, zoned, model| {
            if !fresh(snap, now) {
                return Ok(None);
            }
            let zoned = zoned.clone();
            let report = crate::dashboard::dashboard(store, snap, &zoned, model)?;
            let risk = spending_risk(&report, snap, &zoned);
            Ok(Some((report, risk, !snap.accounts.is_empty())))
        })?;
        let Some((report, risk, has_accounts)) = built else {
            return Ok(RunOutcome::Stale);
        };
        // Python waited for its background forecast and then gave up; the
        // outlook here is computed in place, so missing means no history.
        if report.savings_outlooks.is_empty() && has_accounts {
            return Err("Savings forecast is still preparing".into());
        }
        let receipts = self.read_store(|c| {
            Ok((
                receipt(c, ReceiptKind::Alert)?,
                receipt(c, ReceiptKind::Digest)?,
            ))
        })?;
        let plan = plan(
            risk.as_ref(),
            !report.daily_forecasts.is_empty(),
            settings,
            receipts.0.as_ref(),
            receipts.1.as_ref(),
            now,
        );
        let Some(message) = compose(
            plan,
            risk.as_ref(),
            &report.daily_forecasts,
            settings,
            writer,
            false,
        ) else {
            return Ok(RunOutcome::Quiet);
        };
        sender.send(&message)?;
        self.with_connection(|c| match plan {
            Plan::Alert { severity } => save_receipt(c, ReceiptKind::Alert, now, Some(severity)),
            Plan::Digest => save_receipt(c, ReceiptKind::Digest, now, None),
            Plan::Quiet => Ok(()),
        })?;
        Ok(match plan {
            Plan::Alert { .. } => RunOutcome::AlertSent,
            Plan::Digest => RunOutcome::DigestSent,
            Plan::Quiet => RunOutcome::Quiet,
        })
    }

    /// The "Send test" button: today's email as it would go out now, marked
    /// as a test edition. It leaves the receipts and the schedule alone.
    pub fn spending_email_test(
        &self,
        now: Timestamp,
        settings: &EmailSettings,
        writer: &dyn Writer,
    ) -> std::result::Result<Message, String> {
        let (report, risk) = self.read_forecast_at(now, |store, snap, zoned, model| {
            let zoned = zoned.clone();
            let report = crate::dashboard::dashboard(store, snap, &zoned, model)?;
            let risk = spending_risk(&report, snap, &zoned);
            Ok((report, risk))
        })?;
        let plan = if risk.is_some() {
            Plan::Alert {
                severity: severity(risk.as_ref()),
            }
        } else if report.daily_forecasts.is_empty() {
            // Python `send_now`'s answer.
            return Err("Forecast is not ready yet (it needs four weeks of history).".into());
        } else {
            Plan::Digest
        };
        compose(
            plan,
            risk.as_ref(),
            &report.daily_forecasts,
            settings,
            writer,
            true,
        )
        .ok_or_else(|| "Nothing to send yet.".to_owned())
    }
}

#[cfg(test)]
mod tests;
