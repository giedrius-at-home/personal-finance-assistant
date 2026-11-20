//! Report section types for events: wishes with dates that own the
//! day-to-day spending of their days.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;
use super::wishes::{WishMonth, WishRow};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase {
    #[default]
    Upcoming,
    Running,
    Past,
}

/// Spending on the event's days against the days gone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    #[default]
    OnTrack,
    Fast,
    Over,
}

/// One calendar month an event touches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMonth {
    /// `YYYY-MM-01`.
    pub month: String,
    /// The money for the days that falls in this month.
    pub share: Decimal,
    pub spent: Decimal,
    /// What the groups usually spend on these days, handed to the event.
    pub usual: Decimal,
}

/// A transaction the event owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTransaction {
    pub transaction_id: String,
    pub description: String,
    /// `YYYY-MM-DD`.
    pub on: String,
    /// Negative for a refund.
    pub amount: Decimal,
    /// Picked in by hand rather than by its date.
    pub picked: bool,
    /// The bought part this payment is.
    pub part_id: Option<String>,
}

/// What an event has spent, from the ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSpend {
    pub wish_id: String,
    /// Payments picked in from before the first day (not a bought part's).
    pub picked_ahead: Decimal,
    /// `(YYYY-MM-01, spent)` from the first day on, by budget month,
    /// without the payments that are a bought part's.
    pub by_month: Vec<(String, Decimal)>,
    /// Every transaction the event owns, newest first.
    pub transactions: Vec<EventTransaction>,
}

/// An event with what it costs, what it has spent and how it is going.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRow {
    /// The wish, its parts and the ladder's verdict.
    pub row: WishRow,
    pub phase: EventPhase,
    /// What the groups usually spend on the event's days.
    pub usual: Decimal,
    /// The money for the days above the usual: what the budget must find.
    pub extra: Decimal,
    /// Bought parts and payments picked in from before the first day.
    pub paid_ahead: Decimal,
    pub spent: Decimal,
    /// The planned amount less the parts still to buy and what is spent.
    pub left: Decimal,
    pub days: u32,
    /// Today included; zero before and after.
    pub days_left: u32,
    pub per_day: Decimal,
    pub pace: Pace,
    /// The largest amount that needs no trade-off; `None` when not judged.
    pub fits_up_to: Option<Decimal>,
    pub months: Vec<EventMonth>,
    /// Newest first.
    pub transactions: Vec<EventTransaction>,
}

/// The events screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsView {
    pub currency: Currency,
    /// Running first, then upcoming by start, then past, newest first.
    pub events: Vec<EventRow>,
    pub months: Vec<WishMonth>,
}
