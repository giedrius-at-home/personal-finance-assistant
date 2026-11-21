//! Owner settings: preview, apply and roll back (Python `web.py`
//! `/settings/preview`, `/settings/apply`, `/settings/{id}/rollback` and
//! `SQLiteDerivedState.preview_settings` / `apply_settings` /
//! `rollback_settings` / `list_settings_revisions`).

use std::collections::HashSet;
use std::str::FromStr;

use finance_types::gateway::OwnerSettings;
use finance_types::manage::{SettingsApply, SettingsForm, SettingsPreview, SettingsRevision};
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::csv_import::hex;
use super::{Writer, invalid};
use crate::{Error, Result};

const ACTOR: &str = "local-owner";

/// Alerts are delivered as macOS notifications by the app (the Python
/// service never had a delivery channel enabled).
pub const NOTIFICATIONS_AVAILABLE: bool = true;

/// The stored settings, or the defaults when none were saved.
pub fn current(conn: &Connection) -> Result<OwnerSettings> {
    let text: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM owner_settings WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match text {
        Some(text) => serde_json::from_str(&text).map_err(|e| Error::Invalid(e.to_string())),
        None => Ok(OwnerSettings::default()),
    }
}

/// The newest 50 revisions.
pub fn revisions(conn: &Connection) -> Result<Vec<SettingsRevision>> {
    let mut stmt = conn.prepare(
        "SELECT revision_id, actor, applied_at, active, rolled_back_at, rolled_back_by \
         FROM settings_history ORDER BY revision_id DESC LIMIT 50",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SettingsRevision {
            revision_id: r.get(0)?,
            actor: r.get(1)?,
            applied_at: r.get(2)?,
            active: r.get(3)?,
            rolled_back_at: r.get(4)?,
            rolled_back_by: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Python `json.dumps(value, sort_keys=True, separators=(",", ":"))` with
/// its default ASCII escaping.
fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_json(&Value::String(key.clone()), out);
                out.push(':');
                canonical_json(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        Value::String(text) => {
            let encoded = serde_json::to_string(text).unwrap_or_default();
            for c in encoded.chars() {
                if c.is_ascii() {
                    out.push(c);
                } else {
                    let mut units = [0_u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
            }
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Python `OwnerSettings.content_hash`.
pub fn content_hash(settings: &OwnerSettings) -> String {
    let value = serde_json::to_value(settings).unwrap_or(Value::Null);
    let mut text = String::new();
    canonical_json(&value, &mut text);
    hex(&Sha256::digest(text.as_bytes()))
}

fn token(before: &OwnerSettings, after: &OwnerSettings) -> String {
    hex(&Sha256::digest(
        format!(
            "settings-preview\0{}\0{}",
            content_hash(before),
            content_hash(after)
        )
        .as_bytes(),
    ))
}

/// `^\d+(\.\d{1,2})?$` with at most 12 digits.
fn money(text: &str) -> Option<Decimal> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let ok = !whole.is_empty()
        && digits(whole)
        && digits(fraction)
        && (!text.contains('.') || (1..=2).contains(&fraction.len()))
        && whole.trim_start_matches('0').len() + fraction.len() <= 12;
    if ok {
        Decimal::from_str(text).ok()
    } else {
        None
    }
}

fn whole<T: FromStr>(text: &str) -> Option<T> {
    let text = text.trim();
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn optional(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty() && text != "None").then_some(text)
}

/// Python `_settings`: the typed form as settings, keeping what the form
/// does not carry (policy version, savings-goal contributions).
pub fn from_form(form: &SettingsForm, current: &OwnerSettings) -> Result<OwnerSettings> {
    let timezone = form.timezone.trim();
    if timezone.is_empty() || timezone.len() > 64 {
        return Err(invalid("Enter a timezone."));
    }
    let settings = OwnerSettings {
        policy_version: OwnerSettings::default().policy_version,
        timezone: timezone.to_owned(),
        evaluation_hour: whole(&form.evaluation_hour)
            .ok_or_else(|| invalid("Evaluation hour must be a whole hour from 0 to 23."))?,
        notifications_enabled: form.notifications_enabled,
        savings_goal_contributions_enabled: current.savings_goal_contributions_enabled,
        payday_day: match optional(&form.payday_day) {
            None => None,
            Some(text) => Some(
                whole(text).ok_or_else(|| invalid("Monthly payday must be a day from 1 to 31."))?,
            ),
        },
        monthly_salary_eur: match optional(&form.monthly_salary_eur) {
            None => None,
            Some(text) => Some(money(text).ok_or_else(|| {
                invalid("Monthly salary must be an amount with at most two decimals.")
            })?),
        },
        salary_account_id: optional(&form.salary_account_id).map(str::to_owned),
        safety_buffer_eur: match optional(&form.safety_buffer_eur) {
            None => current.safety_buffer_eur,
            Some(text) => money(text).ok_or_else(|| {
                invalid("Safety buffer must be an amount with at most two decimals.")
            })?,
        },
        heads_up_spent_ratio: ratio(&form.heads_up_spent_ratio)
            .ok_or_else(|| invalid("Heads-up spent ratio must be a number from 0 to 1."))?,
        heads_up_pace_ratio: ratio(&form.heads_up_pace_ratio)
            .ok_or_else(|| invalid("Heads-up pace ratio must be a number above 0, at most 10."))?,
        heads_up_cooldown_hours: hours(&form.heads_up_cooldown_hours)?,
        warning_cooldown_hours: hours(&form.warning_cooldown_hours)?,
        critical_cooldown_hours: hours(&form.critical_cooldown_hours)?,
        forecast_excluded_categories: form.forecast_excluded_categories.clone(),
        predicted_bill_categories: form.predicted_bill_categories.clone(),
    };
    Ok(settings)
}

fn ratio(text: &str) -> Option<Decimal> {
    let text = text.trim();
    if text.is_empty() || text.len() > 8 {
        return None;
    }
    Decimal::from_str(text).ok()
}

fn hours(text: &str) -> Result<u16> {
    whole(text).ok_or_else(|| invalid("Cooldowns must be whole hours from 1 to 744."))
}

/// The model checks of Python's `OwnerSettings` (and the site's form
/// schema); returns the settings with excluded categories cleaned.
pub fn validate(mut s: OwnerSettings) -> Result<OwnerSettings> {
    if s.evaluation_hour > 23 {
        return Err(invalid(
            "Evaluation hour must be a whole hour from 0 to 23.",
        ));
    }
    if s.payday_day.is_some_and(|d| !(1..=31).contains(&d)) {
        return Err(invalid("Monthly payday must be a day from 1 to 31."));
    }
    if s.monthly_salary_eur
        .is_some_and(|m| m <= Decimal::ZERO || money(&m.to_string()).is_none())
    {
        return Err(invalid(
            "Monthly salary must be an amount with at most two decimals.",
        ));
    }
    if s.safety_buffer_eur < Decimal::ZERO || money(&s.safety_buffer_eur.to_string()).is_none() {
        return Err(invalid(
            "Safety buffer must be an amount with at most two decimals.",
        ));
    }
    if s.salary_account_id.as_ref().is_some_and(|a| a.len() > 128) {
        return Err(invalid("Unknown salary account."));
    }
    if s.heads_up_spent_ratio < Decimal::ZERO || s.heads_up_spent_ratio > Decimal::ONE {
        return Err(invalid(
            "Heads-up spent ratio must be a number from 0 to 1.",
        ));
    }
    if s.heads_up_pace_ratio <= Decimal::ZERO || s.heads_up_pace_ratio > Decimal::TEN {
        return Err(invalid(
            "Heads-up pace ratio must be a number above 0, at most 10.",
        ));
    }
    for h in [
        s.heads_up_cooldown_hours,
        s.warning_cooldown_hours,
        s.critical_cooldown_hours,
    ] {
        if !(1..=744).contains(&h) {
            return Err(invalid("Cooldowns must be whole hours from 1 to 744."));
        }
    }
    if !(s.critical_cooldown_hours <= s.warning_cooldown_hours
        && s.warning_cooldown_hours <= s.heads_up_cooldown_hours)
    {
        return Err(invalid("Cooldowns must shorten as severity increases."));
    }
    if jiff::tz::TimeZone::get(&s.timezone).is_err() {
        return Err(invalid("Timezone is unknown."));
    }
    s.forecast_excluded_categories = category_names(
        &s.forecast_excluded_categories,
        "Each excluded category needs a name of at most 128 characters.",
        "Exclude at most 50 categories.",
    )?;
    s.predicted_bill_categories = category_names(
        &s.predicted_bill_categories,
        "Each predicted bill needs a category name of at most 128 characters.",
        "Predict at most 50 categories.",
    )?;
    Ok(s)
}

/// Trimmed, case-insensitively unique category names, at most 50.
fn category_names(raw: &[String], too_long: &str, too_many: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in raw {
        let name = raw.trim();
        if name.chars().count() > 128 || name.contains(',') {
            return Err(invalid(too_long));
        }
        if !name.is_empty() && seen.insert(name.to_lowercase()) {
            names.push(name.to_owned());
        }
    }
    if names.len() > 50 {
        return Err(invalid(too_many));
    }
    Ok(names)
}

/// The route checks around the model: a changed salary account must exist,
/// and notifications need a delivery channel.
fn check(
    desired: &OwnerSettings,
    current: &OwnerSettings,
    accounts: &HashSet<String>,
) -> Result<()> {
    if let Some(account) = &desired.salary_account_id
        && Some(account) != current.salary_account_id.as_ref()
        && !accounts.contains(account)
    {
        return Err(invalid("Unknown salary account."));
    }
    if desired.notifications_enabled && !NOTIFICATIONS_AVAILABLE {
        return Err(invalid("Desktop notification delivery is not installed."));
    }
    Ok(())
}

/// Validate `form` against the stored settings and show what would change.
pub fn preview(
    conn: &Connection,
    form: &SettingsForm,
    accounts: &HashSet<String>,
) -> Result<SettingsPreview> {
    let before = current(conn)?;
    let after = validate(from_form(form, &before)?)?;
    check(&after, &before, accounts)?;
    Ok(SettingsPreview {
        token: token(&before, &after),
        before,
        after,
    })
}

/// `2026-10-02T14:45:01.572332+00:00`, as Python's `isoformat()` wrote it.
/// Apply exactly the previewed settings if nothing changed since; returns
/// the new revision id.
pub fn apply(w: &Writer<'_>, request: &SettingsApply, accounts: &HashSet<String>) -> Result<i64> {
    let before = current(&w.tx)?;
    let mut after = request.settings.clone();
    after.policy_version = OwnerSettings::default().policy_version;
    after.savings_goal_contributions_enabled = before.savings_goal_contributions_enabled;
    let after = validate(after)?;
    check(&after, &before, accounts)?;
    let expected = token(&before, &after);
    if request.token != expected {
        return Err(invalid(
            "Settings changed after the preview. Review the changes again.",
        ));
    }
    // A change made, rolled back and made again has the same token; keep
    // each history row unique.
    let used: i64 = w.tx.query_row(
        "SELECT count(*) FROM settings_history WHERE preview_token = ?1 OR preview_token LIKE ?1 || '-%'",
        [&expected],
        |r| r.get(0),
    )?;
    let stored_token = if used == 0 {
        expected
    } else {
        format!("{expected}-{used}")
    };
    let stamp = super::transactions::iso_now(w);
    let after_json = serde_json::to_string(&after).map_err(|e| Error::Invalid(e.to_string()))?;
    let before_json = serde_json::to_string(&before).map_err(|e| Error::Invalid(e.to_string()))?;
    w.tx.execute(
        "INSERT INTO settings_history(before_hash, after_hash, before_json, after_json, \
         preview_token, actor, applied_at, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
        params![
            content_hash(&before),
            content_hash(&after),
            before_json,
            after_json,
            stored_token,
            ACTOR,
            stamp
        ],
    )?;
    let revision = w.tx.last_insert_rowid();
    w.tx.execute(
        "INSERT INTO owner_settings(singleton, content_hash, payload_json, updated_at) \
         VALUES (1, ?1, ?2, ?3) ON CONFLICT(singleton) DO UPDATE SET \
         content_hash = excluded.content_hash, payload_json = excluded.payload_json, \
         updated_at = excluded.updated_at",
        params![content_hash(&after), after_json, stamp],
    )?;
    Ok(revision)
}

/// Restore the settings from before `revision_id`, if they were not changed
/// again since.
pub fn rollback(w: &Writer<'_>, revision_id: i64) -> Result<()> {
    let row: Option<(String, String, bool)> =
        w.tx.query_row(
            "SELECT before_hash, before_json, active FROM settings_history WHERE revision_id = ?1",
            [revision_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let after_hash: Option<String> =
        w.tx.query_row(
            "SELECT after_hash FROM settings_history WHERE revision_id = ?1",
            [revision_id],
            |r| r.get(0),
        )
        .optional()?;
    let (Some((before_hash, before_json, true)), Some(after_hash)) = (row, after_hash) else {
        return Err(invalid("This revision cannot be rolled back."));
    };
    if content_hash(&current(&w.tx)?) != after_hash {
        return Err(invalid("Newer settings prevent rollback of this revision."));
    }
    let stamp = super::transactions::iso_now(w);
    w.tx.execute(
        "UPDATE owner_settings SET content_hash = ?1, payload_json = ?2, updated_at = ?3 \
         WHERE singleton = 1",
        params![before_hash, before_json, stamp],
    )?;
    w.tx.execute(
        "UPDATE settings_history SET active = 0, rolled_back_at = ?1, rolled_back_by = ?2 \
         WHERE revision_id = ?3",
        params![stamp, ACTOR, revision_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = r#"{"policy_version":"2026-09-v1","timezone":"Europe/Amsterdam","evaluation_hour":18,"notifications_enabled":false,"savings_goal_contributions_enabled":false,"payday_day":26,"monthly_salary_eur":"3042.27","salary_account_id":null,"safety_buffer_eur":"300","heads_up_spent_ratio":"0.65","heads_up_pace_ratio":"1.25","heads_up_cooldown_hours":168,"warning_cooldown_hours":72,"critical_cooldown_hours":24,"forecast_excluded_categories":["Driving Lessons","Driving School","Credit Card","Flights","Dental Fees","Accomodation","Money Transfer","Transfer to me","NS","Education","Swapfiets","Insurance"]}"#;

    #[test]
    fn hash_matches_python() {
        let settings: OwnerSettings = serde_json::from_str(GOLDEN).expect("settings");
        assert_eq!(
            content_hash(&settings),
            "7f136f1f7c79bd8416a931a902ea7c37ed56437298f73bae73e69572bdd4cc90"
        );
        assert_eq!(serde_json::to_string(&settings).expect("json"), GOLDEN);
        let mut out = String::new();
        canonical_json(&Value::String("Café ✓".into()), &mut out);
        let slash = '\\';
        assert_eq!(out, format!("\"Caf{slash}u00e9 {slash}u2713\""));
    }

    fn form(s: &OwnerSettings) -> SettingsForm {
        SettingsForm {
            timezone: s.timezone.clone(),
            evaluation_hour: s.evaluation_hour.to_string(),
            notifications_enabled: s.notifications_enabled,
            payday_day: s.payday_day.map(|d| d.to_string()).unwrap_or_default(),
            monthly_salary_eur: s
                .monthly_salary_eur
                .map(|d| d.to_string())
                .unwrap_or_default(),
            salary_account_id: s.salary_account_id.clone().unwrap_or_default(),
            safety_buffer_eur: s.safety_buffer_eur.to_string(),
            heads_up_spent_ratio: s.heads_up_spent_ratio.to_string(),
            heads_up_pace_ratio: s.heads_up_pace_ratio.to_string(),
            heads_up_cooldown_hours: s.heads_up_cooldown_hours.to_string(),
            warning_cooldown_hours: s.warning_cooldown_hours.to_string(),
            critical_cooldown_hours: s.critical_cooldown_hours.to_string(),
            forecast_excluded_categories: s.forecast_excluded_categories.clone(),
            predicted_bill_categories: s.predicted_bill_categories.clone(),
        }
    }

    #[test]
    fn validates_like_the_model() {
        let base = OwnerSettings::default();
        let ok = |f: SettingsForm| validate(from_form(&f, &base)?);
        assert!(ok(form(&base)).is_ok());
        let mut f = form(&base);
        f.timezone = "Mars/Olympus".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.warning_cooldown_hours = "200".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.monthly_salary_eur = "3000.123".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.monthly_salary_eur = "0".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.heads_up_spent_ratio = "1.5".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.payday_day = "32".into();
        assert!(ok(f).is_err());
        let mut f = form(&base);
        f.payday_day = String::new();
        f.safety_buffer_eur = String::new();
        f.forecast_excluded_categories = vec!["Flights".into(), " flights ".into(), "NS".into()];
        let s = ok(f).expect("valid");
        assert_eq!(s.payday_day, None);
        assert_eq!(s.safety_buffer_eur, base.safety_buffer_eur);
        assert_eq!(s.forecast_excluded_categories, vec!["Flights", "NS"]);
    }

    fn store() -> finance_storage::Store {
        let store = finance_storage::Store::open(std::path::Path::new(":memory:")).expect("store");
        store
            .connection()
            .execute_batch(
                "CREATE TABLE owner_settings (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    content_hash TEXT NOT NULL, payload_json TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE settings_history (
                    revision_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    before_hash TEXT NOT NULL, after_hash TEXT NOT NULL,
                    before_json TEXT NOT NULL, after_json TEXT NOT NULL,
                    preview_token TEXT NOT NULL UNIQUE, actor TEXT NOT NULL,
                    applied_at TEXT NOT NULL, active INTEGER NOT NULL CHECK(active IN (0, 1)),
                    rolled_back_at TEXT, rolled_back_by TEXT);",
            )
            .expect("tables");
        store
    }

    #[test]
    fn preview_apply_rollback() {
        let mut store = store();
        let accounts: HashSet<String> = ["firefly:1".to_owned()].into();
        let mut f = form(&OwnerSettings::default());
        f.payday_day = "26".into();
        f.salary_account_id = "firefly:1".into();
        let p = preview(store.connection(), &f, &accounts).expect("preview");
        assert_eq!(p.after.payday_day, Some(26));
        let mut bad = f.clone();
        bad.salary_account_id = "firefly:9".into();
        assert!(preview(store.connection(), &bad, &accounts).is_err());

        let w = Writer {
            tx: store.connection_mut().transaction().expect("tx"),
            now: jiff::Zoned::now(),
        };
        let tampered = SettingsApply {
            token: p.token.clone(),
            settings: OwnerSettings {
                payday_day: Some(1),
                ..p.after.clone()
            },
        };
        assert!(apply(&w, &tampered, &accounts).is_err());
        let revision = apply(
            &w,
            &SettingsApply {
                token: p.token.clone(),
                settings: p.after.clone(),
            },
            &accounts,
        )
        .expect("apply");
        assert_eq!(current(&w.tx).expect("current").payday_day, Some(26));
        // The same preview cannot be applied twice.
        assert!(
            apply(
                &w,
                &SettingsApply {
                    token: p.token.clone(),
                    settings: p.after.clone(),
                },
                &accounts
            )
            .is_err()
        );
        let listed = revisions(&w.tx).expect("revisions");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].active);
        assert!(listed[0].applied_at.ends_with("+00:00"));

        rollback(&w, revision).expect("rollback");
        assert_eq!(current(&w.tx).expect("current").payday_day, None);
        assert!(rollback(&w, revision).is_err());
        let listed = revisions(&w.tx).expect("revisions");
        assert!(!listed[0].active);
        assert_eq!(listed[0].rolled_back_by.as_deref(), Some(ACTOR));

        // Making the same change again still records a new revision.
        let again = apply(
            &w,
            &SettingsApply {
                token: p.token.clone(),
                settings: p.after.clone(),
            },
            &accounts,
        )
        .expect("apply again");
        assert!(again > revision);
    }

    #[test]
    fn newer_settings_block_rollback() {
        let mut store = store();
        let accounts = HashSet::new();
        let w = Writer {
            tx: store.connection_mut().transaction().expect("tx"),
            now: jiff::Zoned::now(),
        };
        let mut f = form(&OwnerSettings::default());
        f.evaluation_hour = "9".into();
        let first = preview(&w.tx, &f, &accounts).expect("preview");
        let r1 = apply(
            &w,
            &SettingsApply {
                token: first.token,
                settings: first.after,
            },
            &accounts,
        )
        .expect("apply");
        f.evaluation_hour = "10".into();
        let second = preview(&w.tx, &f, &accounts).expect("preview");
        apply(
            &w,
            &SettingsApply {
                token: second.token,
                settings: second.after,
            },
            &accounts,
        )
        .expect("apply");
        assert!(rollback(&w, r1).is_err());
    }
}
