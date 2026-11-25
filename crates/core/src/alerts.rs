//! Goal and budget alerts: evaluation, the durable alert lifecycle and the
//! notification record (Python `alert_evaluation.py`, `domain/alerting.py`,
//! `domain/alerts.py`, `notifications/service.py` and the `process_alert` /
//! `record_notification` writes of `persistence/sqlite.py`).
//!
//! Delivery itself is not here: the app hands [`Service::evaluate_alerts`] a
//! [`Notifier`], and the outcome it reports is recorded exactly as the Python
//! dispatcher recorded it.

use finance_types::gateway::{OwnerSettings, SavingsGoal};
use finance_types::report::Currency;
use finance_types::report::reporting::{AlertSeverity, GoalEvaluation};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::goals::evaluate_goal;
use crate::load::parse_instant;
use crate::pyfmt::{decimal_str, isoformat_utc, json_canonical, micros};
use crate::reporting::py_div;
use crate::service::Service;
use crate::{Error, Result};

/// The alert thresholds and cooldowns of the owner's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertPolicy {
    pub version: String,
    pub heads_up_spent_ratio: Decimal,
    pub heads_up_pace_ratio: Decimal,
    pub heads_up_cooldown_seconds: i64,
    pub warning_cooldown_seconds: i64,
    pub critical_cooldown_seconds: i64,
}

impl AlertPolicy {
    /// The policy `runtime.py` built from the owner settings.
    pub fn from_settings(s: &OwnerSettings) -> Self {
        Self {
            version: s.policy_version.clone(),
            heads_up_spent_ratio: s.heads_up_spent_ratio,
            heads_up_pace_ratio: s.heads_up_pace_ratio,
            heads_up_cooldown_seconds: i64::from(s.heads_up_cooldown_hours) * 3_600,
            warning_cooldown_seconds: i64::from(s.warning_cooldown_hours) * 3_600,
            critical_cooldown_seconds: i64::from(s.critical_cooldown_hours) * 3_600,
        }
    }

    pub fn cooldown_seconds(&self, severity: AlertSeverity) -> i64 {
        match severity {
            AlertSeverity::None => 0,
            AlertSeverity::HeadsUp => self.heads_up_cooldown_seconds,
            AlertSeverity::Warning => self.warning_cooldown_seconds,
            AlertSeverity::Critical => self.critical_cooldown_seconds,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectKind {
    Goal,
    Budget,
}

impl SubjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Budget => "budget",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Dispatch,
    Escalated,
    Suppressed,
    Duplicate,
    Resolved,
    Inactive,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Escalated => "escalated",
            Self::Suppressed => "suppressed",
            Self::Duplicate => "duplicate",
            Self::Resolved => "resolved",
            Self::Inactive => "inactive",
        }
    }
}

/// A content-free alert candidate: the evidence travels only as its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    pub severity: AlertSeverity,
    pub reasons: Vec<String>,
    pub policy_version: String,
    pub evidence_json: String,
    pub input_hash: String,
    pub fingerprint: String,
    /// Whole microseconds, as Python's `datetime` held it.
    pub evaluated_at: Timestamp,
    /// What to show instead of the dispatcher's generic message.
    pub message: Option<Notification>,
    /// What the owner calls the subject: the goal's or plan line's name
    /// (the subject id when there is none).
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub candidate: Candidate,
    pub disposition: Disposition,
    pub notify: bool,
}

/// What a notification shows, and what it is about (for Rafael's voice,
/// `crate::rafael`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub severity: AlertSeverity,
    pub kind: SubjectKind,
    /// The goal's or plan line's name.
    pub name: String,
}

impl Notification {
    /// The dispatcher's message: severity in the title, the subject and its
    /// first reason in the body (at most 96 and 512 characters).
    pub fn for_candidate(c: &Candidate) -> Self {
        if let Some(message) = &c.message {
            return Self {
                severity: c.severity,
                kind: c.subject_kind,
                name: c.name.clone(),
                ..message.clone()
            };
        }
        let title = format!(
            "Personal finance: {}",
            c.severity.as_str().replace('_', " ")
        );
        let body = format!(
            "{}: {}",
            c.subject_id,
            c.reasons.first().map_or("", String::as_str)
        );
        Self {
            title: title.chars().take(96).collect(),
            body: body.chars().take(512).collect(),
            severity: c.severity,
            kind: c.subject_kind,
            name: c.name.clone(),
        }
    }
}

/// How a delivery attempt went (Python `NotificationOutcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    Disabled,
    Unavailable,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Disabled => "disabled",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

/// Delivers one notification; implemented by the app (native notifications)
/// and by tests.
pub trait Notifier {
    fn send(&self, message: &Notification) -> Outcome;
}

/// The notifier the owner turned off: nothing is shown, every attempt is
/// recorded as `disabled` (Python's `MacOSNotifier(enabled=False)`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Disabled;

impl Notifier for Disabled {
    fn send(&self, _: &Notification) -> Outcome {
        Outcome::Disabled
    }
}

/// Inputs for one category and one budget period (Python `BudgetWindow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetWindow {
    pub category: String,
    pub currency: Currency,
    pub budget: Decimal,
    pub spent: Decimal,
    pub available_cash: Decimal,
    pub upcoming_committed: Decimal,
    pub planned_savings: Decimal,
    pub minimum_buffer: Decimal,
    pub elapsed_days: i64,
    pub total_days: i64,
}

/// Python's `max(a, b)`: the first argument unless the second is larger.
fn py_max(a: Decimal, b: Decimal) -> Decimal {
    if b > a { b } else { a }
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn dec(value: Decimal) -> Value {
    Value::String(decimal_str(value))
}

fn currency_json(c: &Currency) -> Value {
    json!({"code": c.code, "minor_unit": c.minor_unit})
}

fn candidate(
    kind: SubjectKind,
    subject_id: &str,
    severity: AlertSeverity,
    reasons: Vec<String>,
    policy: &AlertPolicy,
    evaluated_at: Timestamp,
    evidence: &Value,
) -> Candidate {
    let evidence_json = json_canonical(evidence);
    let input_hash = sha256_hex(&evidence_json);
    let fingerprint = sha256_hex(&format!(
        "{}\0{subject_id}\0{}\0{}\0{input_hash}",
        kind.as_str(),
        severity.as_str(),
        policy.version
    ));
    Candidate {
        subject_kind: kind,
        subject_id: subject_id.to_owned(),
        severity,
        reasons,
        policy_version: policy.version.clone(),
        evidence_json,
        input_hash,
        fingerprint,
        evaluated_at: micros(evaluated_at),
        message: None,
        name: subject_id.to_owned(),
    }
}

fn goal_evidence(e: &GoalEvaluation) -> Value {
    json!({
        "goal_id": e.goal_id,
        "currency": currency_json(&e.currency),
        "completed": e.completed,
        "overdue": e.overdue,
        "on_track": e.on_track,
        "remaining": dec(e.remaining),
        "expected_saved": dec(e.expected_saved),
        "schedule_variance": dec(e.schedule_variance),
        "required_weekly_contribution": dec(e.required_weekly_contribution),
        "days_remaining": e.days_remaining,
        "projected_completion_date": e.projected_completion_date,
    })
}

/// Python `goal_alert_candidate`.
pub fn goal_candidate(
    goal: &SavingsGoal,
    as_of: jiff::civil::Date,
    evaluated_at: Timestamp,
    policy: &AlertPolicy,
) -> Candidate {
    let e = evaluate_goal(goal, as_of);
    let (severity, reason) = if e.completed || e.on_track {
        (
            AlertSeverity::None,
            if e.completed {
                "goal is completed"
            } else {
                "goal is on track"
            },
        )
    } else if e.overdue {
        (
            AlertSeverity::Critical,
            "incomplete goal is past its target date",
        )
    } else if -e.schedule_variance >= e.required_weekly_contribution {
        (
            AlertSeverity::Warning,
            "goal is behind by at least one required weekly contribution",
        )
    } else {
        (
            AlertSeverity::HeadsUp,
            "goal is behind its contribution schedule",
        )
    };
    let mut c = candidate(
        SubjectKind::Goal,
        &goal.goal_id,
        severity,
        vec![reason.to_owned()],
        policy,
        evaluated_at,
        &goal_evidence(&e),
    );
    c.name = goal.name.clone();
    c
}

/// Python `evaluate_budget` + `budget_alert_candidate`.
pub fn budget_candidate(
    w: &BudgetWindow,
    evaluated_at: Timestamp,
    policy: &AlertPolicy,
) -> Candidate {
    let budget = w.budget;
    let spent = w.spent;
    let elapsed_ratio = py_div(
        Decimal::from(w.elapsed_days.max(1)),
        Decimal::from(w.total_days),
    );
    let spent_ratio = py_div(spent, budget);
    let pace_ratio = py_div(spent_ratio, elapsed_ratio);
    let forecast = py_div(spent, elapsed_ratio);
    let remaining_budget = py_max(budget - spent, Decimal::ZERO);
    let safe_to_spend = py_max(
        w.available_cash - w.upcoming_committed - w.planned_savings - w.minimum_buffer,
        Decimal::ZERO,
    );
    let mut severity = AlertSeverity::None;
    let mut reasons = Vec::new();
    let protected = w.upcoming_committed + w.planned_savings + w.minimum_buffer;
    if w.available_cash <= protected {
        severity = AlertSeverity::Critical;
        reasons.push("available cash does not cover commitments, savings, and buffer".to_owned());
    }
    if spent >= budget {
        severity = AlertSeverity::Critical;
        reasons.push("category budget is exhausted".to_owned());
    } else if forecast > budget {
        severity = severity.max(AlertSeverity::Warning);
        reasons.push("forecast spend exceeds the category budget".to_owned());
    } else if spent_ratio >= policy.heads_up_spent_ratio || pace_ratio >= policy.heads_up_pace_ratio
    {
        severity = severity.max(AlertSeverity::HeadsUp);
        reasons.push("spending is ahead of the configured budget pace".to_owned());
    }
    let evidence = json!({
        "category": w.category,
        "currency": currency_json(&w.currency),
        "severity": severity.as_str(),
        "spent_ratio": dec(spent_ratio),
        "elapsed_ratio": dec(elapsed_ratio),
        "pace_ratio": dec(pace_ratio),
        "forecast_spend": dec(forecast),
        "remaining_budget": dec(remaining_budget),
        "safe_to_spend": dec(safe_to_spend),
        "reasons": reasons,
        "policy_version": policy.version,
    });
    if reasons.is_empty() {
        reasons.push("budget is within configured limits".to_owned());
    }
    candidate(
        SubjectKind::Budget,
        &w.category,
        severity,
        reasons,
        policy,
        evaluated_at,
        &evidence,
    )
}

/// Share of a budget line's plan that raises a heads-up.
pub const BUDGET_LINE_HEADS_UP: Decimal = Decimal::from_parts(8, 0, 0, false, 1);

/// One candidate per plan line of the running month's budgets: a heads-up
/// once [`BUDGET_LINE_HEADS_UP`] of the plan is spent, critical once it is
/// all spent. The evidence holds only the line, month and severity, so a
/// line notifies once per level per month, whatever is spent after.
pub fn budget_line_candidates(
    budgets: &[finance_types::report::budget::MonthlyBudget],
    current_month: &str,
    evaluated_at: Timestamp,
    policy: &AlertPolicy,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for budget in budgets.iter().filter(|b| b.plan.month == current_month) {
        let code = &budget.plan.currency.code;
        for row in budget.groups.iter().filter(|r| r.planned > Decimal::ZERO) {
            let ratio = row.spent / row.planned;
            let severity = if ratio >= Decimal::ONE {
                AlertSeverity::Critical
            } else if ratio >= BUDGET_LINE_HEADS_UP {
                AlertSeverity::HeadsUp
            } else {
                AlertSeverity::None
            };
            let percent = (ratio * Decimal::ONE_HUNDRED).round();
            let money = |v: Decimal| match code.as_str() {
                "EUR" => format!("€{:.2}", v),
                "GBP" => format!("£{:.2}", v),
                "USD" => format!("${:.2}", v),
                _ => format!("{:.2} {code}", v),
            };
            let left = row.planned - row.spent;
            let week = row.week.as_ref().map_or(String::new(), |w| {
                if w.left >= Decimal::ZERO {
                    format!(" {} left this week.", money(w.left))
                } else {
                    format!(" {} over this week.", money(-w.left))
                }
            });
            let reason = match severity {
                AlertSeverity::Critical => format!(
                    "{}: {} of {} spent, {} over plan.{week}",
                    row.name,
                    money(row.spent),
                    money(row.planned),
                    money(-left)
                ),
                AlertSeverity::None => format!("{}: within plan", row.name),
                _ => format!(
                    "{}: {percent}% of the plan used, {} left this month.{week}",
                    row.name,
                    money(left)
                ),
            };
            let subject = format!("line:{}:{code}:{}", budget.plan.month, row.id);
            let evidence = json!({
                "line": row.id,
                "month": budget.plan.month,
                "currency": code,
                "severity": severity.as_str(),
                "policy_version": policy.version,
            });
            let mut c = candidate(
                SubjectKind::Budget,
                &subject,
                severity,
                vec![reason.clone()],
                policy,
                evaluated_at,
                &evidence,
            );
            c.name = row.name.clone();
            c.message = Some(Notification {
                title: match severity {
                    AlertSeverity::Critical => format!("{} is over budget", row.name),
                    _ => format!("{} at {percent}% of budget", row.name),
                }
                .chars()
                .take(96)
                .collect(),
                body: reason.chars().take(512).collect(),
                severity,
                kind: SubjectKind::Budget,
                name: row.name.clone(),
            });
            out.push(c);
        }
    }
    out
}

fn parse_time(text: &str) -> Result<Timestamp> {
    parse_instant(text).ok_or_else(|| Error::Invalid(format!("alert time {text}")))
}

/// Python `process_alert`: deduplication, cooldown, resolution and
/// escalation, written before any delivery is attempted.
pub fn process(conn: &Connection, c: &Candidate, policy: &AlertPolicy) -> Result<Decision> {
    let now = c.evaluated_at;
    let kind = c.subject_kind.as_str();
    let current: Option<(bool, String, String, Option<String>)> = conn
        .query_row(
            "SELECT active, severity, last_fingerprint, cooldown_until \
             FROM alert_lifecycle WHERE subject_kind = ?1 AND subject_id = ?2",
            params![kind, c.subject_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let stored_cooldown = current
        .as_ref()
        .and_then(|row| row.3.as_deref())
        .filter(|s| !s.is_empty())
        .map(parse_time)
        .transpose()?;
    let (disposition, notify) = match &current {
        Some(row) if row.2 == c.fingerprint => (Disposition::Duplicate, false),
        _ if c.severity == AlertSeverity::None => {
            if current.as_ref().is_some_and(|row| row.0) {
                (Disposition::Resolved, false)
            } else {
                (Disposition::Inactive, false)
            }
        }
        None => (Disposition::Dispatch, true),
        Some(row) if !row.0 => (Disposition::Dispatch, true),
        Some(row) if AlertSeverity::parse(&row.1).is_none_or(|stored| c.severity > stored) => {
            (Disposition::Escalated, true)
        }
        Some(_) => {
            let notify = now >= stored_cooldown.unwrap_or(now);
            (
                if notify {
                    Disposition::Dispatch
                } else {
                    Disposition::Suppressed
                },
                notify,
            )
        }
    };
    let active = c.severity != AlertSeverity::None;
    let stored_fingerprint = match &current {
        Some(row) if disposition == Disposition::Suppressed => row.2.clone(),
        _ => c.fingerprint.clone(),
    };
    let cooldown = if notify {
        Some(now + SignedDuration::from_secs(policy.cooldown_seconds(c.severity)))
    } else {
        stored_cooldown
    };
    conn.execute(
        "INSERT INTO alert_lifecycle(subject_kind, subject_id, active, severity, \
         last_fingerprint, cooldown_until, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(subject_kind, subject_id) DO UPDATE SET active = excluded.active, \
         severity = excluded.severity, last_fingerprint = excluded.last_fingerprint, \
         cooldown_until = excluded.cooldown_until, updated_at = excluded.updated_at",
        params![
            kind,
            c.subject_id,
            i64::from(active),
            c.severity.as_str(),
            stored_fingerprint,
            cooldown.map(isoformat_utc),
            isoformat_utc(now),
        ],
    )?;
    let reasons = json_canonical(&Value::from(c.reasons.clone()));
    conn.execute(
        "INSERT INTO alert_events(subject_kind, subject_id, severity, input_hash, fingerprint, \
         policy_version, evidence_json, reasons_json, disposition, occurred_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            kind,
            c.subject_id,
            c.severity.as_str(),
            c.input_hash,
            c.fingerprint,
            c.policy_version,
            c.evidence_json,
            reasons,
            disposition.as_str(),
            isoformat_utc(now),
        ],
    )?;
    Ok(Decision {
        candidate: c.clone(),
        disposition,
        notify,
    })
}

/// Python `record_notification`: every attempt is kept; one that did not
/// deliver clears the fingerprint and retries after an hour.
pub fn record_notification(
    conn: &Connection,
    c: &Candidate,
    outcome: Outcome,
    attempted_at: Timestamp,
) -> Result<()> {
    let at = isoformat_utc(attempted_at);
    conn.execute(
        "INSERT INTO notification_attempts(subject_kind, subject_id, fingerprint, outcome, \
         attempted_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            c.subject_kind.as_str(),
            c.subject_id,
            c.fingerprint,
            outcome.as_str(),
            at
        ],
    )?;
    if outcome != Outcome::Delivered {
        let retry = isoformat_utc(micros(attempted_at) + SignedDuration::from_hours(1));
        conn.execute(
            "UPDATE alert_lifecycle SET last_fingerprint = '', cooldown_until = ?1, \
             updated_at = ?2 WHERE subject_kind = ?3 AND subject_id = ?4 \
             AND last_fingerprint = ?5",
            params![
                retry,
                at,
                c.subject_kind.as_str(),
                c.subject_id,
                c.fingerprint
            ],
        )?;
    }
    Ok(())
}

/// Every goal (in `goal_id` order), then every budget window, as
/// candidates for `now` in the owner's time zone.
pub fn candidates(
    goals: &[SavingsGoal],
    budgets: &[BudgetWindow],
    settings: &OwnerSettings,
    now: Timestamp,
) -> Result<Vec<Candidate>> {
    let zone = TimeZone::get(&settings.timezone)
        .map_err(|_| Error::Invalid(format!("unknown time zone {}", settings.timezone)))?;
    let policy = AlertPolicy::from_settings(settings);
    let as_of = now.to_zoned(zone).date();
    let mut goals: Vec<&SavingsGoal> = goals.iter().collect();
    goals.sort_by(|a, b| a.goal_id.cmp(&b.goal_id));
    Ok(goals
        .into_iter()
        .map(|g| goal_candidate(g, as_of, now, &policy))
        .chain(budgets.iter().map(|b| budget_candidate(b, now, &policy)))
        .collect())
}

/// Python `AlertDispatcher.handle` for each candidate: persist the decision,
/// deliver what is due, record how delivery went. Each candidate commits on
/// its own, as the Python state did.
pub fn dispatch(
    conn: &mut Connection,
    candidates: &[Candidate],
    policy: &AlertPolicy,
    notifier: &dyn Notifier,
) -> Result<Vec<Decision>> {
    let mut decisions = Vec::with_capacity(candidates.len());
    for c in candidates {
        let tx = conn.transaction()?;
        let decision = process(&tx, c, policy)?;
        tx.commit()?;
        if decision.notify {
            let outcome = notifier.send(&Notification::for_candidate(c));
            let tx = conn.transaction()?;
            record_notification(&tx, c, outcome, c.evaluated_at)?;
            tx.commit()?;
        }
        decisions.push(decision);
    }
    Ok(decisions)
}

impl Service {
    /// Evaluate every savings goal at `now` and deliver what is due through
    /// `notifier`, or record it as `disabled` when the owner turned desktop
    /// notifications off. Budget windows came from Firefly's budget limits,
    /// which the local ledger does not have, so there are none.
    pub fn evaluate_alerts(
        &self,
        now: Timestamp,
        notifier: &dyn Notifier,
    ) -> std::result::Result<Vec<Decision>, String> {
        let (goals, settings) = self.read_snapshot(|s| (s.goals.clone(), s.settings.clone()))?;
        let goals: Vec<SavingsGoal> = goals.into_iter().map(|g| g.goal).collect();
        let mut found = candidates(&goals, &[], &settings, now).map_err(|e| e.to_string())?;
        let policy = AlertPolicy::from_settings(&settings);
        // The running month's budget lines (80% and over); a dashboard that
        // cannot be built leaves the goal alerts to go out alone.
        if let Ok(dashboard) = self.dashboard() {
            let budgets = &dashboard.monthly_budgets;
            if let Some(month) = budgets.iter().map(|b| b.plan.month.clone()).min() {
                found.extend(budget_line_candidates(budgets, &month, now, &policy));
            }
        }
        let notifier: &dyn Notifier = if settings.notifications_enabled {
            notifier
        } else {
            &Disabled
        };
        self.with_connection(|conn| dispatch(conn, &found, &policy, notifier))
    }
}

#[cfg(test)]
mod tests;
