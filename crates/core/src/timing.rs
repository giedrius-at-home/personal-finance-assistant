//! When in the day the owner buys things, and the extra bank syncs that
//! follows from it.
//!
//! The banks report dates only (Enable Banking's `booking_date`,
//! `value_date`, `transaction_date`). Two sources give the time anyway:
//! card memos that carry it (Swedbank and ABN AMRO, [`card_purchase_time`]),
//! exact; and when a sync first saw a purchase, pending or booked, which
//! puts it between that bank's previous sync and this one. The estimate
//! sharpens as syncs get denser, which is what the slots below do.
//!
//! The slots: up to three a day, an hour and a half after the busiest
//! buying hours. With the daily sync that makes at most four unattended
//! syncs a day per bank, the usual PSD2 allowance; a rate limit still backs
//! off for six hours ([`crate::bank_sync::next_eligible`]).

use finance_types::bank::{BankRun, PurchaseTiming, SyncStatus};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;
use crate::bank_sync::Entry;
use crate::groups::is_recurring_debit;
use crate::load::Snapshot;
use crate::model::Kind;
use crate::pyfmt::isoformat_utc;
use crate::service::Service;
use crate::text::card_purchase_time;

/// How far back the pattern looks.
const HISTORY_DAYS: i64 = 90;
/// A sighting whose window is longer says too little about the hour.
const MAX_WINDOW_HOURS: i64 = 12;
/// Purchases (exact or estimated) before the owner's own pattern is used.
const ENOUGH: f64 = 15.0;
/// Until then: lunch, after work, evening.
pub const DEFAULT_PEAKS: [u8; 3] = [12, 17, 20];
/// A slot still counts this long after it passed (the 15-minute scheduler
/// may be late, the server may have restarted).
const SLOT_GRACE_HOURS: i64 = 3;

/// Record when this sync first saw each outflow of `downloads` (bank
/// account uid → entries, pending ones included). `since` is the bank's
/// previous successful sync. A booked entry whose pending form was already
/// seen (same account and amount, up to ten days earlier) is not counted
/// twice.
pub fn record_sightings(
    conn: &Connection,
    bank: &str,
    downloads: &[(String, Vec<Entry>)],
    since: Option<Timestamp>,
    now: Timestamp,
) -> Result<u32> {
    let mut added = 0;
    for (uid, entries) in downloads {
        for entry in entries {
            if !entry.amount.is_sign_negative() || entry.amount.is_zero() {
                continue;
            }
            let pending = entry.status == "pending";
            let booked_on = entry
                .date
                .unwrap_or_else(|| now.to_zoned(TimeZone::UTC).date());
            let amount = entry.amount.abs().to_string();
            let key = format!("{}:{uid}:{}", if pending { "p" } else { "b" }, entry.id);
            if !pending {
                let from = booked_on.checked_sub(10.days()).unwrap_or(booked_on);
                let seen_pending: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM purchase_sightings WHERE pending = 1 AND account_uid = ?1 \
                         AND amount = ?2 AND booked_on BETWEEN ?3 AND ?4",
                        params![uid, amount, from.to_string(), booked_on.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                if seen_pending.is_some() {
                    continue;
                }
            }
            let exact = card_purchase_time(Some(&entry.description()), booked_on);
            added += conn.execute(
                "INSERT OR IGNORE INTO purchase_sightings \
                 (entry_key, bank, account_uid, amount, pending, booked_on, seen_at, since, exact_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    key,
                    bank,
                    uid,
                    amount,
                    pending,
                    booked_on.to_string(),
                    isoformat_utc(now),
                    since.map(isoformat_utc),
                    exact.map(|t| t.to_string()),
                ],
            )? as u32;
        }
    }
    Ok(added)
}

/// One sighting without an exact time: the window it happened in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    pub since: Timestamp,
    pub seen_at: Timestamp,
}

/// Sightings of the last 90 days that need an estimate (no exact time).
pub fn sightings(conn: &Connection, now: Timestamp) -> Result<Vec<Sighting>> {
    let from = isoformat_utc(now - SignedDuration::from_hours(24 * HISTORY_DAYS));
    let mut stmt = conn.prepare(
        "SELECT since, seen_at FROM purchase_sightings \
         WHERE exact_at IS NULL AND since IS NOT NULL AND seen_at >= ?1",
    )?;
    let rows = stmt.query_map([from], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (since, seen) = row?;
        if let (Ok(since), Ok(seen_at)) = (since.parse(), seen.parse()) {
            out.push(Sighting { since, seen_at });
        }
    }
    Ok(out)
}

/// Discretionary outflows: no bill, no recurring debit.
fn is_purchase(t: &crate::model::Tx) -> bool {
    t.kind == Kind::Outflow
        && t.subscription_id.is_none()
        && !is_recurring_debit(t.description.as_deref())
}

/// The pattern from the ledger's card memos and the sightings, at `now`.
pub fn purchase_timing(snap: &Snapshot, sightings: &[Sighting], now: Timestamp) -> PurchaseTiming {
    let zone = &snap.zone;
    let today = now.to_zoned(zone.clone()).date();
    let from = today.checked_sub(HISTORY_DAYS.days()).unwrap_or(today);
    let mut hours = [0.0_f64; 24];
    let mut weekdays = [0_u32; 7];
    let mut exact = 0;
    for t in snap.raw_transactions.iter().filter(|t| is_purchase(t)) {
        let day = t.purchased_on.unwrap_or_else(|| t.occurred_at.date());
        if day < from || day > today {
            continue;
        }
        weekdays[day.weekday().to_monday_zero_offset() as usize] += 1;
        if let Some(at) = card_purchase_time(t.description.as_deref(), t.occurred_at.date()) {
            hours[at.hour() as usize] += 1.0;
            exact += 1;
        }
    }
    let mut estimated = 0;
    for s in sightings {
        let window = s.seen_at.duration_since(s.since);
        if window <= SignedDuration::ZERO || window > SignedDuration::from_hours(MAX_WINDOW_HOURS) {
            continue;
        }
        // Twelve evenly spaced points across the window, each a twelfth.
        for i in 0..12 {
            let at = s.since + window * (2 * i + 1) / 24;
            hours[at.to_zoned(zone.clone()).hour() as usize] += 1.0 / 12.0;
        }
        estimated += 1;
    }
    let total: f64 = hours.iter().sum();
    let learned = total >= ENOUGH;
    let peaks = if learned {
        peaks(&hours)
    } else {
        DEFAULT_PEAKS.to_vec()
    };
    PurchaseTiming {
        hours: hours.iter().map(|h| (h * 10.0).round() as u32).collect(),
        weekdays: weekdays.to_vec(),
        exact,
        estimated,
        slots: slot_times(&peaks, today, zone)
            .iter()
            .map(|t| t.to_zoned(zone.clone()).strftime("%H:%M").to_string())
            .collect(),
        peaks,
        learned,
    }
}

/// Up to three busiest hours at least three hours apart, each holding at
/// least a tenth of the purchases; neighbours count half, so a busy stretch
/// beats a lone spike.
pub fn peaks(hours: &[f64; 24]) -> Vec<u8> {
    let total: f64 = hours.iter().sum();
    let smooth: Vec<f64> = (0..24)
        .map(|h| hours[(h + 23) % 24] / 2.0 + hours[h] + hours[(h + 1) % 24] / 2.0)
        .collect();
    let mut chosen: Vec<u8> = Vec::new();
    let mut order: Vec<usize> = (0..24).collect();
    order.sort_by(|a, b| smooth[*b].total_cmp(&smooth[*a]).then(a.cmp(b)));
    for h in order {
        if chosen.len() == 3 || smooth[h] <= 0.0 || smooth[h] < total / 10.0 {
            break;
        }
        let near = chosen.iter().any(|c| {
            let d = (i32::from(*c) - h as i32).rem_euclid(24);
            d.min(24 - d) < 3
        });
        if !near {
            chosen.push(h as u8);
        }
    }
    chosen
}

/// Today's extra syncs: half past the hour after each peak.
pub fn slot_times(peaks: &[u8], today: Date, zone: &TimeZone) -> Vec<Timestamp> {
    let mut out: Vec<Timestamp> = peaks
        .iter()
        .filter_map(|p| {
            let at = today.at(((p + 1) % 24) as i8, 30, 0, 0);
            at.to_zoned(zone.clone()).ok().map(|z| z.timestamp())
        })
        .collect();
    out.sort();
    out
}

/// Whether a bank is due now: the daily sync ([`crate::bank_sync::is_due`]),
/// or a slot that passed since its last attempt while its last sync went
/// well (failures and rate limits keep their own backoff).
pub fn is_due_adaptive(run: &BankRun, now: Timestamp, slots: &[Timestamp]) -> bool {
    if crate::bank_sync::is_due(run, now) {
        return true;
    }
    if run.status != Some(SyncStatus::Success) {
        return false;
    }
    let Some(last) = run
        .last_attempt
        .as_deref()
        .and_then(|s| s.parse::<Timestamp>().ok())
    else {
        return true;
    };
    slots.iter().any(|slot| {
        last < *slot
            && *slot <= now
            && now.duration_since(*slot) <= SignedDuration::from_hours(SLOT_GRACE_HOURS)
    })
}

impl Service {
    /// The purchase pattern and today's extra syncs.
    pub fn purchase_timing(&self) -> std::result::Result<PurchaseTiming, String> {
        let now = Timestamp::now();
        let seen = self.read_store(|c| sightings(c, now))?;
        self.read_snapshot(|snap| purchase_timing(snap, &seen, now))
    }

    /// Today's extra sync times, for the scheduler.
    pub fn sync_slots(&self, now: Timestamp) -> std::result::Result<Vec<Timestamp>, String> {
        let seen = self.read_store(|c| sightings(c, now))?;
        self.read_snapshot(|snap| {
            let timing = purchase_timing(snap, &seen, now);
            slot_times(
                &timing.peaks,
                now.to_zoned(snap.zone.clone()).date(),
                &snap.zone,
            )
        })
    }
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
