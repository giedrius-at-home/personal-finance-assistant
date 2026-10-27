//! A bank's consent: the Enable Banking sessions and the ledger account each
//! bank account fills. It is the old importer's `<bank>.json`, reduced to the
//! fields sync needs (`bank_import_worker.php` validated the same ones).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use finance_types::bank::SyncStatus;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An owner-verified binding of one bank account to its ledger account, for
/// banks that disclose no IBAN after consent. It holds only for the exact
/// target and sessions it was made for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub target: i64,
    pub sessions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iban_sha256: Option<String>,
    #[serde(default)]
    pub no_iban: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Enable Banking session ids; with the application key they grant
    /// read access, so they live in the secret store only.
    pub sessions: Vec<String>,
    /// Enable Banking account uid → ledger account id.
    pub accounts: BTreeMap<String, i64>,
    #[serde(default)]
    pub verified_identity: BTreeMap<String, Identity>,
    /// Never import before this day (the cut-over from an older import).
    #[serde(default)]
    pub import_not_before: Option<Date>,
    /// The bank's name for each account the consent shares (uid → name),
    /// to recognise transfers between accounts without IBANs (pockets).
    #[serde(default)]
    pub names: BTreeMap<String, String>,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("sessions", &self.sessions.len())
            .field("targets", &self.targets())
            .finish_non_exhaustive()
    }
}

fn identity(value: &Value) -> Option<Identity> {
    Some(Identity {
        target: value.get("target")?.as_i64()?,
        sessions: value
            .get("sessions")?
            .as_array()?
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect::<Option<_>>()?,
        iban_sha256: value
            .get("iban_sha256")
            .and_then(Value::as_str)
            .map(str::to_owned),
        no_iban: value.get("no_iban").and_then(Value::as_bool) == Some(true),
    })
}

impl Connection {
    /// Read an importer configuration as the worker's
    /// `validateBankConfiguration` did.
    pub fn from_importer(raw: &str) -> Result<Self, SyncStatus> {
        let value: Value =
            serde_json::from_str(raw).map_err(|_| SyncStatus::InvalidConfiguration)?;
        if value.get("flow").and_then(Value::as_str) != Some("eb") {
            return Err(SyncStatus::InvalidConfiguration);
        }
        let sessions: Vec<String> = value
            .get("enable_banking_sessions")
            .and_then(Value::as_array)
            .ok_or(SyncStatus::InvalidConfiguration)?
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or(SyncStatus::InvalidConfiguration)?;
        let raw_accounts = value
            .get("accounts")
            .and_then(Value::as_object)
            .ok_or(SyncStatus::InvalidConfiguration)?;
        if sessions.is_empty() || raw_accounts.is_empty() {
            return Err(SyncStatus::InvalidConfiguration);
        }
        let mut accounts = BTreeMap::new();
        for (uid, target) in raw_accounts {
            // PHP `is_int`: a JSON integer, never a string or a float.
            let target = target
                .as_i64()
                .filter(|t| *t > 0)
                .ok_or(SyncStatus::InvalidMapping)?;
            accounts.insert(uid.clone(), target);
        }
        let verified_identity = value
            .get("pfa_verified_identity")
            .and_then(Value::as_object)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter_map(|(uid, b)| Some((uid.clone(), identity(b)?)))
                    .collect()
            })
            .unwrap_or_default();
        let import_not_before = match value.get("pfa_import_not_before") {
            None | Some(Value::Null) => None,
            Some(day) => Some(
                day.as_str()
                    .filter(|d| d.len() == 10)
                    .and_then(|d| d.parse::<Date>().ok())
                    .ok_or(SyncStatus::InvalidConfiguration)?,
            ),
        };
        Ok(Self {
            sessions,
            accounts,
            verified_identity,
            import_not_before,
            names: BTreeMap::new(),
        })
    }

    /// A renewed consent replacing `old` (`bank_control.py` `_consent`):
    /// every ledger account `old` filled must stay mapped, new ones may be
    /// added, and no two bank accounts may fill the same ledger account.
    /// The cut-over day carries over; a verified identity only while the
    /// accounts and sessions are unchanged.
    pub fn renew(old: Option<&Self>, raw: &str) -> Result<Self, SyncStatus> {
        let provider = serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|v| v.get("pfa_provider").cloned());
        if provider.is_some_and(|p| !p.is_null()) {
            return Err(SyncStatus::MappingMismatch);
        }
        let mut candidate = Self::from_importer(raw).map_err(|status| match status {
            SyncStatus::InvalidMapping => SyncStatus::MappingMismatch,
            other => other,
        })?;
        let targets = candidate.targets();
        let unique: BTreeSet<i64> = targets.iter().copied().collect();
        if unique.len() != targets.len() {
            return Err(SyncStatus::MappingMismatch);
        }
        // The owner's own confirmations in the file stay when they are bound
        // to exactly this file's sessions and mapping (Python dropped them;
        // pockets cannot be renewed without them).
        let sessions = candidate.sessions.clone();
        let accounts = candidate.accounts.clone();
        candidate
            .verified_identity
            .retain(|uid, b| b.sessions == sessions && accounts.get(uid) == Some(&b.target));
        candidate.import_not_before = None;
        if let Some(old) = old {
            if !old.targets().iter().all(|t| unique.contains(t)) {
                return Err(SyncStatus::MappingMismatch);
            }
            candidate.import_not_before = old.import_not_before;
            if old.accounts == candidate.accounts && old.sessions == candidate.sessions {
                candidate
                    .verified_identity
                    .clone_from(&old.verified_identity);
            }
        }
        Ok(candidate)
    }

    /// The ledger account ids this consent fills, in uid order.
    pub fn targets(&self) -> Vec<i64> {
        self.accounts.values().copied().collect()
    }

    fn binding(&self, uid: &str, target: i64) -> Option<&Identity> {
        self.verified_identity
            .get(uid)
            .filter(|b| b.target == target && b.sessions == self.sessions)
    }

    /// `hashIdentityBound`: the owner verified this ledger IBAN for `uid`.
    pub fn hash_bound(&self, uid: &str, target: i64, ledger_iban: &str) -> bool {
        !ledger_iban.is_empty()
            && self.binding(uid, target).is_some_and(|b| {
                b.iban_sha256.as_deref() == Some(super::sha256_hex(ledger_iban).as_str())
            })
    }

    /// `noIbanIdentityBound`: the ledger account has no IBAN (a pocket) and
    /// the owner confirmed this mapping for these sessions.
    pub fn no_iban_bound(&self, uid: &str, target: i64, ledger_iban: &str) -> bool {
        ledger_iban.is_empty() && self.binding(uid, target).is_some_and(|b| b.no_iban)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{"flow":"eb","enable_banking_sessions":["s1"],
        "accounts":{"uid-a":5,"uid-b":7},"access_token":"x",
        "pfa_verified_identity":{"uid-b":{"target":7,"sessions":["s1"],"no_iban":true}},
        "pfa_import_not_before":"2026-03-01"}"#;

    #[test]
    fn reads_the_importer_configuration() {
        let c = Connection::from_importer(CONFIG).unwrap();
        assert_eq!(c.sessions, ["s1"]);
        assert_eq!(c.targets(), [5, 7]);
        assert_eq!(c.import_not_before, Some(jiff::civil::date(2026, 3, 1)));
        assert!(c.no_iban_bound("uid-b", 7, ""));
        assert!(!c.no_iban_bound("uid-b", 7, "NL00"));
        assert!(!c.no_iban_bound("uid-a", 5, ""));
    }

    #[test]
    fn rejects_bad_configurations() {
        assert_eq!(
            Connection::from_importer(r#"{"flow":"csv"}"#).unwrap_err(),
            SyncStatus::InvalidConfiguration
        );
        assert_eq!(
            Connection::from_importer(
                r#"{"flow":"eb","enable_banking_sessions":["s"],"accounts":{"u":"5"}}"#
            )
            .unwrap_err(),
            SyncStatus::InvalidMapping
        );
        assert_eq!(
            Connection::from_importer(
                r#"{"flow":"eb","enable_banking_sessions":[],"accounts":{"u":5}}"#
            )
            .unwrap_err(),
            SyncStatus::InvalidConfiguration
        );
    }

    #[test]
    fn renewal_keeps_targets_and_cutover() {
        let old = Connection::from_importer(CONFIG).unwrap();
        let renewed = Connection::renew(
            Some(&old),
            r#"{"flow":"eb","enable_banking_sessions":["s2"],"accounts":{"n1":5,"n2":7,"n3":9}}"#,
        )
        .unwrap();
        assert_eq!(renewed.import_not_before, old.import_not_before);
        assert!(renewed.verified_identity.is_empty());
        // Dropping ledger account 7 is refused.
        assert_eq!(
            Connection::renew(
                Some(&old),
                r#"{"flow":"eb","enable_banking_sessions":["s2"],"accounts":{"n1":5}}"#
            )
            .unwrap_err(),
            SyncStatus::MappingMismatch
        );
        // Two bank accounts into one ledger account is refused.
        assert_eq!(
            Connection::renew(
                None,
                r#"{"flow":"eb","enable_banking_sessions":["s2"],"accounts":{"n1":5,"n2":5}}"#
            )
            .unwrap_err(),
            SyncStatus::MappingMismatch
        );
        // A binding scoped to the file's own session stays; a stale one goes.
        let pockets = Connection::renew(
            Some(&old),
            r#"{"flow":"eb","enable_banking_sessions":["s3"],"accounts":{"m":5,"p":7},
                "pfa_verified_identity":{
                    "p":{"target":7,"sessions":["s3"],"no_iban":true},
                    "m":{"target":5,"sessions":["old"],"no_iban":true}}}"#,
        )
        .unwrap();
        assert_eq!(pockets.verified_identity.keys().collect::<Vec<_>>(), ["p"]);
        // Same accounts and sessions keep the verified identity.
        let same = Connection::renew(Some(&old), CONFIG).unwrap();
        assert_eq!(same.verified_identity, old.verified_identity);
    }
}
