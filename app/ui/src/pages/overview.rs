//! The Overview section for the owner (personal-site `integrated-finance.tsx`,
//! Overview branch): the monthly plan (`owner-monthly-overview.tsx`), the
//! monthly budget (`monthly-budget.tsx` and the panels it holds) and the
//! trends card (`overview-money.tsx` with `showMoney={false}`).

mod brief;
mod budget;
mod extras;
mod forecast;
mod kpis;
mod monthly;
mod nudges;
pub mod outlook;
mod today;
pub mod visuals;

use leptos::prelude::*;

use crate::format;
use crate::layout::{Initial, PageView, PanelDef};
use crate::pages::report_is;

/// Where the site shows `MonthClosureBanner` (`month-closure.tsx`): the nudge
/// to close last month, above the Overview.
fn month_closure_banner() -> impl IntoView {
    crate::pages::closure::MonthClosureBanner()
}

/// Overview's panels, in their first-run order.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "overview.kpis",
        title: "This month at a glance",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || kpis::OverviewKpis().into_any(),
        summary: None,
        shown: Some(|| report_is(|r| kpis::first_allocation(r).is_some())),
    },
    PanelDef {
        id: "overview.nudges",
        title: "Do this now",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || nudges::OverviewNudges().into_any(),
        summary: Some(nudges::summary),
        shown: Some(|| report_is(|r| r.monthly_allocation.is_some())),
    },
    PanelDef {
        id: "overview.brief",
        title: "Today's brief",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || brief::OverviewBrief().into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "overview.today",
        title: "What today cost",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || today::OverviewToday().into_any(),
        summary: None,
        shown: Some(|| report_is(|r| r.monthly_allocation.is_some())),
    },
    PanelDef {
        id: "overview.spendable",
        title: "Spendable and From salary",
        page: PAGE,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || monthly::OwnerMonthlyOverview().into_any(),
        summary: Some(monthly::spendable_summary),
        shown: None,
    },
    PanelDef {
        id: "overview.heading",
        title: "Where this month is heading",
        page: PAGE,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || monthly::MonthHeading().into_any(),
        summary: None,
        shown: Some(|| report_is(|r| monthly::heading_allocation(r).is_some())),
    },
    PanelDef {
        id: "overview.budget",
        title: "Monthly budget",
        page: PAGE,
        movable: false,
        initial: Initial::Open,
        half: false,
        render: || budget::MonthlyBudgetPanel().into_any(),
        summary: Some(budget::summary),
        shown: None,
    },
    PanelDef {
        id: "overview.week",
        title: "The next seven days",
        page: PAGE,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: budget::week,
        summary: Some(budget::week_summary),
        shown: Some(budget::week_shown),
    },
    PanelDef {
        id: "overview.trajectory",
        title: "Spending trajectory",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: budget::trajectory,
        summary: None,
        shown: Some(budget::budget_shown),
    },
    PanelDef {
        id: "overview.extras",
        title: "This month only",
        page: PAGE,
        movable: false,
        initial: Initial::Folded,
        half: false,
        render: budget::extras,
        summary: None,
        shown: Some(budget::budget_shown),
    },
];

const PAGE: &str = "overview";

#[component]
pub fn Overview() -> impl IntoView {
    provide_context(budget::PanelState::new());
    view! {
        {month_closure_banner()}
        <PageView tab="overview" tab_name="Overview" page=PAGE />
    }
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
