//! The Overview section for the owner (personal-site `integrated-finance.tsx`,
//! Overview branch): the monthly plan (`owner-monthly-overview.tsx`), the
//! monthly budget (`monthly-budget.tsx` and the panels it holds) and the
//! trends card (`overview-money.tsx` with `showMoney={false}`).

mod budget;
mod extras;
mod forecast;
mod monthly;
mod outlook;
mod trends;
mod visuals;

use leptos::prelude::*;

use crate::format;

/// Where the site shows `MonthClosureBanner` (`month-closure.tsx`): the nudge
/// to close last month, above the Overview. Month closing is ported
/// separately; it plugs in here.
fn month_closure_banner() -> impl IntoView {}

#[component]
pub fn Overview() -> impl IntoView {
    view! {
        {month_closure_banner()}
        <monthly::OwnerMonthlyOverview />
        {budget_panel()}
        <trends::OverviewTrends />
    }
}

/// The monthly budget panel, shared with Spending → Plan & patterns.
pub fn budget_panel() -> impl IntoView {
    view! { <budget::MonthlyBudgetPanel /> }
}

/// `2026-10-01` → `October` (en-GB long month).
fn month_name(value: &str) -> String {
    format::month_long(&value[..7.min(value.len())])
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// `2026-10-01` → `October 2026`.
fn month_label(value: &str) -> String {
    format::month_long(&value[..7.min(value.len())])
}

/// `2026-10-26` → `Mon` (en-GB short weekday).
fn weekday(value: &str) -> &'static str {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let part = |range: std::ops::Range<usize>| {
        value
            .get(range)
            .and_then(|p| p.parse::<i32>().ok())
            .unwrap_or(1)
    };
    let (mut y, m, d) = (part(0..4), part(5..7).clamp(1, 12), part(8..10));
    if m < 3 {
        y -= 1;
    }
    let index = (y + y / 4 - y / 100 + y / 400 + OFFSETS[(m - 1) as usize] + d).rem_euclid(7);
    DAYS[index as usize]
}

/// Days left in the month after `through` (at least one).
fn remaining_days(through: &str) -> u32 {
    let day: u32 = through.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(1);
    format::days_in_month(through).saturating_sub(day).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays() {
        assert_eq!(weekday("2026-10-26"), "Mon");
        assert_eq!(weekday("2024-02-29"), "Thu");
        assert_eq!(weekday("2026-01-01"), "Thu");
    }

    #[test]
    fn months() {
        assert_eq!(month_name("2026-10-01"), "October");
        assert_eq!(month_label("2026-11-01"), "November 2026");
        assert_eq!(remaining_days("2026-10-26"), 5);
        assert_eq!(remaining_days("2026-10-31"), 1);
    }
}
