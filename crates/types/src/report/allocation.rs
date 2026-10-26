//! Report section types for `finance_core::allocation`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Currency;

/// A spending envelope: its limit, spending and what is still reserved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAllocation {
    pub group: String,
    pub limit: Decimal,
    pub spent: Decimal,
    pub remaining: Decimal,
    pub projected_month_end: Decimal,
}

/// One bank account's booked balance and the planned payments it must cover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankAllocation {
    pub account_id: String,
    pub name: String,
    pub booked: Decimal,
    pub reserved: Decimal,
    pub available: Decimal,
    /// Whether the balance funds the envelopes (savings accounts never do).
    pub included: bool,
}

/// This month's commitments against booked cash, and next month's against
/// the salary that funds it, in one currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyAllocation {
    pub currency: Currency,
    pub current_month: String,
    pub funded_month: String,
    pub bank_balance: Decimal,
    pub protected_savings: Decimal,
    pub salary_reserved: Decimal,
    pub salary_received_on: Option<String>,
    pub current_month_funds: Decimal,
    pub current_fixed_remaining: Decimal,
    pub current_plans_remaining: Decimal,
    pub current_groups_remaining: Decimal,
    pub current_wiggle_room: Decimal,
    pub salary_at_risk: Decimal,
    pub next_fixed: Decimal,
    pub next_goals: Decimal,
    pub next_plans: Decimal,
    pub next_groups: Decimal,
    pub next_wiggle_room: Decimal,
    pub next_shortfall: Decimal,
    pub groups: Vec<GroupAllocation>,
    #[serde(default)]
    pub banks: Vec<BankAllocation>,
    #[serde(default)]
    pub next_group_rows: Vec<GroupAllocation>,
    #[serde(default)]
    pub current_expected_income: Decimal,
    #[serde(default)]
    pub next_expected_income: Decimal,
    #[serde(default)]
    pub expected_salary: Decimal,
    #[serde(default)]
    pub next_planned_funds: Decimal,
    #[serde(default)]
    pub current_planned_wiggle_room: Decimal,
    /// Already set aside in savings-role accounts; never spendable funds.
    #[serde(default)]
    pub savings_balance: Decimal,
}

/// Where this and next month's money goes, per currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyAllocation {
    pub through_date: String,
    pub currencies: Vec<CurrencyAllocation>,
}
