//! The wishes screen's report: each month's free money, the card, and a
//! verdict per planned purchase.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::gateway::{Wish, WishSettings};
use crate::report::Currency;

/// A budget group trimmed to afford a wish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub group_id: String,
    pub name: String,
    pub from: Decimal,
    pub to: Decimal,
}

/// How one purchase (or the previous month's card charge) is paid for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// `None` is the previous month's card charge ("Credit card") or a month
    /// that starts short ("Already short").
    pub wish_id: Option<String>,
    pub label: String,
    pub amount: Decimal,
    pub from_free: Decimal,
    pub cuts: Vec<Cut>,
    pub on_card: Decimal,
    pub short: Decimal,
    /// A monthly saving set-aside rather than the purchase itself.
    #[serde(default)]
    pub set_aside: bool,
    /// This verdict's share of the next month's card charge that the next
    /// month could not pay: its short, and the cuts it forced.
    #[serde(default)]
    pub next_short: Decimal,
    #[serde(default)]
    pub next_cuts: Vec<Cut>,
}

/// One month of the schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishMonth {
    /// `YYYY-MM-01`.
    pub month: String,
    pub free_before: Decimal,
    pub free_left: Decimal,
    pub card_charge_in: Decimal,
    pub card_used: Decimal,
    pub verdicts: Vec<Verdict>,
}

/// A wish with its sub-wishes and how it fares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishRow {
    pub wish: Wish,
    /// Empty for sub-wishes.
    pub children: Vec<WishRow>,
    /// Parents only, else 0.
    pub rest: Decimal,
    pub over_cap: bool,
    pub monthly_set_aside: Decimal,
    /// The sum of its verdicts' `short` and `next_short`.
    pub short: Decimal,
    /// Any verdict with cuts or card use.
    pub tradeoffs: bool,
}

/// Everything the wishes screen shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishesView {
    pub currency: Currency,
    pub settings: WishSettings,
    pub card_stale: bool,
    pub months: Vec<WishMonth>,
    /// Top-level only, newest due last.
    pub wishes: Vec<WishRow>,
}
