//! Spending groups: the reviewed group of each category and the owner's own
//! group names (Python `web.py` `/spending-groups/save` and
//! `/spending-groups/create`, `SQLiteDerivedState.save_group_mapping` and
//! `create_spending_group`, behind the site's `/api/finance/group` and
//! `/api/finance/group-create` routes, whose messages the owner sees).

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use super::{Writer, invalid};
use crate::Result;
use crate::groups::{DEFAULT_GROUPS, expense_categories};
use crate::text::casefold;
use crate::{Error, Snapshot};

const INVALID_INPUT: &str = "Invalid category, group or revision.";
const NOT_SAVED: &str = "The group could not be saved.";
const CHANGED: &str = "This category changed. Refresh and try again.";
const INVALID_NAME: &str = "Invalid group name.";
const NOT_CREATED: &str = "Could not save the group.";

/// Python `GroupName`: 1–64 characters after trimming, no control characters.
fn group_name(text: &str) -> Option<&str> {
    let name = text.trim();
    let length = name.chars().count();
    ((1..=64).contains(&length) && !name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}'))
        .then_some(name)
}

/// The categories a group may be saved for: those used by spending or refunds.
pub fn known_categories(snap: &Snapshot) -> BTreeSet<String> {
    let mut known = expense_categories(&snap.raw_transactions);
    known.extend(expense_categories(&snap.transactions));
    known
}

/// Python `save_group_mapping`: `Writer::stamp` with Python's `+00:00` offset.
fn reviewed_at(w: &Writer<'_>) -> String {
    let stamp = w.stamp();
    stamp
        .strip_suffix('Z')
        .map_or(stamp.clone(), |s| format!("{s}+00:00"))
}

/// Append a reviewed revision for `category` (a `None` group unassigns it)
/// if nobody saved it since `revision`.
pub fn save_group_mapping(
    w: &Writer<'_>,
    categories: &BTreeSet<String>,
    custom_groups: &[String],
    category: &str,
    group: Option<&str>,
    revision: u32,
) -> Result<()> {
    let category_length = category.chars().count();
    if !(1..=128).contains(&category_length) {
        return Err(invalid(INVALID_INPUT));
    }
    let group = match group {
        None => None,
        Some(text) => {
            let trimmed = text.trim();
            let length = trimmed.chars().count();
            if !(1..=64).contains(&length) {
                return Err(invalid(INVALID_INPUT));
            }
            (trimmed != "unassigned").then_some(trimmed)
        }
    };
    if !categories.contains(category) {
        return Err(invalid(NOT_SAVED));
    }
    if let Some(name) = group {
        let allowed = DEFAULT_GROUPS.contains(&name) || custom_groups.iter().any(|g| g == name);
        if !allowed || group_name(name).is_none() {
            return Err(invalid(NOT_SAVED));
        }
    }
    let changed = w.tx.execute(
        "INSERT INTO spending_group_history (category, revision, group_name, reviewed_at) \
         SELECT ?1, ?2, ?3, ?4 WHERE COALESCE((SELECT MAX(revision) \
         FROM spending_group_history WHERE category = ?1), 0) = ?5",
        params![category, revision + 1, group, reviewed_at(w), revision],
    )?;
    if changed != 1 {
        return Err(invalid(CHANGED));
    }
    Ok(())
}

/// Add an owner-named group and return its stored name. A built-in group's
/// name (any case) returns the built-in name; an existing custom group's
/// name returns that group unchanged.
pub fn create_spending_group(w: &Writer<'_>, name: &str) -> Result<String> {
    let Some(name) = group_name(name) else {
        return Err(invalid(INVALID_NAME));
    };
    let key = casefold(name);
    if key == "unassigned" {
        return Err(invalid(NOT_CREATED));
    }
    if let Some(standard) = DEFAULT_GROUPS.iter().find(|g| casefold(g) == key) {
        return Ok((*standard).to_owned());
    }
    w.tx.execute(
        "INSERT OR IGNORE INTO custom_spending_groups (name_key, name) VALUES (?1, ?2)",
        params![key, name],
    )?;
    w.tx.query_row(
        "SELECT name FROM custom_spending_groups WHERE name_key = ?1",
        [&key],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .ok_or_else(|| Error::Invalid("spending group insert was not persisted".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Zoned;
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE spending_group_history (
                category TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0),
                group_name TEXT, reviewed_at TEXT NOT NULL, PRIMARY KEY(category, revision));
             CREATE TABLE custom_spending_groups (name_key TEXT PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO custom_spending_groups VALUES ('pets', 'Pets');",
        )
        .unwrap();
        conn
    }

    fn writer(conn: &mut Connection) -> Writer<'_> {
        Writer {
            tx: conn.transaction().unwrap(),
            now: Zoned::now(),
        }
    }

    fn categories() -> BTreeSet<String> {
        ["Albert Heijn", "Vet"].map(String::from).into()
    }

    fn message<T: std::fmt::Debug>(result: Result<T>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn saves_revisions_and_rejects_stale_forms() {
        let mut conn = db();
        let custom = vec!["Pets".to_owned()];
        let w = writer(&mut conn);
        save_group_mapping(&w, &categories(), &custom, "Vet", Some("Pets"), 0).unwrap();
        save_group_mapping(&w, &categories(), &custom, "Vet", None, 1).unwrap();
        assert_eq!(
            message(save_group_mapping(
                &w,
                &categories(),
                &custom,
                "Vet",
                Some("Other"),
                1
            )),
            CHANGED
        );
        let rows: Vec<(u32, Option<String>, String)> = w
            .tx
            .prepare("SELECT revision, group_name, reviewed_at FROM spending_group_history ORDER BY revision")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.as_deref(), Some("Pets"));
        assert_eq!(rows[1].1, None);
        assert!(rows[0].2.ends_with("+00:00"));
    }

    #[test]
    fn rejects_unknown_categories_and_groups() {
        let mut conn = db();
        let w = writer(&mut conn);
        let custom = vec!["Pets".to_owned()];
        let cats = categories();
        assert_eq!(
            message(save_group_mapping(
                &w,
                &cats,
                &custom,
                "Nope",
                Some("Other"),
                0
            )),
            NOT_SAVED
        );
        assert_eq!(
            message(save_group_mapping(
                &w,
                &cats,
                &custom,
                "Vet",
                Some("Made up"),
                0
            )),
            NOT_SAVED
        );
        assert_eq!(
            message(save_group_mapping(&w, &cats, &custom, "", Some("Other"), 0)),
            INVALID_INPUT
        );
        assert_eq!(
            message(save_group_mapping(&w, &cats, &custom, "Vet", Some("  "), 0)),
            INVALID_INPUT
        );
        save_group_mapping(&w, &cats, &custom, "Albert Heijn", Some(" Groceries "), 0).unwrap();
    }

    #[test]
    fn creates_custom_groups_once() {
        let mut conn = db();
        let w = writer(&mut conn);
        assert_eq!(create_spending_group(&w, "  Family ").unwrap(), "Family");
        assert_eq!(create_spending_group(&w, "FAMILY").unwrap(), "Family");
        assert_eq!(create_spending_group(&w, "pets").unwrap(), "Pets");
        assert_eq!(create_spending_group(&w, "groceries").unwrap(), "Groceries");
        assert_eq!(
            message(create_spending_group(&w, "Unassigned")),
            NOT_CREATED
        );
        assert_eq!(message(create_spending_group(&w, "")), INVALID_NAME);
        assert_eq!(message(create_spending_group(&w, "a\u{7}b")), INVALID_NAME);
        assert_eq!(
            message(create_spending_group(&w, &"x".repeat(65))),
            INVALID_NAME
        );
        let count: i64 =
            w.tx.query_row("SELECT COUNT(*) FROM custom_spending_groups", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
    }
}
