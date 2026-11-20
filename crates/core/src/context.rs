//! What one report build shares: the snapshot, the moment it is built for
//! and the closure-aware month resolver.

use jiff::Zoned;
use jiff::civil::Date;

use crate::Snapshot;
use crate::period::PeriodResolver;

#[derive(Clone, Debug)]
pub struct Ctx<'a> {
    pub snap: &'a Snapshot,
    /// The build moment, in the owner's time zone.
    pub now: Zoned,
    pub resolver: PeriodResolver,
    /// The events, which own the spending of their days; set by the
    /// builds that read the wishes.
    pub events: Vec<crate::events::EventClaim>,
}

impl<'a> Ctx<'a> {
    pub fn new(snap: &'a Snapshot, now: &Zoned) -> Self {
        let resolver = PeriodResolver::new(
            &snap.month_periods,
            snap.first_observed.clone(),
            snap.zone.clone(),
        );
        Self {
            snap,
            now: now.with_time_zone(snap.zone.clone()),
            resolver,
            events: Vec::new(),
        }
    }

    /// Today in the owner's time zone.
    pub fn today(&self) -> Date {
        self.now.date()
    }

    /// The budget month that is running now (after an early cutoff, the next one).
    pub fn current_month(&self) -> Date {
        self.resolver.current(&self.now)
    }
}
