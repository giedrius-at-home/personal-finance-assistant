//! Report section types for `finance_core::plans`.

use serde::{Deserialize, Serialize};

use super::UnitMoney;
use crate::gateway::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Upcoming,
    Overdue,
    /// Exactly one transaction matches, and no other plan claims it.
    Paid,
    /// Several transactions match, or one matches several plans.
    NeedsReview,
    /// A card reservation still waiting for its booking.
    Pending,
}

/// A planned payment with its reconciliation against imported transactions.
/// The plan fields are repeated (not flattened) so optional ones serialize
/// as `null`, as the Python view did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPaymentView {
    pub plan_id: String,
    pub name: String,
    pub match_category: String,
    pub direction: Direction,
    pub amount: UnitMoney,
    pub due_on: String,
    pub created_at: String,
    pub reimbursement_for: Option<String>,
    pub pending: bool,
    pub account_id: Option<String>,
    pub match_description: Option<String>,
    pub status: PlanStatus,
    pub paid_on: Option<String>,
    pub transaction_id: Option<String>,
    pub candidate_count: usize,
    pub candidates: Vec<PlanMatchCandidate>,
    /// The owner tied `transaction_id` to the plan by hand.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub by_hand: bool,
}

/// A transaction that could be the plan's payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanMatchCandidate {
    pub transaction_id: String,
    pub occurred_on: String,
    pub description: String,
    pub amount: UnitMoney,
}
