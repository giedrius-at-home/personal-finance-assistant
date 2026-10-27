//! Savings goal progress (Python domain/goals.py).

use finance_types::gateway::SavingsGoal;
use finance_types::report::reporting::GoalEvaluation;
use jiff::ToSpan;
use jiff::civil::Date;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::reporting::py_div;

/// Progress against a linear contribution schedule from `started_on` to
/// `target_date`. Unparseable dates count as `as_of`.
pub fn evaluate_goal(goal: &SavingsGoal, as_of: Date) -> GoalEvaluation {
    let started: Date = goal
        .started_on
        .get(..10)
        .and_then(|d| d.parse().ok())
        .unwrap_or(as_of);
    let target_date: Date = goal
        .target_date
        .get(..10)
        .and_then(|d| d.parse().ok())
        .unwrap_or(as_of);
    let days = |from: Date, to: Date| i64::from((to - from).get_days());
    let total_days = days(started, target_date);
    let elapsed_days = days(started, as_of).max(0).min(total_days);
    let days_remaining = days(as_of, target_date).max(0);

    let target = goal.target.amount;
    let current = goal.current.amount;
    // Python's `max(target - current, Decimal(0))` keeps the first of equals
    // (`0.00`, not `0`).
    let remaining = if target - current < Decimal::ZERO {
        Decimal::ZERO
    } else {
        target - current
    };
    let completed = remaining.is_zero();
    let overdue = as_of > target_date && !completed;

    // A goal that starts on its target date is fully due.
    let elapsed_ratio = if total_days == 0 {
        Decimal::ONE
    } else {
        py_div(Decimal::from(elapsed_days), Decimal::from(total_days))
    };
    let expected_saved = crate::trends::py_mul(target, elapsed_ratio);
    let schedule_variance = current - expected_saved;

    let required_weekly = if completed {
        Decimal::ZERO
    } else if days_remaining == 0 {
        remaining
    } else {
        py_div(remaining * Decimal::from(7), Decimal::from(days_remaining))
    };

    let observed_days = days(started, as_of).max(0);
    let projected = if completed {
        Some(as_of)
    } else if observed_days > 0 && current > Decimal::ZERO {
        let daily_rate = py_div(current, Decimal::from(observed_days));
        let projected_days = py_div(remaining, daily_rate).ceil();
        projected_days
            .to_i64()
            .and_then(|n| as_of.checked_add(n.days()).ok())
    } else {
        None
    };

    GoalEvaluation {
        goal_id: goal.goal_id.clone(),
        currency: goal.target.currency.clone(),
        completed,
        overdue,
        on_track: completed || schedule_variance >= Decimal::ZERO,
        remaining,
        expected_saved,
        schedule_variance,
        required_weekly_contribution: required_weekly,
        days_remaining,
        projected_completion_date: projected.map(|d| d.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::{Currency, UnitMoney};
    use jiff::civil::date;

    fn goal(current: &str) -> SavingsGoal {
        let money = |amount: &str| UnitMoney {
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
        };
        SavingsGoal {
            goal_id: "holiday".into(),
            name: "Holiday".into(),
            target: money("900"),
            current: money(current),
            started_on: "2026-01-01".into(),
            target_date: "2026-04-01".into(),
        }
    }

    #[test]
    fn exact_and_explainable() {
        let r = evaluate_goal(&goal("300"), date(2026, 1, 31));
        assert_eq!(r.remaining, Decimal::from(600));
        assert_eq!(r.expected_saved, Decimal::from(300));
        assert_eq!(r.schedule_variance, Decimal::ZERO);
        assert_eq!(r.required_weekly_contribution, Decimal::from(70));
        assert_eq!(r.days_remaining, 60);
        assert!(r.on_track);
        assert_eq!(r.projected_completion_date.as_deref(), Some("2026-04-01"));
    }

    #[test]
    fn completed_never_negative() {
        let r = evaluate_goal(&goal("1000"), date(2026, 2, 1));
        assert!(r.completed);
        assert_eq!(r.remaining, Decimal::ZERO);
        assert_eq!(r.required_weekly_contribution, Decimal::ZERO);
        assert_eq!(r.projected_completion_date.as_deref(), Some("2026-02-01"));
    }

    #[test]
    fn overdue_requires_everything_now() {
        let r = evaluate_goal(&goal("300"), date(2026, 4, 2));
        assert!(r.overdue);
        assert_eq!(r.days_remaining, 0);
        assert_eq!(r.required_weekly_contribution, Decimal::from(600));
    }
}
