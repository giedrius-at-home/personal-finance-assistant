//! Saved monthly group limits judged against this month's spending pace
//! (Python `domain/spending_limits.py`).

use finance_types::gateway::SpendingLimit;
use finance_types::report::trends::{SpendingNudge, SpendingRisk, TimeTrends};
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::reporting::{py_div, py_round};
use crate::trends::py_mul;

fn two_places(d: Decimal) -> String {
    format!(
        "{:.2}",
        d.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointNearestEven)
    )
}

/// How each saved limit is doing, from the (full, not variable) time trends;
/// the most urgent first. Limits in a currency without spending are skipped.
pub fn spending_nudges(limits: &[SpendingLimit], trends: &TimeTrends) -> Vec<SpendingNudge> {
    let Ok(through) = trends.through.parse::<Date>() else {
        return Vec::new();
    };
    let month_progress = py_div(
        Decimal::from(through.day()),
        Decimal::from(through.days_in_month()),
    );
    let mut nudges: Vec<SpendingNudge> = limits
        .iter()
        .filter_map(|saved| {
            let currency = &saved.amount.currency;
            let currency_trend = trends
                .currencies
                .iter()
                .find(|c| c.currency.code == currency.code)?;
            let group = currency_trend
                .groups
                .iter()
                .find(|g| g.group.as_deref() == Some(saved.group.as_str()));
            let spent = group
                .and_then(|g| g.values.last().copied().flatten())
                .unwrap_or_default();
            let projected = group.map(|g| g.projected_month_end).unwrap_or_default();
            let limit = saved.amount.amount;
            let remaining = Decimal::ZERO.max(limit - spent);
            let limit_progress = py_div(spent, limit);
            let purchase_count = group.map_or(0, |g| g.purchase_count);
            let (risk, reason) = if spent >= limit {
                (
                    SpendingRisk::OverLimit,
                    format!(
                        "This group is {} {} over its saved monthly limit.",
                        two_places(spent - limit),
                        currency.code
                    ),
                )
            } else if purchase_count >= 2 && projected >= py_mul(limit, Decimal::new(110, 2)) {
                (
                    SpendingRisk::LikelyOver,
                    format!(
                        "At the current transaction pace this group is projected to finish {} {} above its limit.",
                        two_places(py_round(projected - limit)),
                        currency.code
                    ),
                )
            } else if purchase_count >= 2
                && (projected > limit
                    || limit_progress > py_round(month_progress + Decimal::new(10, 2)))
            {
                (
                    SpendingRisk::Watch,
                    "This group is using its monthly limit faster than the month is passing."
                        .to_owned(),
                )
            } else {
                (
                    SpendingRisk::OnTrack,
                    "Current categorized spending remains within the saved pace.".to_owned(),
                )
            };
            Some(SpendingNudge {
                group: saved.group.clone(),
                currency: currency.clone(),
                risk,
                limit,
                spent,
                projected_month_end: projected,
                remaining,
                historical_median: group.and_then(|g| g.historical_median),
                history_month_count: group.map_or(0, |g| g.history_month_count),
                purchase_count,
                month_progress,
                limit_progress,
                reason,
            })
        })
        .collect();
    nudges.sort_by(|a, b| (a.risk, &a.group).cmp(&(b.risk, &b.group)));
    nudges
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use finance_types::report::{Currency, UnitMoney};
    use jiff::Zoned;

    use super::*;
    use crate::trends::tests::{mapping, out};
    use crate::trends::time_trends;

    const NOW: &str = "2026-09-10T12:00:00+00:00[UTC]";

    fn limit(group: &str, amount: &str) -> SpendingLimit {
        SpendingLimit {
            group: group.into(),
            amount: UnitMoney {
                amount: amount.parse().unwrap(),
                currency: Currency::eur(),
            },
            revision: 1,
            updated_at: "2026-09-10T12:00:00Z".into(),
        }
    }

    fn trends(amounts: &[&str]) -> TimeTrends {
        let rows: Vec<_> = amounts
            .iter()
            .enumerate()
            .map(|(i, a)| out(&i.to_string(), a, Some("Jumbo"), NOW))
            .collect();
        let now: Zoned = NOW.parse().unwrap();
        time_trends(
            &rows,
            &[mapping("Jumbo", "Groceries")],
            &now,
            &HashSet::new(),
        )
    }

    #[test]
    fn nudge_reacts_to_group_pace() {
        let result = spending_nudges(
            &[limit("Groceries", "200.00")],
            &trends(&["60.00", "60.00"]),
        );
        assert_eq!(result.len(), 1);
        let nudge = &result[0];
        assert_eq!(nudge.spent, "120.00".parse().unwrap());
        assert_eq!(nudge.projected_month_end, "360.00".parse().unwrap());
        assert_eq!(nudge.risk, SpendingRisk::LikelyOver);
        assert_eq!(nudge.remaining, "80.00".parse().unwrap());
        assert_eq!(
            nudge.month_progress,
            "0.3333333333333333333333333333".parse().unwrap()
        );
        assert!(nudge.reason.contains("finish 160.00 EUR above"));
    }

    #[test]
    fn risks_order_most_urgent_first() {
        let trends = trends(&["60.00", "60.00"]);
        let result = spending_nudges(
            &[
                limit("Groceries", "1000"),
                limit("Groceries", "100"),
                limit("Travel", "50"),
                limit("Groceries", "340"),
            ],
            &trends,
        );
        let risks: Vec<_> = result
            .iter()
            .map(|n| (n.risk, n.limit.to_string()))
            .collect();
        assert_eq!(
            risks,
            [
                (SpendingRisk::OverLimit, "100".into()),
                (SpendingRisk::Watch, "340".into()),
                (SpendingRisk::OnTrack, "1000".into()),
                (SpendingRisk::OnTrack, "50".into()),
            ]
        );
        assert_eq!(
            result[0].reason,
            "This group is 20.00 EUR over its saved monthly limit."
        );
        let travel = &result[3];
        assert_eq!((travel.spent, travel.purchase_count), (Decimal::ZERO, 0));
    }

    #[test]
    fn limits_in_other_currencies_are_skipped() {
        let mut usd = limit("Groceries", "10");
        usd.amount.currency = Currency::new("USD");
        assert!(spending_nudges(&[usd], &trends(&["5"])).is_empty());
    }
}
