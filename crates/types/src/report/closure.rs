//! Month closure (Python `domain/month_closure.py`, `closure_service.py`):
//! the stored `month_periods` payload, the closure screen's view and the
//! requests its actions take. Field names and order match the Python JSON,
//! so periods written by either implementation stay readable by the other.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

use super::Currency;
use super::outlook::ForecastExclusions;

/// Where a month is in its closure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureStatus {
    /// Nothing done yet.
    #[default]
    Open,
    /// Banks synced; the split can be made.
    Settling,
    /// Split saved; transfers may still be made before the final close.
    Settled,
    /// Final: later bookings belong to the next month.
    Closed,
    /// Closed, but transactions of the month arrived after the close.
    ChangedAfterClose,
}

impl ClosureStatus {
    /// The word the Python service used (and its error messages show).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Settling => "settling",
            Self::Settled => "settled",
            Self::Closed => "closed",
            Self::ChangedAfterClose => "changed_after_close",
        }
    }
}

/// Whether a line (or hop) needs a bank transfer and whether it was seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Matched,
    #[default]
    NotNeeded,
}

/// A split line made at settle time, or a delta line added after a late change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    #[default]
    Split,
    Delta,
}

fn pending() -> TransferStatus {
    TransferStatus::Pending
}

/// One bank transfer on a routed line's path, e.g. ABN → Revolut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferHop {
    pub from_account_id: String,
    pub to_account_id: String,
    pub amount: Decimal,
    /// `pending` or `matched` (never `not_needed`).
    #[serde(default = "pending")]
    pub transfer_status: TransferStatus,
    #[serde(default)]
    pub matched_transaction_ids: Vec<String>,
    /// Marked by the owner rather than matched on the exact amount.
    #[serde(default)]
    pub manual: bool,
    /// What the marked transactions moved, when the owner marked them.
    #[serde(default)]
    pub moved_amount: Option<Decimal>,
}

/// Where part of a month's leftover goes. The client sends `line_id`,
/// `from_account_id`, `destination`, `amount`, `currency` and the `plan_*`
/// fields; the server owns every transfer field and `plan_id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationLine {
    pub line_id: String,
    pub from_account_id: String,
    /// `goal:<id>`, `wish:<wish_id>`, `pot:<pot_id>` (legacy, read-only), `plan` (a planned purchase), `buffer`
    /// (next month) or `stay`.
    pub destination: String,
    /// Negative only for a take from a pot (split) or a delta line.
    pub amount: Decimal,
    pub currency: Currency,
    #[serde(default)]
    pub kind: LineKind,
    #[serde(default)]
    pub transfer_status: TransferStatus,
    #[serde(default)]
    pub to_account_id: Option<String>,
    #[serde(default)]
    pub matched_transaction_ids: Vec<String>,
    #[serde(default)]
    pub plan_name: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub plan_due_on: Option<String>,
    /// How the planned purchase is recognised once paid.
    #[serde(default)]
    pub plan_match_category: Option<String>,
    #[serde(default)]
    pub plan_match_description: Option<String>,
    /// The planned payment a plan line created.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// A take from a pot moves money from the pot's account.
    #[serde(default)]
    pub transfer_source_account_id: Option<String>,
    /// A routed transfer's bank moves, in order; empty for a direct one.
    #[serde(default)]
    pub transfer_hops: Vec<TransferHop>,
    /// A direct transfer the owner marked done by hand.
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    pub moved_amount: Option<Decimal>,
}

impl AllocationLine {
    /// The pot a `pot:<id>` line moves money into (or takes it from).
    pub fn pot_id(&self) -> Option<&str> {
        self.destination.strip_prefix("pot:")
    }

    /// The savings goal of a `goal:<id>` line.
    pub fn goal_id(&self) -> Option<&str> {
        self.destination.strip_prefix("goal:")
    }

    /// The planned wish a `wish:<id>` line saves toward.
    pub fn wish_id(&self) -> Option<&str> {
        self.destination.strip_prefix("wish:")
    }
}

/// One budget group's plan and spending, under the account it lives on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupReview {
    pub group_id: String,
    pub name: String,
    pub planned: Decimal,
    pub spent: Decimal,
    /// The part spent from other accounts.
    #[serde(default)]
    pub spent_elsewhere: Decimal,
}

/// "spending", "savings", "ignored" or "unclassified" (no role saved yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRoleKind {
    Spending,
    Savings,
    Ignored,
    Unclassified,
}

impl AccountRoleKind {
    /// A stored role name; anything unknown reads as unclassified.
    pub fn parse(role: &str) -> Self {
        match role {
            "spending" => Self::Spending,
            "savings" => Self::Savings,
            "ignored" => Self::Ignored,
            _ => Self::Unclassified,
        }
    }
}

/// One account's month: what came in, went out and is left over.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReview {
    pub account_id: String,
    pub name: String,
    pub currency: Currency,
    pub role: AccountRoleKind,
    pub closing_balance: Decimal,
    /// Last month's buffer plus money set aside for a purchase due this month.
    pub carry_in: Decimal,
    pub income_planned: Decimal,
    pub income_actual: Decimal,
    pub fixed_planned: Decimal,
    pub fixed_actual: Decimal,
    pub other_spent: Decimal,
    #[serde(default)]
    pub groups: Vec<GroupReview>,
    pub leftover: Decimal,
    /// Own-account transactions in another currency, left out of the totals.
    #[serde(default)]
    pub skipped_foreign: usize,
}

/// The per-account review of one month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureReview {
    /// `YYYY-MM-01`.
    pub month: String,
    pub accounts: Vec<AccountReview>,
    /// Budget groups with no (known) home account.
    #[serde(default)]
    pub unassigned_groups: Vec<String>,
    /// Planned payments of the month with no account.
    #[serde(default)]
    pub unassigned_payments: Vec<String>,
    #[serde(default)]
    pub unclassified_accounts: Vec<String>,
}

/// `month_periods.payload_json`: one month's closure. Instants are RFC 3339
/// strings as stored (`…Z` for those the service wrote).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonthPeriod {
    /// `YYYY-MM-01`.
    pub month: String,
    /// Derived on every read: midnight, or the previous month's early cutoff.
    pub starts_at: String,
    /// Set by the final close; later bookings belong to the next month.
    #[serde(default)]
    pub cutoff_at: Option<String>,
    #[serde(default)]
    pub status: ClosureStatus,
    #[serde(default)]
    pub settled_at: Option<String>,
    /// The review the split (or last resettle) was made against.
    #[serde(default)]
    pub settled_snapshot: Option<ClosureReview>,
    /// Banks that were not synced when the month was closed.
    #[serde(default)]
    pub missing_banks: Vec<String>,
    #[serde(default)]
    pub allocations: Vec<AllocationLine>,
    /// Bumped by every save; a write must name the revision it read.
    #[serde(default)]
    pub revision: u32,
    /// Late arrivals observed up to here were covered by a resettle.
    #[serde(default)]
    pub reviewed_through: Option<String>,
    #[serde(default)]
    pub reopened_at: Option<String>,
    /// The owner's confirmed bank balance per account (`account_id` → check).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub balance_checks: BTreeMap<String, BalanceCheck>,
}

/// The owner's confirmed bank balance for one account at the close.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BalanceCheck {
    pub balance: Decimal,
    pub ledger_balance: Decimal,
    pub confirmed_at: String,
    /// The bank-side transaction the check was matched to, if any.
    #[serde(default)]
    pub transaction_id: Option<String>,
}

/// Whether an account's balance has been confirmed for the month.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceState {
    Unconfirmed,
    Confirmed,
    Stale,
}

/// One account's balance check as the closure screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceStatus {
    pub account_id: String,
    pub name: String,
    pub currency: Currency,
    pub ledger: Decimal,
    pub bank: Option<Decimal>,
    pub bank_reported_on: Option<String>,
    pub check: Option<BalanceCheck>,
    pub state: BalanceState,
    pub adjusted_by: Option<Decimal>,
}

/// One known account and its closure role, ignored ones included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRoleView {
    pub account_id: String,
    pub name: String,
    pub role: AccountRoleKind,
    #[serde(default)]
    pub goal_id: Option<String>,
    /// Money reaches (and leaves) this account only through that one.
    #[serde(default)]
    pub via_account_id: Option<String>,
}

/// A savings pot: an earmark on money in one account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotView {
    pub pot_id: String,
    pub name: String,
    /// Where the money physically sits, e.g. `firefly:413`.
    pub account_id: Option<String>,
    pub archived: bool,
    /// Balance per currency code, e.g. `{"EUR": "123.45"}`.
    pub balances: BTreeMap<String, Decimal>,
}

/// A planned one-off wish a split line can save toward (view only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishOption {
    pub wish_id: String,
    /// `Wish: <name> (€saved of €amount)`, or `Wish: <name> rest` for a
    /// parent with sub-wishes.
    pub label: String,
    pub currency: Currency,
    /// What the wish can still take this month.
    pub room: Decimal,
}

/// A one-off planned payment due in the months after the funded one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownPayment {
    pub name: String,
    pub due_on: String,
    pub amount: Decimal,
}

/// EUR inputs for the live savings suggestion, as of `as_of`.
/// `projected_available` is Overview's figure for `month` (the month being
/// funded now); its parts are listed so the breakdown adds up. The UI then
/// subtracts the split's own lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingsSuggestion {
    pub as_of: String,
    pub month: String,
    pub projected_available: Decimal,
    pub bank_balance: Decimal,
    pub protected_savings: Decimal,
    pub salary_reserved: Decimal,
    pub bills_remaining: Decimal,
    pub plans_remaining: Decimal,
    pub groups_remaining: Decimal,
    pub expected_income: Decimal,
    /// Everyday spending still forecast this month, outside the budget groups.
    pub everyday_remaining: Decimal,
    pub forecast_available: bool,
    pub forecast_model: Option<String>,
    /// What the everyday forecast left out of its history.
    pub forecast_excluded: Option<ForecastExclusions>,
    /// One-off plans due in the three months after `month`.
    pub known_future_payments: Decimal,
    pub known_payments: Vec<KnownPayment>,
    pub safety_buffer: Decimal,
}

/// One own-account bank transfer since the split (paired legs as one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferCandidate {
    pub transaction_ids: Vec<String>,
    pub from_account_id: String,
    pub to_account_id: Option<String>,
    pub amount: Decimal,
    pub currency: Currency,
    pub booked_on: String,
    pub description: Option<String>,
}

/// Everything the closure screen shows for one month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureView {
    pub period: MonthPeriod,
    /// The month as it stands now.
    pub review: ClosureReview,
    /// `(goal_id, name)` of every savings goal.
    pub goals: Vec<(String, String)>,
    /// Pots with their balances: legacy, for naming old `pot:` lines.
    pub pots: Vec<PotView>,
    /// Wishes a split line can save toward: parents, then their sub-wishes.
    #[serde(default)]
    pub wishes: Vec<WishOption>,
    /// `(wish_id, label)` of every wish, for naming lines: `Wish: <name>`,
    /// or `Wish (bought): <name>` / `Wish (dropped): <name>` /
    /// `Wish (no longer planned): <name>` once it can take no line.
    #[serde(default)]
    pub wish_labels: Vec<(String, String)>,
    pub suggestion: Option<SavingsSuggestion>,
    /// Every known account and its role, ignored ones included.
    pub accounts: Vec<AccountRoleView>,
    /// Own-account transfers since the split, for marking a transfer by hand.
    pub transfer_candidates: Vec<TransferCandidate>,
    /// Each account's balance check for the month.
    #[serde(default)]
    pub balances: Vec<BalanceStatus>,
}

/// An account with no role yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedAccount {
    pub account_id: String,
    pub name: String,
}

/// Accounts without a role, and outflows that look like savings transfers
/// to an account the ledger does not know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnclassifiedAccounts {
    pub accounts: Vec<NamedAccount>,
    /// Transaction ids.
    pub suspected: Vec<String>,
}

/// An account's closure role as the owner sets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountRoleDraft {
    pub account_id: String,
    /// "spending", "savings" or "ignored".
    pub role: String,
    /// Only for a savings account; empty means none.
    #[serde(default)]
    pub goal_id: Option<String>,
    /// Left out keeps the saved route; `null` clears it.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub via_account_id: Option<Option<String>>,
}

/// One closure step, as the web route took it (`POST /api/finance/closures`).
/// Every step names the month (`YYYY-MM-01`) and the period `revision` the
/// screen last read; a stale revision is refused with "month closure
/// changed; refresh".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ClosureAction {
    /// Re-read the data and start settling (from the 25th). `missing_banks`
    /// are bank ids (`abnamro`, `revolut`, `swedbank`) that could not sync.
    Sync {
        month: String,
        revision: u32,
        #[serde(default)]
        missing_banks: Vec<String>,
    },
    /// Save (or replace) the split of every positive leftover.
    Settle {
        month: String,
        revision: u32,
        lines: Vec<AllocationLine>,
    },
    /// Close the month: tick off transfers and buffer any late change.
    Finalize {
        month: String,
        revision: u32,
        #[serde(default)]
        missing_banks: Vec<String>,
    },
    /// Assign the leftover change of a month that changed after its close.
    Resettle {
        month: String,
        revision: u32,
        lines: Vec<AllocationLine>,
    },
    /// Undo the final close; the split stays.
    Reopen { month: String, revision: u32 },
    /// Mark a transfer (or one hop of a routed one) done with these bank
    /// transfers. `hop` is the hop index, or `null`/`"direct"` for an
    /// unrouted transfer.
    Mark {
        month: String,
        revision: u32,
        line_id: String,
        #[serde(default, deserialize_with = "hop")]
        hop: Option<u32>,
        transaction_ids: Vec<String>,
    },
    /// Undo a mark made by hand.
    Unmark {
        month: String,
        revision: u32,
        line_id: String,
        #[serde(default, deserialize_with = "hop")]
        hop: Option<u32>,
    },
    /// Confirm an account's bank balance for the month.
    ConfirmBalance {
        month: String,
        revision: u32,
        account_id: String,
        balance: Decimal,
    },
}

impl ClosureAction {
    /// The month the step acts on.
    pub fn month(&self) -> &str {
        match self {
            Self::Sync { month, .. }
            | Self::Settle { month, .. }
            | Self::Finalize { month, .. }
            | Self::Resettle { month, .. }
            | Self::Reopen { month, .. }
            | Self::Mark { month, .. }
            | Self::Unmark { month, .. }
            | Self::ConfirmBalance { month, .. } => month,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HopInput {
    Index(u32),
    Name(String),
}

/// A hop index, or `null`/`"direct"` for an unrouted transfer.
fn hop<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    use serde::de::Error;
    match Option::<HopInput>::deserialize(d)? {
        None => Ok(None),
        Some(HopInput::Index(i)) => Ok(Some(i)),
        Some(HopInput::Name(name)) if name == "direct" => Ok(None),
        Some(HopInput::Name(name)) => name
            .parse()
            .map(Some)
            .map_err(|_| D::Error::custom("invalid hop")),
    }
}

/// A field that is present (even as `null`) becomes `Some`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn via_route_is_kept_cleared_or_set() {
        let parse = |json: &str| -> AccountRoleDraft { serde_json::from_str(json).unwrap() };
        let base = r#""account_id":"a","role":"savings""#;
        assert_eq!(parse(&format!("{{{base}}}")).via_account_id, None);
        assert_eq!(
            parse(&format!(r#"{{{base},"via_account_id":null}}"#)).via_account_id,
            Some(None)
        );
        assert_eq!(
            parse(&format!(r#"{{{base},"via_account_id":"b"}}"#)).via_account_id,
            Some(Some("b".into()))
        );
    }

    #[test]
    fn actions_take_the_web_route_shape() {
        let mark: ClosureAction = serde_json::from_str(
            r#"{"action":"mark","month":"2026-09-01","revision":3,"line_id":"a","hop":"direct","transaction_ids":["firefly:1"]}"#,
        )
        .unwrap();
        assert!(matches!(mark, ClosureAction::Mark { hop: None, .. }));
        let unmark: ClosureAction = serde_json::from_str(
            r#"{"action":"unmark","month":"2026-09-01","revision":3,"line_id":"a","hop":1}"#,
        )
        .unwrap();
        assert!(matches!(unmark, ClosureAction::Unmark { hop: Some(1), .. }));
        assert_eq!(unmark.month(), "2026-09-01");
    }

    #[test]
    fn stored_period_round_trips() {
        let json = r#"{"month":"2026-09-01","starts_at":"2026-09-01T00:00:00+02:00","cutoff_at":null,"status":"settled","settled_at":"2026-09-30T21:54:02.614463Z","settled_snapshot":null,"missing_banks":["swedbank"],"allocations":[{"line_id":"a","from_account_id":"firefly:1","destination":"stay","amount":"10.00","currency":{"code":"EUR","minor_unit":2},"kind":"split","transfer_status":"not_needed","to_account_id":null,"matched_transaction_ids":[],"plan_name":null,"plan_due_on":null,"plan_match_category":null,"plan_match_description":null,"plan_id":null,"transfer_source_account_id":null,"transfer_hops":[],"manual":false,"moved_amount":null}],"revision":3,"reviewed_through":null,"reopened_at":null}"#;
        let period: MonthPeriod = serde_json::from_str(json).unwrap();
        assert_eq!(period.status, ClosureStatus::Settled);
        assert_eq!(serde_json::to_string(&period).unwrap(), json);
    }

    #[test]
    fn balance_checks_round_trip_and_stay_out_of_an_empty_payload() {
        let json = r#"{"month":"2026-10-01","starts_at":"2026-10-01T00:00:00+02:00","cutoff_at":null,"status":"settling","settled_at":null,"settled_snapshot":null,"missing_banks":[],"allocations":[],"revision":2,"reviewed_through":null,"reopened_at":null,"balance_checks":{"firefly:1":{"balance":"1234.56","ledger_balance":"1234.56","confirmed_at":"2026-11-01T10:00:00Z","transaction_id":"firefly:77"}}}"#;
        let period: MonthPeriod = serde_json::from_str(json).unwrap();
        assert_eq!(
            period.balance_checks["firefly:1"].transaction_id.as_deref(),
            Some("firefly:77")
        );
        assert_eq!(serde_json::to_string(&period).unwrap(), json);
        let empty = MonthPeriod {
            balance_checks: BTreeMap::new(),
            ..period
        };
        assert!(
            !serde_json::to_string(&empty)
                .unwrap()
                .contains("balance_checks")
        );
    }

    #[test]
    fn confirm_balance_action_parses() {
        let action: ClosureAction = serde_json::from_str(
            r#"{"action":"confirm_balance","month":"2026-10-01","revision":2,"account_id":"firefly:1","balance":"270.00"}"#,
        ).unwrap();
        assert!(
            matches!(action, ClosureAction::ConfirmBalance { ref account_id, .. } if account_id == "firefly:1")
        );
        assert_eq!(action.month(), "2026-10-01");
    }
}
