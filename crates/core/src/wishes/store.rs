//! Reading the stored wishes and their settings.

use finance_types::gateway::{Wish, WishSettings};
use rusqlite::{Connection, OptionalExtension};

use crate::{Error, Result};

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |r| r.get(0),
    )?)
}

/// Every stored wish, oldest id first; empty before the table exists.
pub fn load_wishes(conn: &Connection) -> Result<Vec<Wish>> {
    if !table_exists(conn, "wishes")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT payload_json FROM wishes ORDER BY wish_id")?;
    let payloads = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    payloads
        .iter()
        .map(|p| serde_json::from_str(p).map_err(|e| Error::Invalid(format!("wish: {e}"))))
        .collect()
}

/// The stored settings, or the defaults.
pub fn load_wish_settings(conn: &Connection) -> Result<WishSettings> {
    if !table_exists(conn, "wish_settings")? {
        return Ok(WishSettings::default());
    }
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM wish_settings WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    payload.map_or_else(
        || Ok(WishSettings::default()),
        |p| serde_json::from_str(&p).map_err(|e| Error::Invalid(format!("wish settings: {e}"))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_storage::Store;
    use finance_types::gateway::{WishSource, WishStatus};
    use finance_types::report::Currency;
    use rust_decimal::Decimal;
    use std::path::Path;

    #[test]
    fn missing_tables_give_defaults() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(load_wishes(&conn).unwrap().is_empty());
        assert_eq!(
            load_wish_settings(&conn).unwrap().card_limit,
            Decimal::from(2500)
        );
    }

    #[test]
    fn stored_rows_round_trip() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let conn = store.connection();
        let wish = Wish {
            wish_id: "w1".into(),
            parent_id: None,
            name: "Bike".into(),
            amount: Decimal::from(900),
            currency: Currency::new("EUR"),
            due_on: "2026-12-01".into(),
            source: WishSource::Account {
                account_id: "a1".into(),
            },
            saved: Decimal::from(50),
            status: WishStatus::Planned,
            bought: None,
            created_at: "2026-10-28T10:00:00Z".into(),
        };
        let settings = WishSettings {
            card_limit: Decimal::from(3000),
            card_used: Decimal::from(120),
            card_used_month: "2026-10-01".into(),
            card_bill_id: Some("b1".into()),
        };
        conn.execute(
            "INSERT INTO wishes (wish_id, payload_json) VALUES ('w1', ?1)",
            [serde_json::to_string(&wish).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wish_settings (singleton, payload_json) VALUES (1, ?1)",
            [serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
        assert_eq!(load_wishes(conn).unwrap(), vec![wish]);
        assert_eq!(load_wish_settings(conn).unwrap(), settings);
    }
}
