//! Weekly report, net worth, monthly spending and taxonomy hints (Python domain/reporting.py).

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::LazyLock;

use finance_types::gateway::SavingsGoalRecord;
use finance_types::report::reporting::{
    AlertCount, AlertSeverity, CurrencyAmount, CurrencyWeek, NetWorthComponent, NetWorthReport,
    TaxonomyRecommendation, TaxonomySignal, WeekCategory, WeeklyReport,
};
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::Result;
use crate::goals::evaluate_goal;
use crate::load::parse_instant;
use crate::model::{Account, Kind, Tx};
use crate::text::casefold;

pub const REPORT_VERSION: &str = "2026-09-v1";

/// Python's decimal context keeps 28 significant digits.
const PRECISION: u32 = 28;

fn digit_count(mut n: u128) -> u32 {
    let mut count = 0;
    while n > 0 {
        n /= 10;
        count += 1;
    }
    count
}

/// `d` rounded to 28 significant digits (half-even), as Python's decimal
/// context rounds every arithmetic result.
pub fn py_round(d: Decimal) -> Decimal {
    let excess = digit_count(d.mantissa().unsigned_abs()).saturating_sub(PRECISION);
    if excess == 0 || excess > d.scale() {
        return d;
    }
    d.round_dp_with_strategy(d.scale() - excess, RoundingStrategy::MidpointNearestEven)
}

/// `a / b` exactly as Python's `Decimal` divides: 28 significant digits,
/// half-even, and exact quotients reduced to the ideal exponent. Division by
/// zero (an error in Python) gives zero.
pub fn py_div(a: Decimal, b: Decimal) -> Decimal {
    if b.is_zero() {
        return Decimal::ZERO;
    }
    if a.is_zero() {
        // Python keeps the ideal exponent: `Decimal("0.00") / 7` is `0.00`.
        return Decimal::new(0, a.scale().saturating_sub(b.scale()));
    }
    let negative = a.is_sign_negative() != b.is_sign_negative();
    let divisor = b.mantissa().unsigned_abs();
    let ideal_exp = i64::from(b.scale()) - i64::from(a.scale());
    let mut exp = ideal_exp;
    let mut digits = a.mantissa().unsigned_abs() / divisor;
    let mut rest = a.mantissa().unsigned_abs() % divisor;
    while digit_count(digits) < PRECISION {
        rest *= 10;
        digits = digits * 10 + rest / divisor;
        rest %= divisor;
        exp -= 1;
    }
    // Round to the precision; `rest` (over `divisor`) is what lies below.
    let excess = digit_count(digits) - PRECISION;
    let (dropped, unit) = if excess > 0 {
        let unit = 10u128.pow(excess);
        let dropped = digits % unit;
        digits /= unit;
        exp += i64::from(excess);
        (dropped, unit)
    } else {
        (0, 1)
    };
    let exact = dropped == 0 && rest == 0;
    let above_half = if excess > 0 {
        (dropped * 2).cmp(&unit).then(if rest > 0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        })
    } else {
        (rest * 2).cmp(&divisor)
    };
    if above_half.is_gt() || (above_half.is_eq() && digits % 2 == 1) {
        digits += 1;
        if digit_count(digits) > PRECISION {
            digits /= 10;
            exp += 1;
        }
    }
    if exact {
        while exp < ideal_exp && digits.is_multiple_of(10) {
            digits /= 10;
            exp += 1;
        }
    }
    let magnitude = if exp >= 0 {
        let factor = Decimal::from_i128_with_scale(10i128.pow(exp as u32), 0);
        Decimal::from_i128_with_scale(digits as i128, 0)
            .checked_mul(factor)
            .unwrap_or(Decimal::MAX)
    } else {
        let mut scale = u32::try_from(-exp).unwrap_or(u32::MAX);
        // rust_decimal holds at most 28 decimal places.
        while scale > 28 {
            let last = digits % 10;
            digits /= 10;
            if last > 5 || (last == 5 && digits % 2 == 1) {
                digits += 1;
            }
            scale -= 1;
        }
        Decimal::from_i128_with_scale(digits as i128, scale)
    };
    if negative { -magnitude } else { magnitude }
}

/// Report inputs that live in the old alerting and review tables rather than
/// in the ledger. Missing tables read as empty.
#[derive(Clone, Debug, Default)]
pub struct ReportInputs {
    /// Severities of active alerts, newest first (at most 100).
    pub alert_severities: Vec<AlertSeverity>,
    /// The latest safe-to-spend amount of each budget.
    pub safe_to_spend: Vec<UnitMoney>,
    /// Approved categorisation proposals: ledger transaction id → category.
    pub approved_categories: HashMap<i64, String>,
    /// When the ledger was last refreshed from the bank or Firefly.
    pub refreshed_at: Option<Timestamp>,
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

impl ReportInputs {
    /// `sync_success` is the snapshot's last successful refresh, used when
    /// the ledger has no import run.
    pub fn load(conn: &Connection, sync_success: Option<Timestamp>) -> Result<Self> {
        let mut inputs = Self::default();
        if has_table(conn, "alert_lifecycle")? {
            let mut stmt = conn.prepare(
                "SELECT severity FROM alert_lifecycle WHERE active = 1 \
                 ORDER BY updated_at DESC LIMIT 100",
            )?;
            inputs.alert_severities = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .filter_map(|s| AlertSeverity::parse(s))
                .collect();
        }
        if has_table(conn, "alert_events")? {
            let mut stmt = conn.prepare(
                "SELECT evidence_json FROM alert_events WHERE event_id IN ( \
                 SELECT MAX(event_id) FROM alert_events WHERE subject_kind = 'budget' \
                 GROUP BY subject_id) ORDER BY subject_id",
            )?;
            inputs.safe_to_spend = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .filter_map(|json| {
                    let value: serde_json::Value = serde_json::from_str(json).ok()?;
                    let amount = match value.get("safe_to_spend")? {
                        serde_json::Value::String(s) => Decimal::from_str(s).ok()?,
                        other => Decimal::from_str(&other.to_string()).ok()?,
                    };
                    let currency: Currency =
                        serde_json::from_value(value.get("currency")?.clone()).ok()?;
                    Some(UnitMoney { amount, currency })
                })
                .collect();
        }
        if has_table(conn, "category_proposals")? {
            let mut stmt = conn.prepare(
                "SELECT status, source_system, source_entity, external_id, \
                 json_extract(payload_json, '$.proposed_category') \
                 FROM category_proposals ORDER BY created_at DESC LIMIT 500",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Newest first, later rows overwrite: the oldest approval wins, as
            // the Python dict comprehension had it.
            for (status, system, entity, external_id, category) in rows {
                if status != "approved" || system != "firefly_iii" || entity != "transaction" {
                    continue;
                }
                if let (Some(category), Ok(id)) = (category, external_id.parse()) {
                    inputs.approved_categories.insert(id, category);
                }
            }
        }
        let last_import: Option<String> = if has_table(conn, "import_runs")? {
            conn.query_row(
                "SELECT finished_at FROM import_runs ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
        inputs.refreshed_at = last_import
            .as_deref()
            .and_then(parse_instant)
            .into_iter()
            .chain(sync_success)
            .max();
        Ok(inputs)
    }
}

/// The category each transaction counts under: its own, else an approved
/// proposal's. Keyed by `transaction_id`.
pub fn assignments(
    transactions: &[Tx],
    approved: &HashMap<i64, String>,
) -> HashMap<String, String> {
    transactions
        .iter()
        .filter_map(|t| {
            let category = t
                .category
                .clone()
                .or_else(|| approved.get(&t.ledger_id).cloned())?;
            Some((t.transaction_id.clone(), category))
        })
        .collect()
}

/// One local ISO week (Monday through today), per currency, never converted.
pub fn weekly_report(
    transactions: &[Tx],
    assignments: &HashMap<String, String>,
    goals: &[SavingsGoalRecord],
    safe_to_spend: &[UnitMoney],
    alert_severities: &[AlertSeverity],
    now: &Zoned,
) -> WeeklyReport {
    let zone = now.time_zone().clone();
    let today = now.date();
    let start = week_start(today);
    let end = start.checked_add(6.days()).unwrap_or(start).min(today);

    #[derive(Default)]
    struct Week {
        income: Decimal,
        spending: Decimal,
        transfers: u64,
        categories: HashMap<String, (Decimal, u64)>,
    }
    let mut weeks: BTreeMap<Currency, Week> = BTreeMap::new();
    let mut included = 0;
    for t in transactions {
        let on = t.occurred_at.with_time_zone(zone.clone()).date();
        if on < start || on > end {
            continue;
        }
        included += 1;
        let week = weeks.entry(t.currency.clone()).or_default();
        let signed = match t.kind {
            Kind::Transfer => {
                week.transfers += 1;
                continue;
            }
            Kind::Inflow => {
                week.income += t.amount;
                continue;
            }
            Kind::Refund => -t.amount,
            Kind::Outflow => t.amount,
        };
        week.spending += signed;
        let category = assignments
            .get(&t.transaction_id)
            .cloned()
            .unwrap_or_else(|| "Uncategorised".into());
        let entry = week.categories.entry(category).or_default();
        entry.0 += signed;
        entry.1 += 1;
    }
    let currencies = weeks
        .into_iter()
        .map(|(currency, week)| {
            let mut categories: Vec<WeekCategory> = week
                .categories
                .into_iter()
                .map(|(category, (amount, count))| WeekCategory {
                    category,
                    amount,
                    transaction_count: count,
                })
                .collect();
            categories.sort_by(|a, b| {
                b.amount
                    .cmp(&a.amount)
                    .then_with(|| casefold(&a.category).cmp(&casefold(&b.category)))
            });
            CurrencyWeek {
                currency,
                income: week.income,
                spending: week.spending,
                net_cash_flow: week.income - week.spending,
                transfer_count: week.transfers,
                categories,
            }
        })
        .collect();

    // The most conservative amount per currency.
    let mut safe: BTreeMap<Currency, Decimal> = BTreeMap::new();
    for money in safe_to_spend.iter().filter(|m| m.amount >= Decimal::ZERO) {
        safe.entry(money.currency.clone())
            .and_modify(|a| *a = (*a).min(money.amount))
            .or_insert(money.amount);
    }
    let mut alerts: BTreeMap<&'static str, (AlertSeverity, u64)> = BTreeMap::new();
    for severity in alert_severities
        .iter()
        .filter(|s| **s != AlertSeverity::None)
    {
        alerts.entry(severity.as_str()).or_insert((*severity, 0)).1 += 1;
    }
    let mut goals: Vec<_> = goals
        .iter()
        .map(|g| evaluate_goal(&g.goal, today))
        .collect();
    goals.sort_by(|a, b| a.goal_id.cmp(&b.goal_id));
    WeeklyReport {
        report_version: REPORT_VERSION.into(),
        basis: "observed".into(),
        timezone: zone.iana_name().unwrap_or("UTC").into(),
        week_start: start.to_string(),
        through_date: end.to_string(),
        complete_week: start.checked_add(6.days()).is_ok_and(|last| today >= last),
        currencies,
        safe_to_spend: safe
            .into_iter()
            .map(|(currency, amount)| CurrencyAmount { currency, amount })
            .collect(),
        active_alerts: alerts
            .into_values()
            .map(|(severity, count)| AlertCount { severity, count })
            .collect(),
        goals,
        source_transaction_count: included,
    }
}

/// How old a balance may be before it no longer counts toward a total.
pub const STALE_AFTER: SignedDuration = SignedDuration::from_hours(48);

/// Every included account once, with freshness. `refreshed_at` is when the
/// ledger was last refreshed: the new app refreshes all accounts together.
pub fn net_worth_report(
    accounts: &[Account],
    now: &Zoned,
    refreshed_at: Option<Timestamp>,
) -> NetWorthReport {
    let mut reasons = Vec::new();
    let mut included: Vec<&Account> = accounts.iter().filter(|a| a.include_net_worth).collect();
    let excluded = accounts.len() - included.len();
    if excluded > 0 {
        let label = if excluded == 1 { "account" } else { "accounts" };
        reasons.push(format!(
            "source settings exclude {excluded} {label} from net worth"
        ));
    }
    included.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    let stale = refreshed_at.is_none_or(|at| now.timestamp().duration_since(at) > STALE_AFTER);
    let mut totals: BTreeMap<Currency, Decimal> = BTreeMap::new();
    let components: Vec<NetWorthComponent> = included
        .iter()
        .map(|a| {
            *totals.entry(a.currency().clone()).or_default() += a.balance.amount;
            NetWorthComponent {
                account_id: a.account_id.clone(),
                label: a.name.clone(),
                owner: "firefly_iii".into(),
                currency: a.currency().clone(),
                amount: a.balance.amount,
                updated_at: refreshed_at.map(|t| t.to_string()),
                stale,
                excluded_mirror_count: 0,
            }
        })
        .collect();
    let currency_totals: Vec<CurrencyAmount> = totals
        .into_iter()
        .map(|(currency, amount)| CurrencyAmount { currency, amount })
        .collect();
    if stale && !components.is_empty() {
        reasons.push("stale valuations prevent a combined total".into());
    }
    if currency_totals.len() > 1 {
        reasons.push("mixed currencies require an explicit valuation policy".into());
    }
    let single = (currency_totals.len() == 1 && !components.iter().any(|c| c.stale))
        .then(|| currency_totals[0].clone());
    NetWorthReport {
        report_version: REPORT_VERSION.into(),
        as_of: now
            .timestamp()
            .to_zoned(now.time_zone().clone())
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
        components,
        currency_totals,
        total: single.as_ref().map(|c| c.amount),
        total_currency: single.map(|c| c.currency),
        reasons,
    }
}

/// Twelve-month evidence for one category: confirmed expenses, refunds netted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryAggregate {
    pub category: String,
    pub transaction_count: u64,
    /// Never negative.
    pub amount: Decimal,
}

fn python_casefold_sorted<T>(items: &mut [T], key: impl Fn(&T) -> &str) {
    items.sort_by_key(|a| casefold(key(a)));
}

/// Expense evidence per category over the year before `now`.
pub fn taxonomy_aggregates(
    transactions: &[Tx],
    assignments: &HashMap<String, String>,
    now: &Zoned,
) -> Vec<CategoryAggregate> {
    let end = now.timestamp();
    let start = end - SignedDuration::from_hours(365 * 24);
    let mut by_category: HashMap<String, (u64, Decimal)> = HashMap::new();
    for t in transactions {
        let at = t.occurred_at.timestamp();
        if !matches!(t.kind, Kind::Outflow | Kind::Refund) || at < start || at > end {
            continue;
        }
        let Some(category) = t
            .category
            .clone()
            .or_else(|| assignments.get(&t.transaction_id).cloned())
        else {
            continue;
        };
        let entry = by_category.entry(category).or_default();
        entry.0 += 1;
        entry.1 += if t.kind == Kind::Refund {
            -t.amount
        } else {
            t.amount
        };
    }
    let mut out: Vec<CategoryAggregate> = by_category
        .into_iter()
        .map(|(category, (count, amount))| CategoryAggregate {
            category,
            transaction_count: count,
            amount: amount.max(Decimal::ZERO),
        })
        .collect();
    // Stable on exact names first so case-folded ties keep one order.
    out.sort_by(|a, b| a.category.cmp(&b.category));
    python_casefold_sorted(&mut out, |a| &a.category);
    out
}

/// Sum aggregates that name the same category.
pub fn merge_aggregates(aggregates: &[CategoryAggregate]) -> Vec<CategoryAggregate> {
    let mut merged: Vec<CategoryAggregate> = Vec::new();
    for item in aggregates {
        match merged.iter_mut().find(|m| m.category == item.category) {
            Some(m) => {
                m.transaction_count += item.transaction_count;
                m.amount += item.amount;
            }
            None => merged.push(item.clone()),
        }
    }
    python_casefold_sorted(&mut merged, |a| &a.category);
    merged
}

/// Aggregate-only review hints; the vocabulary itself is never changed.
/// `aggregates` must name each category once (see [`merge_aggregates`]).
pub fn taxonomy_recommendations(aggregates: &[CategoryAggregate]) -> Vec<TaxonomyRecommendation> {
    static NON_KEY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("valid regex"));
    let mut ordered: Vec<&CategoryAggregate> = aggregates.iter().collect();
    python_casefold_sorted(&mut ordered, |a| &a.category);
    let mut result = Vec::new();
    for a in ordered.iter().filter(|a| a.transaction_count <= 1) {
        result.push(TaxonomyRecommendation {
            signal: TaxonomySignal::Sparse,
            categories: vec![a.category.clone()],
            reason: "At most one transaction uses this category.".into(),
        });
    }
    let mut normalized: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for a in &ordered {
        let key = NON_KEY.replace_all(&casefold(&a.category), "").into_owned();
        normalized.entry(key).or_default().push(a.category.clone());
    }
    for mut group in normalized.into_values().filter(|g| g.len() > 1) {
        group.sort();
        result.push(TaxonomyRecommendation {
            signal: TaxonomySignal::DuplicateLike,
            categories: group,
            reason: "Category labels normalize to the same value.".into(),
        });
    }
    let total: u64 = aggregates.iter().map(|a| a.transaction_count).sum();
    for a in &ordered {
        if total > 0
            && a.transaction_count >= 20
            && py_div(Decimal::from(a.transaction_count), Decimal::from(total))
                >= Decimal::new(50, 2)
        {
            result.push(TaxonomyRecommendation {
                signal: TaxonomySignal::Overloaded,
                categories: vec![a.category.clone()],
                reason: "At least half of categorized transactions use this label.".into(),
            });
        }
    }
    result
}

/// Day of the local week's Monday, for callers that need the same window.
pub fn week_start(today: Date) -> Date {
    today
        .checked_sub(i64::from(today.weekday().to_monday_zero_offset()).days())
        .unwrap_or(today)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn zoned(text: &str) -> Zoned {
        text.parse().unwrap()
    }

    pub(crate) fn tx(id: &str, kind: Kind, amount: &str, currency: &str, at: &str) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 0,
            account_id: "firefly:1".into(),
            counterparty_account_id: (kind == Kind::Transfer).then(|| "firefly:2".into()),
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::new(currency),
            occurred_at: zoned(at),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: None,
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    const AT: &str = "2026-09-02T14:00:00+02:00[Europe/Amsterdam]";

    #[test]
    fn python_division() {
        let d = |s: &str| Decimal::from_str(s).unwrap();
        assert_eq!(
            py_div(d("80"), d("87")).to_string(),
            "0.9195402298850574712643678161"
        );
        assert_eq!(py_div(d("4200"), d("60")).to_string(), "70");
        assert_eq!(
            py_div(d("1"), d("3")).to_string(),
            "0.3333333333333333333333333333"
        );
        assert_eq!(
            py_div(d("-2"), d("3")).to_string(),
            "-0.6666666666666666666666666667"
        );
        assert_eq!(
            py_div(d("1130.09"), d("4488.36")),
            py_div(d("113009"), d("448836"))
        );
        assert_eq!(
            py_div(d("113009"), d("4488.36")).to_string(),
            "25.17823882219786291652184762"
        );
    }

    #[test]
    fn weekly_separates_currencies_transfers_and_refunds() {
        let rows = vec![
            tx("income", Kind::Inflow, "100", "EUR", AT),
            tx("spend", Kind::Outflow, "30", "EUR", AT),
            tx("refund", Kind::Refund, "5", "EUR", AT),
            tx("transfer", Kind::Transfer, "20", "EUR", AT),
            tx("usd", Kind::Outflow, "7", "USD", AT),
        ];
        let money = |a: &str| UnitMoney {
            amount: a.parse().unwrap(),
            currency: Currency::eur(),
        };
        let now = zoned(AT);
        let report = weekly_report(
            &rows,
            &HashMap::new(),
            &[],
            &[money("60"), money("40")],
            &[
                AlertSeverity::Warning,
                AlertSeverity::Critical,
                AlertSeverity::None,
            ],
            &now,
        );
        let flows: Vec<_> = report
            .currencies
            .iter()
            .map(|c| (c.currency.code.as_str(), c.net_cash_flow))
            .collect();
        assert_eq!(
            flows,
            vec![("EUR", Decimal::from(75)), ("USD", Decimal::from(-7))]
        );
        assert_eq!(report.currencies[0].transfer_count, 1);
        assert_eq!(report.safe_to_spend[0].amount, Decimal::from(40));
        assert_eq!(report.week_start, "2026-08-31");
        assert_eq!(report.active_alerts[0].severity, AlertSeverity::Critical);
        let mut reversed = rows.clone();
        reversed.reverse();
        let again = weekly_report(
            &reversed,
            &HashMap::new(),
            &[],
            &[money("40"), money("60")],
            &[AlertSeverity::Critical, AlertSeverity::Warning],
            &now,
        );
        assert_eq!(report, again);
    }

    fn account(id: &str, include: bool) -> Account {
        Account {
            account_id: id.into(),
            ledger_id: 1,
            name: id.into(),
            balance: UnitMoney {
                amount: Decimal::from(1000),
                currency: Currency::eur(),
            },
            include_net_worth: include,
            active: true,
            iban: None,
        }
    }

    #[test]
    fn net_worth_fails_closed_when_stale() {
        let now = zoned(AT);
        let old = now.timestamp() - SignedDuration::from_hours(72);
        let report = net_worth_report(&[account("a", true)], &now, Some(old));
        assert_eq!(report.currency_totals[0].amount, Decimal::from(1000));
        assert_eq!(report.total, None);
        assert_eq!(
            report.reasons,
            vec!["stale valuations prevent a combined total"]
        );
        let fresh = net_worth_report(&[account("a", true)], &now, Some(now.timestamp()));
        assert_eq!(fresh.total, Some(Decimal::from(1000)));
        assert_eq!(fresh.as_of, "2026-09-02T14:00:00+02:00");
    }

    #[test]
    fn net_worth_honours_exclusion() {
        let now = zoned(AT);
        let report = net_worth_report(&[account("x", false)], &now, Some(now.timestamp()));
        assert!(report.components.is_empty());
        assert_eq!(report.total, None);
        assert_eq!(
            report.reasons,
            vec!["source settings exclude 1 account from net worth"]
        );
    }

    #[test]
    fn taxonomy_signals_are_deterministic() {
        let agg = |c: &str, n: u64| CategoryAggregate {
            category: c.into(),
            transaction_count: n,
            amount: Decimal::from(10),
        };
        let aggregates = vec![agg("Dining out", 20), agg("Dining-out", 1)];
        let signals = taxonomy_recommendations(&aggregates);
        let kinds: Vec<_> = signals.iter().map(|s| s.signal).collect();
        assert_eq!(
            kinds,
            vec![
                TaxonomySignal::Sparse,
                TaxonomySignal::DuplicateLike,
                TaxonomySignal::Overloaded
            ]
        );
        let mut reversed = aggregates.clone();
        reversed.reverse();
        assert_eq!(signals, taxonomy_recommendations(&reversed));
        let merged = merge_aggregates(&[agg("Food", 2), agg("Food", 3)]);
        assert_eq!(merged[0].transaction_count, 5);
    }
}
