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
    /// What "saved so far" should read once this month's set-aside is put
    /// away: `saved + monthly_set_aside` until it is recorded for this
    /// month, then `saved`.
    #[serde(default)]
    pub suggested_saved: Decimal,
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
    /// Card ids whose usage was entered for a statement that has closed.
    #[serde(default)]
    pub stale_cards: Vec<String>,
    pub months: Vec<WishMonth>,
    /// Top-level only, newest due last.
    pub wishes: Vec<WishRow>,
    /// The bills a bill change can be made to.
    #[serde(default)]
    pub changeable_bills: Vec<ChangeableBill>,
    /// Σ saved of planned one-off wishes: what must stay in savings.
    #[serde(default)]
    pub savings_must_keep: Decimal,
    /// The wishes and the emergency fund against current habits.
    #[serde(default)]
    pub goals: Option<GoalOutlook>,
}

/// What the goals need a month against what current habits leave, judged by
/// the same ladder on a habit scenario: every group costs what it usually
/// does and nothing is cut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalOutlook {
    /// What the coming months ask: each later purchase spread over the
    /// months until it is due, plus the planned repeating costs that stay.
    pub needed_monthly: Decimal,
    /// A later month's free money when spending stays as usual.
    pub habit_free: Decimal,
    /// The same when the budgets are kept.
    pub plan_free: Decimal,
    pub gap: Decimal,
    /// `habit_free / needed_monthly` in percent, 0 to 100; `None` when
    /// nothing is needed.
    pub coverage: Option<u32>,
    /// The months judged, the running one included.
    pub horizon_months: usize,
    pub goals: Vec<GoalStatus>,
    /// How the gap could be closed, each group giving the same share.
    pub cuts: Vec<HabitCut>,
    /// The part of the gap that cutting every group to its floor leaves.
    #[serde(default)]
    pub uncovered: Decimal,
    /// `None` when the fund is switched off.
    pub fund: Option<EmergencyFund>,
}

/// One planned wish at current habits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalStatus {
    pub wish_id: String,
    pub name: String,
    /// `YYYY-MM-DD`.
    pub due_on: String,
    pub amount: Decimal,
    pub must: bool,
    /// What cannot be paid when spending stays as usual.
    pub short: Decimal,
    /// What ends up on a credit card when spending stays as usual.
    #[serde(default)]
    pub on_card: Decimal,
    /// What cannot be paid even when the budgets are kept.
    pub plan_short: Decimal,
}

/// What one spending group would give a month to close the gap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitCut {
    pub group_id: String,
    pub name: String,
    /// What the group usually costs a month.
    pub typical: Decimal,
    pub monthly: Decimal,
    /// The usual visits a month, when known.
    pub typical_visits: Option<Decimal>,
}

/// Months of bare living costs to hold; it takes what the wishes leave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencyFund {
    pub months: u32,
    /// Bills a month, with the planned changes that do not end.
    pub fixed_monthly: Decimal,
    /// The typical month of the essential groups.
    pub essentials_monthly: Decimal,
    pub target: Decimal,
    /// The savings pot that is not saved for a wish.
    pub balance: Decimal,
    /// The month (`YYYY-MM-01`) the fund is full at current habits; `None`
    /// when nothing is left over to fill it.
    pub eta: Option<String>,
}
