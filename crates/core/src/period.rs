//! Budget months that respect month closures (Python
//! `domain/month_closure.py`: `period_for`, `period_start`, `PeriodResolver`).

use std::collections::HashMap;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use finance_types::report::closure::MonthPeriod;

use crate::load::parse_instant;
use crate::model::Tx;

/// `month` moved by `offset` months; always the first of the month.
/// The budget month a plan due on `day` falls in, seen from `month`: a day
/// after an early month close (`start`, the first day of `month`'s period)
/// still in the calendar month before belongs to `month`.
pub fn plan_month(day: Date, month: Date, start: Option<Date>) -> Date {
    match start {
        Some(start) if start <= day && day < month => month,
        _ => day.first_of_month(),
    }
}

pub fn shift_month(month: Date, offset: i32) -> Date {
    let index = i32::from(month.year()) * 12 + i32::from(month.month()) - 1 + offset;
    Date::new(
        i16::try_from(index.div_euclid(12)).unwrap_or(9999),
        i8::try_from(index.rem_euclid(12) + 1).unwrap_or(1),
        1,
    )
    .unwrap_or(month)
}

pub fn month_start(day: Date) -> Date {
    day.first_of_month()
}

/// The cutoff of a closed month: later bookings belong to the next month.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cutoff {
    pub month: Date,
    pub cutoff_at: Option<Timestamp>,
    /// `cutoff_at` as stored, so a derived period start keeps its spelling.
    pub stored: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PeriodResolver {
    cutoffs: Vec<Cutoff>,
    observed: HashMap<String, Timestamp>,
    zone: TimeZone,
}

impl PeriodResolver {
    /// `periods` are the stored month closures.
    pub fn new(
        periods: &[MonthPeriod],
        observed: HashMap<String, Timestamp>,
        zone: TimeZone,
    ) -> Self {
        let cutoffs = periods
            .iter()
            .filter_map(|p| {
                let month: Date = p.month.parse().ok()?;
                let cutoff_at = p.cutoff_at.as_deref().and_then(parse_instant);
                Some(Cutoff {
                    month,
                    cutoff_at,
                    stored: p.cutoff_at.clone().filter(|_| cutoff_at.is_some()),
                })
            })
            .collect();
        Self {
            cutoffs,
            observed,
            zone,
        }
    }

    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// The first closure of `month` that has a cutoff.
    fn closed(&self, month: Date) -> Option<&Cutoff> {
        self.cutoffs
            .iter()
            .find(|c| c.month == month && c.cutoff_at.is_some())
    }

    fn cutoff(&self, month: Date) -> Option<Timestamp> {
        self.closed(month).and_then(|c| c.cutoff_at)
    }

    /// The booking date's calendar month, unless that month is closed and the
    /// booking falls after its cutoff (or on the cutoff day, first observed
    /// at or after the cutoff) — then the next month.
    pub fn period_for(&self, booking: Date, observed_at: Option<Timestamp>) -> Date {
        let calendar = month_start(booking);
        let Some(cutoff_at) = self.cutoff(calendar) else {
            return calendar;
        };
        let cutoff_day = cutoff_at.to_zoned(self.zone.clone()).date();
        if booking > cutoff_day
            || (booking == cutoff_day && observed_at.is_some_and(|o| o >= cutoff_at))
        {
            shift_month(calendar, 1)
        } else {
            calendar
        }
    }

    pub fn month_of(&self, t: &Tx) -> Date {
        self.period_for(
            t.effective_on(),
            self.observed.get(&t.transaction_id).copied(),
        )
    }

    pub fn current(&self, now: &Zoned) -> Date {
        let local = now.with_time_zone(self.zone.clone());
        self.period_for(local.date(), Some(now.timestamp()))
    }

    /// When `month` starts: midnight local, or earlier if the month before
    /// was closed early.
    pub fn period_start(&self, month: Date) -> Zoned {
        let calendar = month
            .to_zoned(self.zone.clone())
            .unwrap_or_else(|_| Zoned::now().with_time_zone(self.zone.clone()));
        match self.cutoff(shift_month(month, -1)) {
            Some(cutoff) if cutoff < calendar.timestamp() => cutoff.to_zoned(self.zone.clone()),
            _ => calendar,
        }
    }

    /// [`PeriodResolver::period_start`] as the Python service serialized it:
    /// the local midnight with its offset, or the stored early cutoff.
    pub fn period_start_stamp(&self, month: Date) -> String {
        let start = self.period_start(month);
        match self.closed(shift_month(month, -1)) {
            Some(c) if c.cutoff_at == Some(start.timestamp()) => {
                c.stored.clone().unwrap_or_default()
            }
            _ => start.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        }
    }

    /// `period_start` as a local date.
    pub fn period_start_date(&self, month: Date) -> Date {
        self.period_start(month).date()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use std::collections::BTreeMap;

    fn resolver() -> PeriodResolver {
        let periods = vec![MonthPeriod {
            balance_checks: BTreeMap::new(),
            month: "2026-09-01".into(),
            starts_at: "2026-09-01T00:00:00+02:00".into(),
            cutoff_at: Some("2026-09-28T10:00:00Z".into()),
            status: Default::default(),
            settled_at: None,
            settled_snapshot: None,
            missing_banks: Vec::new(),
            allocations: Vec::new(),
            revision: 0,
            reviewed_through: None,
            reopened_at: None,
        }];
        PeriodResolver::new(
            &periods,
            HashMap::new(),
            TimeZone::get("Europe/Amsterdam").unwrap(),
        )
    }

    #[test]
    fn shifts_across_years() {
        assert_eq!(shift_month(date(2026, 11, 1), 2), date(2027, 1, 1));
        assert_eq!(shift_month(date(2026, 1, 1), -1), date(2025, 12, 1));
    }

    #[test]
    fn early_cutoff_moves_later_bookings() {
        let r = resolver();
        assert_eq!(r.period_for(date(2026, 9, 27), None), date(2026, 9, 1));
        assert_eq!(r.period_for(date(2026, 9, 29), None), date(2026, 10, 1));
        let after: Timestamp = "2026-09-28T11:00:00Z".parse().unwrap();
        assert_eq!(
            r.period_for(date(2026, 9, 28), Some(after)),
            date(2026, 10, 1)
        );
        assert_eq!(r.period_for(date(2026, 9, 28), None), date(2026, 9, 1));
        assert_eq!(r.period_start_date(date(2026, 10, 1)), date(2026, 9, 28));
        assert_eq!(
            r.period_start_stamp(date(2026, 10, 1)),
            "2026-09-28T10:00:00Z"
        );
        assert_eq!(
            r.period_start_stamp(date(2026, 11, 1)),
            "2026-11-01T00:00:00+01:00"
        );
    }
}
