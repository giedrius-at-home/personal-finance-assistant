//! The evening's overspend summary: on a day that went over the plan, what
//! pushed it over, and which of today's purchases could have been skipped,
//! each with something to do instead. It goes to the phone (Web Push), into
//! the risk email, and onto Overview → Today.
//!
//! "Over" is per currency: today's spending in groups with a plan limit
//! against what those limits still allow per day (what is left of each this
//! month, spread over the days left, today included). Which purchases are
//! avoidable is the owner's tick per spending group (Manage → Settings → Avoidable spending),
//! defaulting from the group's name: eating out, delivery, coffee, going
//! out, shopping, entertainment; never groceries, rent or bills.

use std::collections::{BTreeMap, HashMap};

use finance_types::notify::{AvoidableGroup, AvoidablePurchase, DayGroup, DaySummary, DayTotal};
use finance_types::report::Currency;
use finance_types::report::Dashboard;
use finance_types::report::allocation::GroupAllocation;
use jiff::{Timestamp, Zoned};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;

use crate::Result;
use crate::groups::is_recurring_debit;
use crate::load::Snapshot;
use crate::model::Kind;
use crate::service::Service;
use crate::spending_email::{claim, complete, latest_due};
use crate::text::{card_purchase_time, casefold};
use crate::{Ctx, budget, plans};

/// The scheduler job (`evaluation_schedule`): once a day at the email hour.
pub const JOB: &str = "day_summary";
/// How many avoidable purchases the summary names.
const SHOWN: usize = 5;

/// Words that make a group avoidable by default, and what to do instead.
/// The first match wins, so the specific come before the general.
const SUGGESTIONS: [(&[&str], &str); 7] = [
    (
        &[
            "deliver",
            "takeaway",
            "take-away",
            "thuisbezorgd",
            "wolt",
            "uber eats",
        ],
        "cooking at home, or picking it up on a walk",
    ),
    (&["coffee", "cafe", "café"], "making coffee at home"),
    (
        &[
            "bar",
            "going out",
            "nightlife",
            "alcohol",
            "drinks",
            "party",
        ],
        "a free evening out: a walk, or friends over at home",
    ),
    (
        &[
            "eating out",
            "restaurant",
            "lunch",
            "dining",
            "food out",
            "fast food",
        ],
        "a packed lunch or cooking at home",
    ),
    (&["snack", "sweets", "candy"], "bringing a snack from home"),
    (
        &[
            "shopping",
            "clothes",
            "clothing",
            "fashion",
            "electronics",
            "gadgets",
            "impulse",
        ],
        "waiting 48 hours and buying it only if you still want it",
    ),
    (
        &[
            "entertainment",
            "games",
            "gaming",
            "fun",
            "hobbies",
            "leisure",
            "cinema",
            "streaming",
        ],
        "a free plan: going out for a walk or to the park",
    ),
];
/// Never avoidable by default, whatever else the name says.
const NEEDED: [&str; 10] = [
    "grocer",
    "rent",
    "bill",
    "utilit",
    "insurance",
    "health",
    "medic",
    "transport",
    "mortgage",
    "child",
];
const FALLBACK: &str = "skipping it, or something free like going out for a walk";

fn suggestion(group: &str) -> Option<&'static str> {
    let name = casefold(group);
    SUGGESTIONS
        .iter()
        .find(|(words, _)| words.iter().any(|w| name.contains(w)))
        .map(|(_, instead)| *instead)
}

/// Whether a group is avoidable when the owner has not said.
pub fn default_avoidable(group: &str) -> bool {
    let name = casefold(group);
    !NEEDED.iter().any(|w| name.contains(w)) && suggestion(group).is_some()
}

/// What to do instead of a purchase in `group`.
pub fn instead(group: &str) -> &'static str {
    suggestion(group).unwrap_or(FALLBACK)
}

/// The category says more than the group ("Coffee" in "Eating out").
fn instead_of(category: Option<&str>, group: &str) -> &'static str {
    category
        .and_then(suggestion)
        .unwrap_or_else(|| instead(group))
}

/// The owner's ticks: group → avoidable.
pub fn load_ticks(conn: &Connection) -> Result<HashMap<String, bool>> {
    let mut stmt = conn.prepare("SELECT spending_group, avoidable FROM avoidable_groups")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Set (or with `None`, clear back to the default) one group's tick.
pub fn save_tick(conn: &Connection, group: &str, avoidable: Option<bool>) -> Result<()> {
    match avoidable {
        Some(v) => conn.execute(
            "INSERT INTO avoidable_groups (spending_group, avoidable) VALUES (?1, ?2)
             ON CONFLICT(spending_group) DO UPDATE SET avoidable = ?2",
            params![group, v],
        )?,
        None => conn.execute(
            "DELETE FROM avoidable_groups WHERE spending_group = ?1",
            [group],
        )?,
    };
    Ok(())
}

pub(crate) fn is_avoidable(ticks: &HashMap<String, bool>, group: &str) -> bool {
    ticks
        .get(group)
        .copied()
        .unwrap_or_else(|| default_avoidable(group))
}

/// Every spending group the owner has, with its tick.
pub fn avoidable_groups(snap: &Snapshot, ticks: &HashMap<String, bool>) -> Vec<AvoidableGroup> {
    let mut names: Vec<String> = snap
        .group_mappings
        .iter()
        .filter_map(|m| m.group.clone().filter(|g| !g.is_empty()))
        .chain(snap.custom_groups.iter().cloned())
        .collect();
    names.sort_by_key(|n| casefold(n));
    names.dedup();
    names
        .into_iter()
        .map(|group| AvoidableGroup {
            avoidable: is_avoidable(ticks, &group),
            default: !ticks.contains_key(&group),
            group,
        })
        .collect()
}

/// This month's group limits per currency, from the dashboard.
pub fn limits(report: &Dashboard) -> Vec<(Currency, Vec<GroupAllocation>)> {
    report
        .monthly_allocation
        .iter()
        .flat_map(|a| a.currencies.iter())
        .map(|row| (row.currency.clone(), row.groups.clone()))
        .collect()
}

/// Today's summary at `now`, against this month's group `limits`.
pub fn day_summary(
    limits: &[(Currency, Vec<GroupAllocation>)],
    snap: &Snapshot,
    now: &Zoned,
    ticks: &HashMap<String, bool>,
) -> DaySummary {
    let ctx = Ctx::new(snap, now);
    let today = ctx.today();
    let next_month = crate::period::shift_month(ctx.current_month(), 1);
    let days_left = Decimal::from(i64::from((next_month - today).get_days()).max(1));
    let group_of = |category: &str| {
        snap.group_mappings
            .iter()
            .filter(|m| m.category == category)
            .filter_map(|m| m.group.as_deref().filter(|g| !g.is_empty()))
            .next_back()
            .unwrap_or("Other")
            .to_owned()
    };
    let spending = budget::spending_transactions(&ctx, &plans::planned_payment_views(&ctx));
    let mut spent: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    let mut avoidable = Vec::new();
    for t in &spending {
        let day = t.purchased_on.unwrap_or_else(|| t.occurred_at.date());
        if t.kind != Kind::Outflow
            || day != today
            || t.subscription_id.is_some()
            || is_recurring_debit(t.description.as_deref())
        {
            continue;
        }
        let category = t.category.as_deref().filter(|c| !c.is_empty());
        let group = category.map_or_else(|| "Other".to_owned(), group_of);
        *spent
            .entry((t.currency.code.clone(), group.clone()))
            .or_default() += t.amount;
        if is_avoidable(ticks, &group) {
            let merchant = t
                .counterparty_name
                .as_deref()
                .or(t.description.as_deref())
                .filter(|m| !m.is_empty())
                .or(category)
                .unwrap_or("A purchase");
            avoidable.push(AvoidablePurchase {
                merchant: merchant.chars().take(60).collect(),
                instead: instead_of(category, &group).to_owned(),
                group,
                amount: t.amount,
                currency: t.currency.clone(),
                time: card_purchase_time(t.description.as_deref(), t.occurred_at.date())
                    .map(|at| at.strftime("%H:%M").to_string()),
            });
        }
    }
    avoidable.sort_by_key(|a| std::cmp::Reverse(a.amount));
    avoidable.truncate(SHOWN);
    let mut totals = Vec::new();
    {
        for (currency, groups) in limits {
            let mut total = DayTotal {
                currency: currency.clone(),
                spent: Decimal::ZERO,
                allowance: Decimal::ZERO,
                over_by: Decimal::ZERO,
                groups: Vec::new(),
            };
            for g in groups {
                let today_spent = spent
                    .get(&(currency.code.clone(), g.group.clone()))
                    .copied()
                    .unwrap_or_default();
                let before = (g.spent - today_spent).max(Decimal::ZERO);
                let allowance = ((g.limit - before).max(Decimal::ZERO) / days_left).round_dp(2);
                total.spent += today_spent;
                total.allowance += allowance;
                if today_spent > allowance {
                    total.groups.push(DayGroup {
                        group: g.group.clone(),
                        spent: today_spent,
                        allowance,
                    });
                }
            }
            total.over_by = (total.spent - total.allowance).max(Decimal::ZERO);
            total
                .groups
                .sort_by_key(|a| std::cmp::Reverse(a.spent - a.allowance));
            if total.spent > Decimal::ZERO {
                totals.push(total);
            }
        }
    }
    DaySummary {
        date: today.to_string(),
        totals,
        avoidable,
    }
}

fn money(amount: Decimal, currency: &Currency) -> String {
    let sign = match currency.code.as_str() {
        "EUR" => "€",
        "USD" => "$",
        "GBP" => "£",
        _ => "",
    };
    let value = amount.round_dp(2);
    if sign.is_empty() {
        format!("{} {value:.2}", currency.code)
    } else {
        format!("{sign}{value:.2}")
    }
}

/// The phone notification, in Rafael's voice (`crate::rafael`).
pub fn push_text(s: &DaySummary) -> (String, String) {
    let day = s.date.parse().unwrap_or_default();
    crate::rafael::day_summary(s, crate::rafael::seed("day", day))
}

/// The summary as plain lines, for the email.
pub fn text_lines(s: &DaySummary) -> Vec<String> {
    let mut lines = Vec::new();
    for t in s.totals.iter().filter(|t| t.over_by > Decimal::ZERO) {
        lines.push(format!(
            "Today: {} spent against {} the plan allows per day, {} over.",
            money(t.spent, &t.currency),
            money(t.allowance, &t.currency),
            money(t.over_by, &t.currency)
        ));
        for g in t.groups.iter().take(3) {
            lines.push(format!(
                "{}: {} today; {} a day is left.",
                g.group,
                money(g.spent, &t.currency),
                money(g.allowance, &t.currency)
            ));
        }
    }
    for p in &s.avoidable {
        lines.push(format!(
            "Could have been avoided: {}{} {} ({}), {} instead.",
            p.merchant,
            p.time
                .as_ref()
                .map(|t| format!(" at {t}"))
                .unwrap_or_default(),
            money(p.amount, &p.currency),
            p.group,
            p.instead
        ));
    }
    lines
}

impl Service {
    /// Today's summary now (Overview → Today).
    pub fn day_summary(&self) -> std::result::Result<DaySummary, String> {
        self.day_summary_at(Timestamp::now())
    }

    pub fn day_summary_at(&self, now: Timestamp) -> std::result::Result<DaySummary, String> {
        self.read_forecast_at(now, |store, snap, zoned, model| {
            let report = crate::dashboard::dashboard(store, snap, zoned, model)?;
            let ticks = load_ticks(store.connection())?;
            Ok(day_summary(&limits(&report), snap, zoned, &ticks))
        })
    }

    pub fn avoidable_groups(&self) -> std::result::Result<Vec<AvoidableGroup>, String> {
        let ticks = self.read_store(load_ticks)?;
        self.read_snapshot(|snap| avoidable_groups(snap, &ticks))
    }

    /// Tick (or untick, or with `None` reset) one group.
    pub fn set_avoidable_group(
        &self,
        group: &str,
        avoidable: Option<bool>,
    ) -> std::result::Result<Vec<AvoidableGroup>, String> {
        let group = group.trim();
        if group.is_empty() || group.chars().count() > 120 {
            return Err("Choose a spending group.".into());
        }
        self.with_connection(|c| save_tick(c, group, avoidable))?;
        self.avoidable_groups()
    }

    /// The scheduler's evening run: once per daily slot at the email hour,
    /// `deliver` gets the summary when the day went over. `Ok(true)` when it
    /// was delivered.
    pub fn run_day_summary(
        &self,
        now: Timestamp,
        deliver: &dyn Fn(&DaySummary) -> bool,
    ) -> std::result::Result<bool, String> {
        let settings = self.email_settings()?;
        let zone = self.read_snapshot(|s| s.zone.clone())?;
        let due = latest_due(now, &zone, settings.hour);
        if !self.with_connection(|c| claim(c, JOB, now, due))? {
            return Ok(false);
        }
        let summary = self.day_summary_at(now)?;
        let sent = summary.over() && deliver(&summary);
        self.with_connection(|c| complete(c, JOB, now))?;
        Ok(sent)
    }
}

#[cfg(test)]
#[path = "day_summary_tests.rs"]
mod tests;
