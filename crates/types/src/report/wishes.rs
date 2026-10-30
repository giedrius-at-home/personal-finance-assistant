//! The wishes screen's report: each month's free money, the card, and a
//! verdict per planned purchase.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::gateway::{Repeat, Wish, WishSettings};
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
    /// Drawn from the savings pot, after free money and before any cut.
    #[serde(default)]
    pub from_savings: Decimal,
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
    /// A must-have wish.
    #[serde(default)]
    pub must: bool,
    /// A wish left unpaid so the plan stays whole.
    #[serde(default)]
    pub skipped: bool,
}

/// What one card owes at the end of a month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardUsed {
    pub card_id: String,
    pub name: String,
    pub used: Decimal,
    /// The card's regular spending the month started with (typed usage or
    /// its linked bill), already paid for outside the wishes.
    #[serde(default)]
    pub regular: Decimal,
}

/// One month of the schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishMonth {
    /// `YYYY-MM-01`.
    pub month: String,
    pub free_before: Decimal,
    pub free_left: Decimal,
    /// The savings pot at the start and end of the month.
    #[serde(default)]
    pub savings_before: Decimal,
    #[serde(default)]
    pub savings_left: Decimal,
    pub card_charge_in: Decimal,
    pub card_used: Decimal,
    /// End-of-month usage per card: next month's payback.
    #[serde(default)]
    pub cards: Vec<CardUsed>,
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
    /// Any verdict with cuts, card use or money from savings.
    pub tradeoffs: bool,
    /// Why a planned wish with money still to find has no verdicts: "no
    /// budget data", "too far ahead" or "bill can't be judged".
    #[serde(default)]
    pub not_judged: Option<String>,
}

/// A bill whose schedule the judging knows, so a change to it can be judged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeableBill {
    pub bill_id: String,
    pub every: Repeat,
}

/// Everything the wishes screen shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishesView {
    pub currency: Currency,
    pub settings: WishSettings,
    pub card_stale: bool,
    /// Card ids whose usage was entered in an earlier month.
    #[serde(default)]
    pub stale_cards: Vec<String>,
    pub months: Vec<WishMonth>,
    /// Top-level only, newest due last.
    pub wishes: Vec<WishRow>,
    /// The bills a bill change can be made to.
    #[serde(default)]
    pub changeable_bills: Vec<ChangeableBill>,
}
