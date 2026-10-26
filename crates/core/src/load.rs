//! Build the canonical snapshot from the ledger and the gateway tables.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::LazyLock;

use finance_storage::Store;
use finance_types::gateway::{
    AccountRole, BudgetPlan, DailyReviewEntry, DailyReviewPolicy, GroupMapping, OwnerSettings,
    PlannedPayment, SavedDetails, SavingsGoalRecord, SpendingLimit, SubscriptionCategoryRule,
};
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;

use crate::model::{Account, Kind, Tx};
use crate::text::{card_purchase_date, compact_iban, counterparty_name, iban_country};
use crate::{Error, Result};

/// A ledger bill (recurring payment), as Firefly had it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bill {
    pub id: String,
    pub name: String,
    pub currency: Currency,
    pub amount_min: Decimal,
    pub amount_max: Decimal,
    pub start: Date,
    pub end: Option<Date>,
    /// weekly, monthly, quarterly, half-year or yearly.
    pub repeat_freq: String,
    pub skip: u32,
    pub active: bool,
}

/// Everything the reports read, loaded once per build.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub settings: OwnerSettings,
    pub zone: TimeZone,
    pub accounts: Vec<Account>,
    pub transactions: Vec<Tx>,
    /// Transactions before personal-spending rewrites (own transfers resolved).
    pub raw_transactions: Vec<Tx>,
    pub goals: Vec<SavingsGoalRecord>,
    pub bills: Vec<Bill>,
    /// Keyed by `Tx::source_key`.
    pub details: HashMap<String, SavedDetails>,
    pub group_mappings: Vec<GroupMapping>,
    pub custom_groups: Vec<String>,
    pub budget_plans: Vec<BudgetPlan>,
    pub planned_payments: Vec<PlannedPayment>,
    pub archived_payments: Vec<PlannedPayment>,
    pub spending_limits: Vec<SpendingLimit>,
    pub subscription_rules: Vec<SubscriptionCategoryRule>,
    /// (bill id, transaction id) pairs the owner said are not the bill.
    pub declined_bill_matches: HashSet<(String, String)>,
    pub deferred_sources: HashSet<String>,
    pub account_roles: BTreeMap<String, AccountRole>,
    /// `month_periods` payloads, oldest month first.
    pub month_periods: Vec<serde_json::Value>,
    /// When each transaction was first seen (`firefly:<id>` → instant).
    pub first_observed: HashMap<String, Timestamp>,
    pub daily_review_policy: Option<DailyReviewPolicy>,
    pub daily_review_entries: Vec<DailyReviewEntry>,
    pub daily_review_baseline: HashSet<String>,
    /// Last successful bank refresh, last attempt, and whether it failed.
    pub sync_health: (Option<Timestamp>, Option<Timestamp>, bool),
    /// When the ledger (accounts, transactions, bills) was last imported.
    pub last_import: Option<Timestamp>,
    /// The latest non-superseded category proposal of each transaction.
    pub category_proposals: Vec<finance_types::report::review::CategoryProposal>,
}

impl Snapshot {
    pub fn load(store: &Store) -> Result<Self> {
        let conn = store.connection();
        let settings: OwnerSettings = optional_payload(
            conn,
            "SELECT payload_json FROM owner_settings WHERE singleton = 1",
        )?
        .unwrap_or_default();
        let zone = TimeZone::get(&settings.timezone)
            .map_err(|_| Error::Invalid(format!("unknown time zone {}", settings.timezone)))?;
        let accounts = load_accounts(conn)?;
        let transactions = load_transactions(conn, &accounts, &zone)?;
        let transactions = crate::transfers::resolve_own_transfers(transactions, &accounts);
        // The payload holds only the owner's values; the revision is a column.
        let details: HashMap<String, SavedDetails> = rows(
            conn,
            "SELECT source_key, revision, payload_json FROM transaction_details",
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?
        .into_iter()
        .map(|(k, revision, v)| {
            // The payload holds only the values; the revision is its own column.
            let values = parse(&v)?;
            Ok((normalize_key(&k), SavedDetails { revision, values }))
        })
        .collect::<Result<_>>()?;
        let raw_transactions = transactions.clone();
        let transactions = crate::personal::apply_personal_spending(transactions, &details);
        Ok(Self {
            goals: payloads(conn, "SELECT payload_json FROM canonical_goals ORDER BY goal_id")?,
            bills: load_bills(conn)?,
            group_mappings: rows(
                conn,
                "SELECT category, group_name, revision FROM spending_group_history AS h \
                 WHERE revision = (SELECT MAX(revision) FROM spending_group_history \
                 WHERE category = h.category) ORDER BY category",
                |r| {
                    Ok(GroupMapping {
                        category: r.get(0)?,
                        group: r.get(1)?,
                        revision: r.get(2)?,
                    })
                },
            )?,
            custom_groups: rows(
                conn,
                "SELECT name FROM custom_spending_groups ORDER BY name_key",
                |r| r.get(0),
            )?,
            budget_plans: payloads(
                conn,
                "SELECT payload_json FROM monthly_budget_plans ORDER BY month, currency_code",
            )?,
            planned_payments: payloads(
                conn,
                "SELECT payload_json FROM planned_payments WHERE archived_at IS NULL ORDER BY plan_id",
            )?,
            archived_payments: payloads(
                conn,
                "SELECT payload_json FROM planned_payments WHERE archived_at IS NOT NULL ORDER BY plan_id",
            )?,
            spending_limits: payloads(
                conn,
                "SELECT payload_json FROM spending_group_limits ORDER BY group_key, currency_code",
            )?,
            subscription_rules: payloads(
                conn,
                "SELECT payload_json FROM subscription_category_rules ORDER BY bill_id",
            )?,
            declined_bill_matches: rows(
                conn,
                "SELECT bill_id, transaction_id FROM declined_bill_matches",
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .into_iter()
            .collect(),
            deferred_sources: rows(conn, "SELECT source_key FROM review_deferrals", |r| {
                r.get::<_, String>(0)
            })?
            .iter()
            .map(|k| normalize_key(k))
            .collect(),
            account_roles: payloads::<AccountRole>(
                conn,
                "SELECT payload_json FROM account_roles ORDER BY account_id",
            )?
            .into_iter()
            .map(|r| (r.account_id.clone(), r))
            .collect(),
            month_periods: payloads(
                conn,
                "SELECT payload_json FROM month_periods ORDER BY month",
            )?,
            first_observed: rows(
                conn,
                "SELECT canonical_id, MIN(observed_at) FROM source_versions \
                 WHERE record_kind = 'transaction' GROUP BY canonical_id",
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?
            .into_iter()
            .filter_map(|(id, at)| parse_instant(&at).map(|t| (id, t)))
            .collect(),
            daily_review_policy: optional_payload(
                conn,
                "SELECT payload_json FROM daily_review_policy WHERE singleton = 1",
            )?,
            daily_review_entries: payloads(
                conn,
                "SELECT payload_json FROM daily_review_entries ORDER BY transaction_id",
            )?,
            daily_review_baseline: rows(
                conn,
                "SELECT transaction_id FROM daily_review_baseline",
                |r| r.get(0),
            )?
            .into_iter()
            .collect(),
            sync_health: conn
                .query_row(
                    "SELECT last_success, last_attempt, failed FROM firefly_sync_health WHERE singleton = 1",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, bool>(2)?,
                        ))
                    },
                )
                .optional()?
                .map(|(s, a, f)| {
                    (
                        s.as_deref().and_then(parse_instant),
                        a.as_deref().and_then(parse_instant),
                        f,
                    )
                })
                .unwrap_or((None, None, false)),
            last_import: conn
                .query_row(
                    "SELECT finished_at FROM import_runs ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .as_deref()
                .and_then(parse_instant),
            category_proposals: crate::review::latest_proposals(conn)?,
            settings,
            zone,
            accounts,
            transactions,
            raw_transactions,
            details,
        })
    }

    pub fn account(&self, account_id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.account_id == account_id)
    }
}

/// `source_key` rows were written by Python's `json.dumps`; compare them in
/// that exact spacing.
fn normalize_key(raw: &str) -> String {
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(parts) if parts.len() == 3 => {
            format!("[\"{}\", \"{}\", \"{}\"]", parts[0], parts[1], parts[2])
        }
        _ => raw.to_owned(),
    }
}

/// An ISO instant with an offset or `Z` (Python `isoformat()` output).
pub fn parse_instant(text: &str) -> Option<Timestamp> {
    Timestamp::from_str(text)
        .ok()
        .or_else(|| Timestamp::from_str(&text.replace(' ', "T")).ok())
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Invalid(format!("stored payload: {e}")))
}

fn rows<T>(
    conn: &Connection,
    sql: &str,
    f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt.query_map([], f)?.collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

fn payloads<T: DeserializeOwned>(conn: &Connection, sql: &str) -> Result<Vec<T>> {
    rows(conn, sql, |r| r.get::<_, String>(0))?
        .iter()
        .map(|p| parse(p))
        .collect()
}

fn optional_payload<T: DeserializeOwned>(conn: &Connection, sql: &str) -> Result<Option<T>> {
    let text: Option<String> = conn.query_row(sql, [], |r| r.get(0)).optional()?;
    text.as_deref().map(parse).transpose()
}

fn decimal(text: &str) -> Result<Decimal> {
    Decimal::from_str(text).map_err(|_| Error::Invalid(format!("amount {text:?}")))
}

fn date(text: &str) -> Result<Date> {
    Date::from_str(&text[..text.len().min(10)])
        .map_err(|_| Error::Invalid(format!("date {text:?}")))
}

const OWN_KINDS: &str = "('asset', 'liability', 'cash')";

fn load_accounts(conn: &Connection) -> Result<Vec<Account>> {
    let mut sums: HashMap<i64, BTreeMap<String, (Decimal, u64)>> = HashMap::new();
    for (account, amount, currency) in rows(
        conn,
        &format!(
            "SELECT p.account_id, p.amount, p.currency FROM postings p \
             JOIN accounts a ON a.id = p.account_id WHERE a.kind IN {OWN_KINDS}"
        ),
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    )? {
        let entry = sums
            .entry(account)
            .or_default()
            .entry(currency)
            .or_default();
        entry.0 += decimal(&amount)?;
        entry.1 += 1;
    }
    let list = rows(
        conn,
        &format!(
            "SELECT id, name, iban, active, include_net_worth FROM accounts \
             WHERE kind IN {OWN_KINDS} ORDER BY id"
        ),
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
            ))
        },
    )?;
    Ok(list
        .into_iter()
        .map(|(id, name, iban, active, include_net_worth)| {
            // An account's currency is the one most of its postings use.
            let per_currency = sums.remove(&id).unwrap_or_default();
            let (code, (amount, _)) = per_currency
                .into_iter()
                .max_by_key(|(_, (_, count))| *count)
                .unwrap_or_else(|| ("EUR".into(), (Decimal::ZERO, 0)));
            Account {
                account_id: format!("firefly:{id}"),
                ledger_id: id,
                name: name.trim().to_owned(),
                balance: UnitMoney {
                    amount: amount.round_dp(Currency::new(&code).minor_unit.into()),
                    currency: Currency::new(&code),
                },
                include_net_worth,
                active,
                iban,
            }
        })
        .collect())
}

fn load_bills(conn: &Connection) -> Result<Vec<Bill>> {
    rows(
        conn,
        "SELECT id, name, currency, amount_min, amount_max, start_date, end_date, repeat_freq, skip, active \
         FROM bills ORDER BY id",
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, u32>(8)?,
                r.get::<_, bool>(9)?,
            ))
        },
    )?
    .into_iter()
    .map(
        |(id, name, currency, min, max, start, end, repeat_freq, skip, active)| {
            Ok(Bill {
                id: id.to_string(),
                name,
                currency: Currency::new(&currency),
                amount_min: decimal(&min)?,
                amount_max: decimal(&max)?,
                start: date(&start)?,
                end: end.as_deref().map(date).transpose()?,
                repeat_freq,
                skip,
                active,
            })
        },
    )
    .collect()
}

struct LedgerAccount {
    name: String,
    iban: Option<String>,
    own: bool,
}

/// One own account's IBAN, with the memo patterns that name it.
struct OwnIban {
    account_id: String,
    /// `IBAN: NL…` anywhere in a memo (the end is checked by hand).
    labelled: Regex,
    /// `(NL…)` in the bank's "Outgoing/Incoming transfer" format.
    bank_format: Regex,
}

/// Own asset accounts by IBAN; an IBAN shared by several accounts is left out.
fn own_iban_index(accounts: &[Account]) -> HashMap<String, OwnIban> {
    let mut matches: HashMap<String, Vec<String>> = HashMap::new();
    for account in accounts {
        if let Some(iban) = account.iban.as_deref().and_then(compact_iban) {
            matches
                .entry(iban)
                .or_default()
                .push(account.account_id.clone());
        }
    }
    matches
        .into_iter()
        .filter(|(_, ids)| ids.len() == 1)
        .filter_map(|(iban, mut ids)| {
            let identifier = iban
                .chars()
                .map(|c| regex::escape(&c.to_string()))
                .collect::<Vec<_>>()
                .join(r"\s*");
            let own = OwnIban {
                account_id: ids.remove(0),
                labelled: Regex::new(&format!(r"(?i)\bIBAN:\s*{identifier}")).ok()?,
                bank_format: Regex::new(&format!(r"(?i)\(\s*{identifier}\s*\)")).ok()?,
            };
            Some((iban, own))
        })
        .collect()
}

/// An own account named by the counterparty IBAN, or by one labelled IBAN in
/// the memo. A name alone is never ownership evidence.
fn owned_counterparty(
    counterparty_iban: Option<&str>,
    description: &str,
    primary: &str,
    ibans: &HashMap<String, OwnIban>,
) -> Option<(String, &'static str)> {
    static TRANSFER_MEMO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(?:Outgoing|Incoming) transfer\b").expect("valid regex")
    });
    if let Some(raw) = counterparty_iban.filter(|r| !r.trim().is_empty()) {
        let compact: String = raw
            .to_uppercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        return ibans
            .get(&compact)
            .filter(|own| own.account_id != primary)
            .map(|own| (own.account_id.clone(), "account_iban"));
    }
    let bank_format = TRANSFER_MEMO.is_match(description);
    let mut found: HashSet<&String> = HashSet::new();
    for own in ibans.values() {
        if own.account_id == primary {
            continue;
        }
        let hit = if bank_format {
            own.bank_format.is_match(description)
        } else {
            // `(?![A-Z0-9])` after the IBAN, checked by hand.
            own.labelled.find_iter(description).any(|m| {
                !description[m.end()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
            })
        };
        if hit {
            found.insert(&own.account_id);
        }
    }
    (found.len() == 1).then(|| {
        (
            found.into_iter().next().cloned().unwrap_or_default(),
            "description_iban",
        )
    })
}

fn load_transactions(conn: &Connection, accounts: &[Account], zone: &TimeZone) -> Result<Vec<Tx>> {
    let ledger_accounts: HashMap<i64, LedgerAccount> = rows(
        conn,
        &format!("SELECT id, name, iban, kind IN {OWN_KINDS} FROM accounts"),
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                LedgerAccount {
                    name: r.get(1)?,
                    iban: r.get(2)?,
                    own: r.get(3)?,
                },
            ))
        },
    )?
    .into_iter()
    .collect();
    let mut legs: HashMap<i64, Vec<(i64, Decimal, String)>> = HashMap::new();
    for (tx, account, amount, currency) in rows(
        conn,
        "SELECT transaction_id, account_id, amount, currency FROM postings ORDER BY id",
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        },
    )? {
        legs.entry(tx)
            .or_default()
            .push((account, decimal(&amount)?, currency));
    }
    let categories: HashMap<i64, String> = rows(conn, "SELECT id, name FROM categories", |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .into_iter()
    .collect();
    let ibans = own_iban_index(accounts);
    let list = rows(
        conn,
        "SELECT id, group_id, kind, booked_at, description, source_account_id, destination_account_id, \
                category_id, bill_id FROM transactions \
         WHERE kind IN ('withdrawal', 'deposit', 'transfer') ORDER BY id",
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        },
    )?;
    let mut out = Vec::with_capacity(list.len());
    for (id, group_id, kind, booked_at, description, source, destination, category, bill) in list {
        let own = |a: i64| ledger_accounts.get(&a).is_some_and(|l| l.own);
        let canonical = |a: i64| format!("firefly:{a}");
        let mut kind = match kind.as_str() {
            "withdrawal" => Kind::Outflow,
            "deposit" => Kind::Inflow,
            _ => Kind::Transfer,
        };
        let (mut account, mut counterparty) = match kind {
            Kind::Inflow => (destination, None),
            Kind::Outflow => (source, None),
            _ => (source, Some(destination)),
        };
        if !own(account) {
            // Firefly's API only listed journals of own accounts.
            continue;
        }
        let mut transfer_evidence = None;
        if kind != Kind::Transfer {
            let opposite = if kind == Kind::Outflow {
                destination
            } else {
                source
            };
            let found = if own(opposite) && opposite != account {
                Some((canonical(opposite), "account_ids"))
            } else {
                owned_counterparty(
                    ledger_accounts
                        .get(&opposite)
                        .and_then(|a| a.iban.as_deref()),
                    &description,
                    &canonical(account),
                    &ibans,
                )
            };
            if let Some((other, evidence)) = found
                && let Some(other_id) = crate::model::ledger_id(&other)
            {
                if kind == Kind::Inflow {
                    counterparty = Some(account);
                    account = other_id;
                } else {
                    counterparty = Some(other_id);
                }
                kind = Kind::Transfer;
                transfer_evidence = Some(evidence);
            }
        }
        let tx_legs = legs.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        let leg = tx_legs
            .iter()
            .find(|(a, _, _)| *a == account)
            .or_else(|| tx_legs.first())
            .ok_or_else(|| Error::Invalid(format!("transaction {id} has no postings")))?;
        let local = DateTime::from_str(&booked_at)
            .map_err(|_| Error::Invalid(format!("booked_at {booked_at:?}")))?;
        let occurred_at: Zoned = local
            .to_zoned(zone.clone())
            .map_err(|_| Error::Invalid(format!("booked_at {booked_at:?}")))?;
        let description = Some(description).filter(|d| !d.is_empty());
        let name_of = |a: i64| ledger_accounts.get(&a).map(|l| l.name.as_str());
        out.push(Tx {
            transaction_id: canonical(id),
            ledger_id: id,
            account_id: canonical(account),
            counterparty_account_id: counterparty.map(canonical),
            kind,
            amount: leg.1.abs(),
            currency: Currency::new(&leg.2),
            purchased_on: card_purchase_date(description.as_deref(), occurred_at.date()),
            occurred_at,
            counterparty_name: match kind {
                Kind::Transfer => None,
                Kind::Inflow => counterparty_name(name_of(source)),
                _ => counterparty_name(name_of(destination)),
            },
            description,
            group_id: group_id.map(|g| g.to_string()),
            category: category.and_then(|c| categories.get(&c).cloned()),
            subscription_id: bill.map(|b| b.to_string()),
            source_country: iban_country(
                ledger_accounts.get(&source).and_then(|a| a.iban.as_deref()),
            ),
            destination_country: iban_country(
                ledger_accounts
                    .get(&destination)
                    .and_then(|a| a.iban.as_deref()),
            ),
            transfer_evidence,
        });
    }
    Ok(out)
}
