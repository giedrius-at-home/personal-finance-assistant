//! The money plan under the wizard: from the balances as they stand, what
//! each account keeps, the transfers to make and what savings must hold.

use std::collections::HashMap;

use finance_types::report::closure::MoneyPlan;
use leptos::prelude::*;

use super::ui::{StepCard, month_label};
use crate::ds::money_plan::MoneyPlanFields;

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `1 Nov` from `YYYY-MM-DD`.
fn short_day(day: &str) -> String {
    match (
        day.get(8..10).and_then(|d| d.parse::<u32>().ok()),
        day.get(5..7).and_then(|m| m.parse::<usize>().ok()),
    ) {
        (Some(d), Some(m @ 1..=12)) => format!("{d} {}", MONTHS_EN[m - 1]),
        _ => day.to_owned(),
    }
}

/// Where the money should go now.
#[component]
pub fn MoneyPlanPanel(plan: MoneyPlan, names: HashMap<String, String>) -> impl IntoView {
    let description = format!(
        "Balances as of {}, for {}.",
        short_day(&plan.as_of),
        month_label(&plan.month, false)
    );
    view! {
        <StepCard title="Where the money goes" description=Some(description)>
            <MoneyPlanFields plan=plan names=names />
        </StepCard>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_day_reads_day_and_month() {
        assert_eq!(short_day("2026-11-01"), "1 Nov");
        assert_eq!(short_day("nonsense"), "nonsense");
    }
}
