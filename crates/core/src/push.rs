//! The devices that receive Web Push (Manage → Settings → Phone
//! notifications). Delivery itself is in `finance_notify::push`; this keeps
//! the subscriptions the installed web app sends.

use finance_types::notify::PushSubscription;
use rusqlite::{Connection, params};

use crate::Result;
use crate::pyfmt::isoformat_utc;
use crate::service::Service;

pub fn list(conn: &Connection) -> Result<Vec<PushSubscription>> {
    let mut stmt = conn.prepare(
        "SELECT endpoint, p256dh, auth, origin FROM push_subscriptions ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PushSubscription {
            endpoint: r.get(0)?,
            p256dh: r.get(1)?,
            auth: r.get(2)?,
            origin: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Save (or renew) one device; the browser hands out a new endpoint when it
/// rotates keys, and the old one then answers 410 and is forgotten.
pub fn save(conn: &Connection, s: &PushSubscription, now: jiff::Timestamp) -> Result<()> {
    conn.execute(
        "INSERT INTO push_subscriptions (endpoint, p256dh, auth, origin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(endpoint) DO UPDATE SET p256dh = ?2, auth = ?3, origin = ?4",
        params![s.endpoint, s.p256dh, s.auth, s.origin, isoformat_utc(now)],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, endpoint: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM push_subscriptions WHERE endpoint = ?1",
        [endpoint],
    )?;
    Ok(())
}

/// The host of a push endpoint, for the settings card ("web.push.apple.com").
pub fn device_label(endpoint: &str) -> String {
    endpoint
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
        .to_owned()
}

impl Service {
    pub fn push_subscriptions(&self) -> std::result::Result<Vec<PushSubscription>, String> {
        self.read_store(list)
    }

    /// Store a subscription the caller has checked (`finance_notify::push`).
    pub fn save_push_subscription(&self, s: &PushSubscription) -> std::result::Result<(), String> {
        self.with_connection(|c| save(c, s, jiff::Timestamp::now()))
    }

    pub fn forget_push_subscription(&self, endpoint: &str) -> std::result::Result<(), String> {
        self.with_connection(|c| delete(c, endpoint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../storage/src/nudges.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn devices_are_saved_renewed_and_forgotten() {
        let conn = store();
        let s = PushSubscription {
            endpoint: "https://web.push.apple.com/a".into(),
            p256dh: "k1".into(),
            auth: "a1".into(),
            origin: "https://finance.example.com".into(),
        };
        save(&conn, &s, jiff::Timestamp::UNIX_EPOCH).unwrap();
        let renewed = PushSubscription {
            p256dh: "k2".into(),
            ..s.clone()
        };
        save(&conn, &renewed, jiff::Timestamp::UNIX_EPOCH).unwrap();
        assert_eq!(list(&conn).unwrap(), vec![renewed]);
        delete(&conn, &s.endpoint).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        assert_eq!(device_label(&s.endpoint), "web.push.apple.com");
    }
}
