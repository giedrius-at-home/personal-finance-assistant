//! Report section types for `finance_core::plans`.

use serde::{Deserialize, Serialize};

use super::UnitMoney;
use crate::gateway::{Direction, PlannedPayment};

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
}

impl PlannedPaymentView {
    /// The stored plan this view reconciles.
    pub fn plan(&self) -> PlannedPayment {
        PlannedPayment {
            plan_id: self.plan_id.clone(),
            name: self.name.clone(),
            match_category: self.match_category.clone(),
            direction: self.direction,
            amount: self.amount.clone(),
            due_on: self.due_on.clone(),
            created_at: self.created_at.clone(),
            reimbursement_for: self.reimbursement_for.clone(),
            pending: self.pending,
            account_id: self.account_id.clone(),
            match_description: self.match_description.clone(),
        }
    }
}

/// A transaction that could be the plan's payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanMatchCandidate {
    pub transaction_id: String,
    pub occurred_on: String,
    pub description: String,
    pub amount: UnitMoney,
}
