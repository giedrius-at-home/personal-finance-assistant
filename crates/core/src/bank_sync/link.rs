//! Connecting a bank from the app, replacing the importer's
//! `LinkController`: ask Enable Banking for an authorisation URL, take the
//! code from the address the bank redirects to, open a session and let the
//! owner map its accounts onto ledger accounts. The importer's rules for a
//! renewed consent apply (every ledger account stays mapped).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use finance_types::bank::{BankLinkAccount, BankLinkChoice, BankLinkProposal, LedgerTarget, aspsp};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use super::{Connection, Identity, OwnAccount, normalize_iban, sha256_hex};
use crate::writes::utc_stamp;

/// The importer asked for 90 days when the bank did not say more.
const DEFAULT_DAYS: i64 = 90;
/// Enable Banking accepts at most 180 days for most banks.
const MAX_DAYS: i64 = 180;

/// The `POST /auth` body for `bank` (`PostAuthRequest::post`). `aspsps` is
/// the answer of `GET /aspsps?country=…`, whose `maximum_consent_validity`
/// (seconds) caps the requested access.
pub fn auth_body(
    bank: &str,
    redirect_url: &str,
    state: &str,
    now: Timestamp,
    aspsps: Option<&Value>,
) -> Result<Value, String> {
    let (name, country) = aspsp(bank).ok_or("Unknown bank")?;
    let limit = aspsps
        .and_then(|a| a.get("aspsps"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| {
            a.get("name").and_then(Value::as_str) == Some(name)
                && a.get("country").and_then(Value::as_str) == Some(country)
        })
        .and_then(|a| a.get("maximum_consent_validity"))
        .and_then(Value::as_i64);
    let seconds = limit.map_or(DEFAULT_DAYS * 86_400, |s| s.min(MAX_DAYS * 86_400));
    let valid_until = now
        .checked_add(SignedDuration::from_secs(seconds - 60))
        .map_err(|_| "Invalid consent length")?;
    if !redirect_url.starts_with("https://") {
        return Err("The redirect URL must start with https://".into());
    }
    Ok(json!({
        "access": {"valid_until": valid_until.to_string()},
        "aspsp": {"name": name, "country": country},
        "state": state,
        "redirect_url": redirect_url,
        "psu_type": "personal",
    }))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The authorisation code in the address the bank redirected to, after
/// checking it answers request `state`.
pub fn callback_code(address: &str, state: &str) -> Result<String, String> {
    let query = address.split_once('?').map(|(_, q)| q).ok_or(
        "Paste the full address of the page the bank opened, including everything after “?”.",
    )?;
    let query = query.split('#').next().unwrap_or("");
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect();
    if let Some(error) = params.get("error") {
        let detail = params.get("error_description").unwrap_or(error);
        return Err(format!("The bank did not approve access: {detail}."));
    }
    if params.get("state").map(String::as_str) != Some(state) {
        return Err("This address belongs to another connection attempt. Start again.".into());
    }
    params
        .get("code")
        .filter(|c| !c.is_empty())
        .cloned()
        .ok_or_else(|| "The address has no authorisation code. Start again.".into())
}

/// One account of an approved session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAccount {
    pub uid: String,
    pub name: String,
    /// Normalised; empty when the bank discloses none.
    pub iban: String,
    pub currency: Option<String>,
}

/// An approved session (`POST /sessions`).
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub accounts: Vec<SessionAccount>,
    pub valid_until: Option<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("accounts", &self.accounts.len())
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let id = value
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("Enable Banking returned no session.")?
            .to_owned();
        let accounts = value
            .get("accounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let uid = a.get("uid")?.as_str()?.to_owned();
                let text = |key: &str| a.get(key).and_then(Value::as_str).unwrap_or("").trim();
                let name = [text("name"), text("details"), text("product")]
                    .into_iter()
                    .find(|n| !n.is_empty())
                    .unwrap_or("Account")
                    .to_owned();
                let iban = normalize_iban(
                    a.get("account_id")
                        .and_then(|i| i.get("iban"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                let currency = a.get("currency").and_then(Value::as_str).map(str::to_owned);
                Some(SessionAccount {
                    uid,
                    name,
                    iban,
                    currency,
                })
            })
            .collect::<Vec<_>>();
        if accounts.is_empty() {
            return Err(
                "The bank shared no accounts. Start again and select the accounts to share.".into(),
            );
        }
        let valid_until = value
            .get("access")
            .and_then(|a| a.get("valid_until"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Timestamp>().ok())
            .map(utc_stamp);
        Ok(Self {
            id,
            accounts,
            valid_until,
        })
    }
}

fn tail(iban: &str) -> Option<String> {
    (iban.len() >= 4).then(|| iban[iban.len() - 4..].to_owned())
}

fn targets(own: &HashMap<i64, OwnAccount>) -> Vec<&OwnAccount> {
    let mut list: Vec<&OwnAccount> = own
        .values()
        .filter(|a| a.kind == "asset" && a.active)
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    list
}

/// The ledger account `account` most likely fills: the previous consent's
/// choice for the same uid, the account with the same IBAN, or the one
/// whose name contains the bank account's name.
fn suggest(
    account: &SessionAccount,
    old: Option<&Connection>,
    own: &HashMap<i64, OwnAccount>,
) -> Option<i64> {
    let candidates = targets(own);
    if let Some(target) = old.and_then(|o| o.accounts.get(&account.uid)) {
        return Some(*target);
    }
    if !account.iban.is_empty()
        && let Some(found) = candidates.iter().find(|a| a.iban == account.iban)
    {
        return Some(found.id);
    }
    let name = account.name.to_lowercase();
    let by_name: Vec<&&OwnAccount> = candidates
        .iter()
        .filter(|a| a.iban.is_empty() && name.len() >= 3 && a.name.to_lowercase().contains(&name))
        .collect();
    match by_name.as_slice() {
        [one] => Some(one.id),
        _ => None,
    }
}

/// What the owner sees after approving: every shared account with a
/// suggested ledger account.
pub fn proposal(
    bank: &str,
    old: Option<&Connection>,
    session: &Session,
    own: &HashMap<i64, OwnAccount>,
) -> BankLinkProposal {
    BankLinkProposal {
        bank: bank.to_owned(),
        accounts: session
            .accounts
            .iter()
            .map(|a| BankLinkAccount {
                uid: a.uid.clone(),
                name: a.name.clone(),
                iban_tail: tail(&a.iban),
                currency: a.currency.clone(),
                suggested: suggest(a, old, own),
            })
            .collect(),
        targets: targets(own)
            .into_iter()
            .map(|a| LedgerTarget {
                id: a.id,
                name: a.name.clone(),
                iban_tail: tail(&a.iban),
            })
            .collect(),
        required: old.map(Connection::targets).unwrap_or_default(),
        valid_until: session.valid_until.clone(),
    }
}

/// The consent the owner's choices make. Each mapping the owner confirmed
/// is recorded as a verified identity bound to this session, so banks that
/// stop disclosing IBANs after consent keep syncing.
pub fn confirm(
    old: Option<&Connection>,
    session: &Session,
    choices: &[BankLinkChoice],
    own: &HashMap<i64, OwnAccount>,
) -> Result<Connection, String> {
    let shared: HashMap<&str, &SessionAccount> = session
        .accounts
        .iter()
        .map(|a| (a.uid.as_str(), a))
        .collect();
    let mut accounts = BTreeMap::new();
    let mut verified = BTreeMap::new();
    let mut used = BTreeSet::new();
    let sessions = vec![session.id.clone()];
    for choice in choices {
        let Some(target) = choice.target else {
            continue;
        };
        let account = shared
            .get(choice.uid.as_str())
            .ok_or("A chosen bank account is not part of this approval.")?;
        let ledger = own
            .get(&target)
            .filter(|a| a.kind == "asset" && a.active)
            .ok_or("Choose an open asset account for every bank account you import.")?;
        if !used.insert(target) {
            return Err(format!("{} is chosen for two bank accounts.", ledger.name));
        }
        if !account.iban.is_empty() && !ledger.iban.is_empty() && account.iban != ledger.iban {
            return Err(format!(
                "{} has a different IBAN than {}.",
                account.name, ledger.name
            ));
        }
        accounts.insert(account.uid.clone(), target);
        verified.insert(
            account.uid.clone(),
            Identity {
                target,
                sessions: sessions.clone(),
                iban_sha256: (!ledger.iban.is_empty()).then(|| sha256_hex(&ledger.iban)),
                no_iban: ledger.iban.is_empty(),
            },
        );
    }
    if accounts.is_empty() {
        return Err("Choose a ledger account for at least one bank account.".into());
    }
    if let Some(old) = old {
        let missing: Vec<String> = old
            .targets()
            .into_iter()
            .filter(|t| !used.contains(t))
            .map(|t| {
                own.get(&t)
                    .map_or_else(|| format!("account {t}"), |a| a.name.clone())
            })
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "Keep {} mapped: the previous connection filled it.",
                missing.join(", ")
            ));
        }
    }
    Ok(Connection {
        sessions,
        accounts,
        verified_identity: verified,
        import_not_before: old.and_then(|o| o.import_not_before),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> HashMap<i64, OwnAccount> {
        let account = |id, name: &str, iban: &str| {
            (
                id,
                OwnAccount {
                    id,
                    name: name.into(),
                    kind: "asset".into(),
                    active: true,
                    iban: iban.into(),
                },
            )
        };
        HashMap::from([
            account(76, "Revolut", "LT001"),
            account(413, "Revolut Savings", ""),
            account(416, "Revolut Fun out", ""),
        ])
    }

    fn session() -> Session {
        Session::parse(&json!({
            "session_id": "s-new",
            "access": {"valid_until": "2027-04-25T10:00:00+00:00"},
            "accounts": [
                {"uid": "u1", "name": "Main", "account_id": {"iban": "lt00 1"}, "currency": "EUR"},
                {"uid": "u2", "name": "Savings", "currency": "EUR"},
                {"uid": "u3", "details": "Fun out"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn auth_body_caps_access_and_requires_https() {
        let now: Timestamp = "2026-10-27T10:00:00Z".parse().unwrap();
        let aspsps = json!({"aspsps": [{"name": "Swedbank", "country": "LT", "maximum_consent_validity": 15_552_000}]});
        let body = auth_body(
            "swedbank",
            "https://localhost:8444/eb-callback",
            "st",
            now,
            Some(&aspsps),
        )
        .unwrap();
        assert_eq!(body["aspsp"]["country"], "LT");
        assert_eq!(body["access"]["valid_until"], "2027-04-25T09:59:00Z");
        assert!(auth_body("swedbank", "http://x", "st", now, None).is_err());
    }

    #[test]
    fn reads_the_code_from_the_landing_address() {
        let url = "https://localhost:8444/eb-callback?state=abc&code=c%2B1";
        assert_eq!(callback_code(url, "abc").unwrap(), "c+1");
        assert!(callback_code(url, "other").is_err());
        assert!(
            callback_code(
                "https://localhost:8444/eb-callback?error=access_denied&state=abc",
                "abc"
            )
            .is_err()
        );
        assert!(callback_code("https://localhost/", "abc").is_err());
    }

    #[test]
    fn suggests_by_iban_and_pocket_name() {
        let p = proposal("revolut", None, &session(), &own());
        let suggested: Vec<Option<i64>> = p.accounts.iter().map(|a| a.suggested).collect();
        assert_eq!(suggested, [Some(76), Some(413), Some(416)]);
        assert_eq!(
            p.valid_until.as_deref(),
            Some("2027-04-25T10:00:00.000000Z")
        );
    }

    #[test]
    fn confirm_binds_choices_and_keeps_old_targets() {
        let choices = vec![
            BankLinkChoice {
                uid: "u1".into(),
                target: Some(76),
            },
            BankLinkChoice {
                uid: "u2".into(),
                target: Some(413),
            },
            BankLinkChoice {
                uid: "u3".into(),
                target: None,
            },
        ];
        let c = confirm(None, &session(), &choices, &own()).unwrap();
        assert_eq!(c.targets(), [76, 413]);
        assert!(c.no_iban_bound("u2", 413, ""));
        assert!(c.hash_bound("u1", 76, "LT001"));

        let old = Connection {
            sessions: vec!["s-old".into()],
            accounts: BTreeMap::from([("x".into(), 416)]),
            verified_identity: BTreeMap::new(),
            import_not_before: None,
        };
        assert!(
            confirm(Some(&old), &session(), &choices, &own())
                .unwrap_err()
                .contains("Fun out")
        );
        let twice = vec![
            BankLinkChoice {
                uid: "u2".into(),
                target: Some(413),
            },
            BankLinkChoice {
                uid: "u3".into(),
                target: Some(413),
            },
        ];
        assert!(confirm(None, &session(), &twice, &own()).is_err());
    }
}
