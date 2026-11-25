//! Rafael, the voice of every message the app sends: the kind Gen Z budget
//! coach who signs the daily email (`spending_email::WRITER_SYSTEM`). The
//! owner may pick Gisel instead (Manage → Settings → Daily voice brief);
//! [`set_persona`] makes every title and sign-off here carry that name. The
//! facts stay the app's own (every figure comes from the alert or the
//! summary); Rafael adds the title, one coaching line and the sign-off.
//!
//! The lines rotate by day and subject (`seed`), so the same alert does not
//! read the same every time, and tests stay deterministic.

use finance_types::notify::DaySummary;
use finance_types::report::Currency;
use finance_types::report::reporting::AlertSeverity;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::alerts::{Notification, SubjectKind};

pub const SIGN_OFF: &str = "— Rafael";

/// The name every message here is signed with; "Rafael" until the app sets
/// the owner's persona.
static PERSONA: std::sync::RwLock<&'static str> = std::sync::RwLock::new("Rafael");

/// Sign every message from here on as `name` ("Gisel" or "Rafael").
pub fn set_persona(name: &'static str) {
    if let Ok(mut p) = PERSONA.write() {
        *p = name;
    }
}

/// The name messages are signed with now.
pub fn persona() -> &'static str {
    PERSONA.read().map_or("Rafael", |p| *p)
}

/// `(title, body)` in the persona's name.
fn signed(message: (String, String)) -> (String, String) {
    signed_as(persona(), message)
}

fn signed_as(name: &str, (title, body): (String, String)) -> (String, String) {
    if name == "Rafael" {
        (title, body)
    } else {
        (title.replace("Rafael", name), body.replace("Rafael", name))
    }
}

const HEADS_UP: [&str; 4] = [
    "Easy does it for the rest of the month, you've got this.",
    "One slower week and this lands right on plan.",
    "Quick check before the next optional buy? Future you says thanks.",
    "Still room, just less of it. Spend it on what matters most.",
];
const WARNING: [&str; 3] = [
    "Time for a little pause on this one.",
    "Let's slow this one down before it tips over.",
    "Hold off on the extras here for a few days.",
];
const CRITICAL: [&str; 4] = [
    "No stress. Hit pause here until the month resets.",
    "It happens. Borrow from a line with room, or let this one rest till next month.",
    "This one's done for the month. Small reset, big win.",
    "Over is a signal, not a verdict. Pause the extras here and you're good.",
];
const GOAL: [&str; 3] = [
    "Even a small top-up this week keeps it moving.",
    "A tiny transfer today goes a long way. 💛",
    "Future you is cheering for this one. Nudge it forward when you can.",
];
const DAY_TITLES: [&str; 3] = [
    "Rafael: today ran {over} over 👀",
    "Rafael: {over} over plan today, tomorrow's a reset ✨",
    "Rafael: today got a bit spendy ({over} over) 👀",
];
const DAY_CLOSERS: [&str; 4] = [
    "Tomorrow's a fresh start.",
    "Small swaps, big wins.",
    "No guilt, just a plan for tomorrow.",
    "You've got this.",
];

/// A stable number for `subject` on `day`: the same alert gets the same
/// words all day, and different ones on another day.
pub fn seed(subject: &str, day: Date) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in subject.bytes().chain(day.to_string().bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    h
}

fn pick<'a>(lines: &[&'a str], seed: u64) -> &'a str {
    lines[(seed % lines.len() as u64) as usize]
}

/// The factual line without the dispatcher's `subject:` prefix, which
/// Rafael's title already names.
fn facts(n: &Notification) -> String {
    let body = n.body.trim();
    // The dispatcher's generic body is always `<subject id>: <reason>`.
    let generic = n.title.starts_with("Personal finance:");
    match body.split_once(": ") {
        Some((subject, rest)) if generic || subject == n.name => rest.trim().to_owned(),
        _ => body.to_owned(),
    }
}

/// Goal and category reasons are lower-case fragments ("goal is behind
/// ..."): capitalised and closed with a full stop.
fn sentence(text: &str) -> String {
    let mut c = text.chars();
    let first: String = c
        .next()
        .map(|f| f.to_uppercase().collect())
        .unwrap_or_default();
    let mut out = format!("{first}{}", c.as_str());
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

fn emoji(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Critical => "🚨",
        AlertSeverity::Warning => "✋",
        _ => "👀",
    }
}

/// An alert in Rafael's voice: `(title, body)`.
pub fn alert(n: &Notification, seed: u64) -> (String, String) {
    signed(alert_text(n, seed))
}

fn alert_text(n: &Notification, seed: u64) -> (String, String) {
    let generic = n.title.starts_with("Personal finance:");
    let title = match n.kind {
        SubjectKind::Goal => match n.severity {
            AlertSeverity::Critical => format!("Rafael: {} is past its date 🚨", n.name),
            _ => format!("Rafael: {} needs a little love 💛", n.name),
        },
        SubjectKind::Budget if generic => {
            format!("Rafael: {} needs a look {}", n.name, emoji(n.severity))
        }
        SubjectKind::Budget => format!("Rafael: {} {}", n.title, emoji(n.severity)),
    };
    let coach = match (n.kind, n.severity) {
        (SubjectKind::Goal, _) => pick(&GOAL, seed),
        (_, AlertSeverity::Critical) => pick(&CRITICAL, seed),
        (_, AlertSeverity::Warning) => pick(&WARNING, seed),
        _ => pick(&HEADS_UP, seed),
    };
    let facts = sentence(&facts(n));
    (
        title.chars().take(96).collect(),
        format!("{facts} {coach} {SIGN_OFF}"),
    )
}

fn money(amount: Decimal, currency: &Currency) -> String {
    let value = amount.round_dp(2);
    match currency.code.as_str() {
        "EUR" => format!("€{value:.2}"),
        "USD" => format!("${value:.2}"),
        "GBP" => format!("£{value:.2}"),
        code => format!("{value:.2} {code}"),
    }
}

/// The evening overspend summary in Rafael's voice: `(title, body)`.
pub fn day_summary(s: &DaySummary, seed: u64) -> (String, String) {
    signed(day_summary_text(s, seed))
}

fn day_summary_text(s: &DaySummary, seed: u64) -> (String, String) {
    let over: Vec<String> = s
        .totals
        .iter()
        .filter(|t| t.over_by > Decimal::ZERO)
        .map(|t| money(t.over_by, &t.currency))
        .collect();
    let title = pick(&DAY_TITLES, seed).replace("{over}", &over.join(" and "));
    let mut lines: Vec<String> = s
        .avoidable
        .iter()
        .take(2)
        .map(|p| {
            format!(
                "{} {}: next time, try {}.",
                p.merchant,
                money(p.amount, &p.currency),
                p.instead
            )
        })
        .collect();
    if lines.is_empty() {
        let groups: Vec<&str> = s
            .totals
            .iter()
            .flat_map(|t| t.groups.iter().map(|g| g.group.as_str()))
            .take(3)
            .collect();
        lines.push(format!("Most of it went to {}.", groups.join(", ")));
    } else if s.avoidable.len() > 2 {
        lines.push(format!("Plus {} more in the app.", s.avoidable.len() - 2));
    }
    lines.push(pick(&DAY_CLOSERS, seed.rotate_left(7)).to_owned());
    lines.push(SIGN_OFF.to_owned());
    (title, lines.join(" "))
}

/// The "Send a test" push.
pub fn test_message() -> (String, String) {
    signed((
        "Rafael: you're all set 🙌".to_owned(),
        format!(
            "Notifications work on this device. I'll ping you when a budget line gets close, and in the evening if a day runs over. {SIGN_OFF}"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use finance_types::notify::{AvoidablePurchase, DayGroup, DayTotal};

    use super::*;

    #[test]
    fn messages_carry_the_chosen_persona() {
        let (title, body) = signed_as("Gisel", test_message());
        assert_eq!(title, "Gisel: you're all set 🙌");
        assert!(body.ends_with("— Gisel"));
        assert_eq!(signed_as("Rafael", test_message()), test_message());
    }

    fn line(severity: AlertSeverity, title: &str, body: &str) -> Notification {
        Notification {
            title: title.into(),
            body: body.into(),
            severity,
            kind: SubjectKind::Budget,
            name: "Groceries".into(),
        }
    }

    #[test]
    fn budget_lines_keep_their_facts_in_rafaels_voice() {
        let n = line(
            AlertSeverity::HeadsUp,
            "Groceries at 82% of budget",
            "Groceries: 82% of the plan used, €54.00 left this month.",
        );
        let (title, body) = alert(&n, 1);
        assert_eq!(title, "Rafael: Groceries at 82% of budget 👀");
        assert_eq!(
            body,
            "82% of the plan used, €54.00 left this month. One slower week and this lands right on plan. — Rafael"
        );
        let over = line(
            AlertSeverity::Critical,
            "Groceries is over budget",
            "Groceries: €310.00 of €300.00 spent, €10.00 over plan.",
        );
        let (title, body) = alert(&over, 0);
        assert_eq!(title, "Rafael: Groceries is over budget 🚨");
        assert!(body.contains("€10.00 over plan.") && body.ends_with("— Rafael"));
        assert!(CRITICAL.iter().any(|c| body.contains(c)));
    }

    #[test]
    fn generic_alerts_get_a_name_and_goals_a_friendly_line() {
        let generic = Notification {
            title: "Personal finance: warning".into(),
            body: "Eating out: forecast spend exceeds the category budget".into(),
            severity: AlertSeverity::Warning,
            kind: SubjectKind::Budget,
            name: "Eating out".into(),
        };
        let (title, body) = alert(&generic, 2);
        assert_eq!(title, "Rafael: Eating out needs a look ✋");
        assert!(body.starts_with("Forecast spend exceeds the category budget. Hold off"));
        let goal = Notification {
            title: "Personal finance: heads up".into(),
            body: "goal-7: goal is behind its contribution schedule".into(),
            severity: AlertSeverity::HeadsUp,
            kind: SubjectKind::Goal,
            name: "Japan trip".into(),
        };
        let (title, body) = alert(&goal, 0);
        assert_eq!(title, "Rafael: Japan trip needs a little love 💛");
        assert!(body.starts_with("Goal is behind its contribution schedule. Even a small top-up"));
    }

    #[test]
    fn the_same_subject_reads_the_same_all_day_and_varies_across_days() {
        let d = |n| jiff::civil::date(2026, 11, n);
        assert_eq!(seed("Groceries", d(25)), seed("Groceries", d(25)));
        let lines: std::collections::HashSet<&str> = (1..=28)
            .map(|n| pick(&HEADS_UP, seed("Groceries", d(n))))
            .collect();
        assert!(lines.len() > 1);
    }

    #[test]
    fn the_evening_summary_speaks_as_rafael() {
        let eur = Currency::eur();
        let s = DaySummary {
            date: "2026-11-25".into(),
            totals: vec![DayTotal {
                currency: eur.clone(),
                spent: "53".parse().unwrap(),
                allowance: "45".parse().unwrap(),
                over_by: "8".parse().unwrap(),
                groups: vec![DayGroup {
                    group: "Food delivery".into(),
                    spent: "18.5".parse().unwrap(),
                    allowance: "10".parse().unwrap(),
                }],
            }],
            avoidable: vec![AvoidablePurchase {
                merchant: "Thuisbezorgd".into(),
                group: "Food delivery".into(),
                amount: "18.5".parse().unwrap(),
                currency: eur,
                time: None,
                instead: "cooking at home, or picking it up on a walk".into(),
            }],
        };
        let (title, body) = day_summary(&s, 0);
        assert_eq!(title, "Rafael: today ran €8.00 over 👀");
        assert!(body.starts_with(
            "Thuisbezorgd €18.50: next time, try cooking at home, or picking it up on a walk."
        ));
        assert!(body.ends_with("— Rafael"));
        let no_skips = DaySummary {
            avoidable: Vec::new(),
            ..s
        };
        let (_, body) = day_summary(&no_skips, 0);
        assert!(body.starts_with("Most of it went to Food delivery."));
    }
}
