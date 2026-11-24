use std::collections::HashSet;

use finance_types::plot::{
    Axis, Chart, Compare, Filters, InsightKind, Measure, MeasureSpec, Period, PeriodBasis,
    PlotData, PlotSpec, Split, Value,
};
use finance_types::report::Currency;
use finance_types::report::closure::{ClosureStatus, MonthPeriod};
use jiff::Zoned;
use rust_decimal::Decimal;

use super::plot_query;
use crate::closure::tests::{account, snapshot};
use crate::model::{Kind, Tx};
use crate::trends::tests::{mapping, tx};
use crate::trends::{local_day, time_trends};
use crate::{Ctx, Snapshot, budget, groups, plans};

fn now() -> Zoned {
    "2026-11-24T12:00:00+01:00[Europe/Amsterdam]"
        .parse()
        .unwrap()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn on(day: &str) -> String {
    format!("{day}T12:00:00+00:00[UTC]")
}

fn named(t: Tx, merchant: &str) -> Tx {
    Tx {
        counterparty_name: Some(merchant.into()),
        ..t
    }
}

fn out(id: &str, amount: &str, category: Option<&str>, day: &str) -> Tx {
    tx(id, amount, Kind::Outflow, category, &on(day))
}

/// Groceries, eating out, a fixed rent, one refund, one own transfer, one
/// income, one future payment, one in dollars.
fn ledger() -> Vec<Tx> {
    let mut usd = out("usd", "15", Some("Restaurant"), "2026-11-12");
    usd.currency = Currency::new("USD");
    vec![
        named(out("g1", "40", Some("Jumbo"), "2026-11-03"), "Jumbo"),
        named(out("g2", "25", Some("Jumbo"), "2026-10-15"), "Jumbo"),
        named(out("g3", "30", Some("Jumbo"), "2026-08-07"), "Jumbo"),
        named(out("e1", "60", Some("Restaurant"), "2026-11-20"), "Bistro"),
        named(out("e2", "35", Some("Restaurant"), "2026-09-12"), "Bistro"),
        out("p1", "8", Some("Pharmacy"), "2026-11-15"),
        out("n1", "12", None, "2026-10-02"),
        out("rent", "800", Some("Rent"), "2026-11-01"),
        out("old", "50", Some("Jumbo"), "2026-03-10"),
        tx("r1", "10", Kind::Refund, Some("Jumbo"), &on("2026-11-05")),
        tx("t1", "500", Kind::Transfer, None, &on("2026-11-10")),
        tx("i1", "3000", Kind::Inflow, None, &on("2026-11-01")),
        // Later today: dated after now.
        tx(
            "f1",
            "999",
            Kind::Outflow,
            Some("Jumbo"),
            "2026-11-24T20:00:00+00:00[UTC]",
        ),
        usd,
    ]
}

fn snap(transactions: Vec<Tx>) -> Snapshot {
    let mut snap = snapshot(
        vec![account("firefly:1"), account("firefly:2")],
        transactions,
        &[],
    );
    snap.group_mappings = vec![
        mapping("Jumbo", "Groceries"),
        mapping("Restaurant", "Eating out"),
    ];
    snap.settings.predicted_bill_categories = vec!["Rent".into()];
    snap
}

fn spec(by: Axis, period: Period, measures: &[Measure]) -> PlotSpec {
    PlotSpec {
        by,
        period,
        measures: measures
            .iter()
            .map(|&measure| MeasureSpec {
                measure,
                ..Default::default()
            })
            .collect(),
        top: 20,
        ..Default::default()
    }
}

fn query(snap: &Snapshot, spec: &PlotSpec) -> PlotData {
    plot_query(&Ctx::new(snap, &now()), spec).unwrap()
}

fn spending(snap: &Snapshot) -> Vec<Tx> {
    let ctx = Ctx::new(snap, &now());
    budget::spending_transactions(&ctx, &plans::planned_payment_views(&ctx))
}

/// The value at `label` in the first series.
fn at(data: &PlotData, label: &str) -> Option<Decimal> {
    let i = data.categories.iter().position(|c| c == label).unwrap();
    data.series[0].values[i]
}

fn total(data: &PlotData) -> Decimal {
    data.series[0].values.iter().flatten().sum()
}

#[test]
fn spent_by_month_equals_time_trends() {
    let snap = snap(ledger());
    let data = query(
        &snap,
        &spec(Axis::Month, Period::Last6Months, &[Measure::Spent]),
    );
    let trends = time_trends(
        &spending(&snap),
        &snap.group_mappings,
        &now(),
        &HashSet::new(),
    );
    let eur = &trends.currencies[0];
    assert_eq!(eur.currency, Currency::eur());
    let expected: Vec<_> = eur
        .months
        .iter()
        .map(|m| Some(m.spending.unwrap_or_default()))
        .collect();
    assert_eq!(data.series[0].values, expected);
    assert_eq!(data.categories, ["Jun", "Jul", "Aug", "Sep", "Oct", "Nov"]);
}

#[test]
fn spent_by_group_equals_time_trends_group_totals() {
    let snap = snap(ledger());
    let data = query(
        &snap,
        &spec(Axis::Group, Period::Last6Months, &[Measure::Spent]),
    );
    let trends = time_trends(
        &spending(&snap),
        &snap.group_mappings,
        &now(),
        &HashSet::new(),
    );
    for group in &trends.currencies[0].groups {
        match group.group.as_deref() {
            Some(name) => assert_eq!(at(&data, name), Some(group.total)),
            None => assert_eq!(
                at(&data, "No group").unwrap() + at(&data, "Uncategorised").unwrap(),
                group.total
            ),
        }
    }
    assert_eq!(at(&data, "Groceries"), Some(dec("85")));
}

#[test]
fn spent_by_weekday_equals_net_of_the_same_rows() {
    let snap = snap(ledger());
    let data = query(
        &snap,
        &spec(Axis::Weekday, Period::Last6Months, &[Measure::Spent]),
    );
    let rows = spending(&snap);
    let zone = now().time_zone().clone();
    let start = jiff::civil::date(2026, 6, 1);
    for (weekday, value) in data.series[0].values.iter().enumerate() {
        let same: Vec<&Tx> = rows
            .iter()
            .filter(|t| matches!(t.kind, Kind::Outflow | Kind::Refund))
            .filter(|t| t.currency == Currency::eur())
            .filter(|t| t.occurred_at.timestamp() <= now().timestamp())
            .filter(|t| {
                let day = local_day(t, &zone);
                day >= start && day.weekday().to_monday_zero_offset() as usize == weekday
            })
            .collect();
        assert_eq!(*value, Some(groups::net(&same)));
    }
    assert_eq!(data.categories.len(), 7);
    let trends = time_trends(&rows, &snap.group_mappings, &now(), &HashSet::new());
    let expected: Vec<_> = trends.currencies[0]
        .weekdays
        .iter()
        .map(|w| Some(w.spending.unwrap_or_default()))
        .collect();
    assert_eq!(data.series[0].values, expected);
}

#[test]
fn refunds_reduce_and_transfers_are_out() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]),
    );
    // 40 + 60 + 8 + 800 − 10; the 500 transfer and the future 999 are out.
    assert_eq!(data.series[0].values, [Some(dec("898"))]);
}

#[test]
fn a_future_payment_is_ignored() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Day, Period::ThisMonth, &[Measure::Largest]),
    );
    assert_eq!(data.categories.last().unwrap(), "24 Nov");
    assert_eq!(data.series[0].values.last().unwrap(), &None);
}

#[test]
fn income_and_net() {
    let data = query(
        &snap(ledger()),
        &spec(
            Axis::Month,
            Period::ThisMonth,
            &[Measure::Income, Measure::Net],
        ),
    );
    assert_eq!(data.series[0].values, [Some(dec("3000"))]);
    assert_eq!(data.series[1].values, [Some(dec("2102"))]);
}

#[test]
fn count_average_largest() {
    let mut s = spec(
        Axis::Month,
        Period::ThisYear,
        &[Measure::Count, Measure::Average, Measure::Largest],
    );
    s.chart = Chart::Table;
    let data = query(&snap(ledger()), &s);
    assert_eq!(data.categories.len(), 11);
    let column =
        |i: usize| -> Vec<Option<Decimal>> { data.series.iter().map(|s| s.values[i]).collect() };
    // November: four purchases, 898 spent, the rent the largest.
    assert_eq!(
        column(10),
        [Some(dec("4")), Some(dec("224.50")), Some(dec("800"))]
    );
    assert_eq!(column(0), [Some(Decimal::ZERO), None, None]);
    assert_eq!(
        data.series.iter().map(|s| s.unit).collect::<Vec<_>>(),
        [
            finance_types::plot::Unit::Count,
            finance_types::plot::Unit::Money,
            finance_types::plot::Unit::Money
        ]
    );
}

#[test]
fn every_bucket_exists_even_when_empty() {
    let snap = snap(ledger());
    let data = query(
        &snap,
        &spec(Axis::Month, Period::Last12Months, &[Measure::Spent]),
    );
    assert_eq!(data.categories.len(), 12);
    assert_eq!(data.categories[0], "Dec 2025");
    assert_eq!(data.series[0].values[0], Some(Decimal::ZERO));
    let days = query(
        &snap,
        &spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(days.categories.len(), 24);
    assert!(days.series[0].values.iter().all(Option::is_some));
    let weeks = query(
        &snap,
        &spec(Axis::Week, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(weeks.categories[0], "26 Oct");
    assert_eq!(
        weeks.series[0].values.iter().flatten().sum::<Decimal>(),
        dec("898")
    );
}

fn thirty_merchants() -> Vec<Tx> {
    (1..=30)
        .map(|i| {
            named(
                out(&format!("m{i}"), &i.to_string(), Some("Shop"), "2026-11-10"),
                &format!("Shop {i}"),
            )
        })
        .collect()
}

#[test]
fn top_n_and_other_sum_to_the_total() {
    let mut s = spec(Axis::Merchant, Period::ThisMonth, &[Measure::Spent]);
    s.top = 5;
    let data = query(&snap(thirty_merchants()), &s);
    assert_eq!(
        data.categories,
        [
            "Shop 30", "Shop 29", "Shop 28", "Shop 27", "Shop 26", "Other"
        ]
    );
    assert_eq!(total(&data), dec("465"));
}

#[test]
fn left_out_counts_what_other_holds() {
    let mut s = spec(Axis::Merchant, Period::ThisMonth, &[Measure::Spent]);
    s.top = 5;
    let data = query(&snap(thirty_merchants()), &s);
    assert_eq!(data.left_out, 25);
    // 1 + 2 + … + 25.
    assert_eq!(at(&data, "Other"), Some(dec("325")));
}

#[test]
fn split_keeps_six_series_at_most() {
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.split = Some(Split::Merchant);
    s.split_top = 6;
    let data = query(&snap(thirty_merchants()), &s);
    assert_eq!(data.series.len(), 6);
    assert_eq!(data.series[0].label, "Shop 30");
    assert_eq!(data.series[5].label, "Other");
    let sum: Decimal = data.series.iter().flat_map(|s| s.values[0]).sum();
    assert_eq!(sum, dec("465"));
    assert_eq!(data.left_out, 25);
}

#[test]
fn filters_combine() {
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.filters = Filters {
        categories: vec!["Jumbo".into()],
        min: Some(dec("26")),
        accounts: vec!["firefly:1".into()],
        ..Default::default()
    };
    let data = query(&snap(ledger()), &s);
    // g1 40 and g3 30; g2 25 and the 10 refund are under the minimum.
    assert_eq!(total(&data), dec("70"));
    s.filters.merchant = "UMB".into();
    assert_eq!(total(&query(&snap(ledger()), &s)), dec("70"));
    s.filters.max = Some(dec("35"));
    assert_eq!(total(&query(&snap(ledger()), &s)), dec("30"));
    s.filters.accounts = vec!["firefly:2".into()];
    assert_eq!(total(&query(&snap(ledger()), &s)), Decimal::ZERO);
}

#[test]
fn a_filter_on_a_missing_category_matches_nothing() {
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.filters.categories = vec!["Nope".into()];
    let data = query(&snap(ledger()), &s);
    assert_eq!(data.series[0].values, vec![Some(Decimal::ZERO); 6]);
}

#[test]
fn variable_only_equals_the_variable_time_trends() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.filters.variable_only = true;
    let data = query(&snap, &s);
    let mut fixed_categories =
        groups::fixed_category_names(&snap.bills, &snap.group_mappings, &snap.subscription_rules);
    fixed_categories.extend(groups::predicted_category_names(
        &snap.settings.predicted_bill_categories,
    ));
    let fixed_ids = groups::non_variable_ids(&snap.transactions, &fixed_categories);
    let variable = time_trends(&spending(&snap), &snap.group_mappings, &now(), &fixed_ids);
    let expected: Vec<_> = variable.currencies[0]
        .months
        .iter()
        .map(|m| Some(m.spending.unwrap_or_default()))
        .collect();
    assert_eq!(data.series[0].values, expected);
    assert_eq!(data.series[0].values[5], Some(dec("98")));
    assert!(data.subtitle.contains("variable only"));
}

#[test]
fn fixed_or_variable_axis_sums_to_spent() {
    let snap = snap(ledger());
    let data = query(
        &snap,
        &spec(Axis::FixedOrVariable, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(at(&data, "Fixed"), Some(dec("800")));
    assert_eq!(at(&data, "Variable"), Some(dec("98")));
    assert_eq!(total(&data), dec("898"));
}

#[test]
fn merchant_spellings_merge_under_one_label() {
    let rows = vec![
        named(out("a", "10", None, "2026-11-02"), "Albert Heijn"),
        named(out("b", "20", None, "2026-11-03"), "Albert  Heijn"),
        named(out("c", "30", None, "2026-11-04"), "ALBERT HEIJN"),
        named(out("d", "5", None, "2026-11-05"), "Albert  Heijn"),
        out("e", "7", None, "2026-11-06"),
    ];
    let data = query(
        &snap(rows),
        &spec(Axis::Merchant, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(data.categories, ["Albert  Heijn", "Unknown"]);
    assert_eq!(data.series[0].values, [Some(dec("65")), Some(dec("7"))]);
}

#[test]
fn the_other_currency_is_a_separate_plot() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.currency = Some("USD".into());
    let data = query(&snap, &s);
    assert_eq!(data.currency, Currency::new("USD"));
    assert_eq!(data.series[0].values, [Some(dec("15"))]);
    s.currency = None;
    assert_eq!(query(&snap, &s).currency, Currency::eur());
}

#[test]
fn an_empty_period_gives_zeroes_and_no_division() {
    let s = spec(
        Axis::Day,
        Period::Between {
            from: "2025-01-01".into(),
            to: "2025-01-31".into(),
        },
        &[Measure::Spent, Measure::Average],
    );
    let data = query(&snap(ledger()), &s);
    assert_eq!(data.categories.len(), 31);
    assert!(
        data.series[0]
            .values
            .iter()
            .all(|v| *v == Some(Decimal::ZERO))
    );
    assert!(data.series[1].values.iter().all(Option::is_none));
    assert!(!data.partial_last);
}

#[test]
fn an_empty_ledger_gives_no_series() {
    let data = query(
        &snap(Vec::new()),
        &spec(Axis::Month, Period::Last6Months, &[Measure::Spent]),
    );
    assert!(data.series.is_empty());
}

#[test]
fn a_bucket_where_refunds_exceed_spending_is_negative() {
    let rows = vec![
        out("a", "20", Some("Jumbo"), "2026-11-02"),
        tx("b", "50", Kind::Refund, Some("Jumbo"), &on("2026-11-03")),
    ];
    let data = query(
        &snap(rows),
        &spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(data.series[0].values, [Some(dec("-30"))]);
}

#[test]
fn an_invalid_spec_is_refused() {
    let s = spec(Axis::Month, Period::ThisMonth, &[]);
    let err = plot_query(&Ctx::new(&snap(ledger()), &now()), &s).unwrap_err();
    assert_eq!(err.to_string(), s.validate().unwrap_err());
}

#[test]
fn last_bucket_is_partial_only_when_unfinished() {
    let snap = snap(ledger());
    let partial = |by, period| query(&snap, &spec(by, period, &[Measure::Spent])).partial_last;
    assert!(partial(Axis::Month, Period::Last6Months));
    assert!(partial(Axis::Day, Period::ThisMonth));
    assert!(!partial(Axis::Month, Period::LastMonth));
    assert!(!partial(Axis::Weekday, Period::ThisMonth));
    assert!(!partial(Axis::Category, Period::ThisMonth));
}

#[test]
fn a_day_axis_over_all_time_is_refused_past_the_limit() {
    let mut rows = ledger();
    rows.push(out("ancient", "5", None, "2020-01-01"));
    let s = spec(Axis::Day, Period::AllTime, &[Measure::Spent]);
    // All time keeps the most recent 366 days and says so.
    let days = query(&snap(rows.clone()), &s);
    assert_eq!(days.categories.len(), 366);
    assert_eq!(
        days.titles.last(),
        Some(&"Tuesday 24 November 2026".to_owned())
    );
    assert!(
        days.subtitle.starts_with("All time, last 366 days shown"),
        "{}",
        days.subtitle
    );
    assert_eq!(total(&days), dec("1050"), "the 2020 row is left out");
    // A Between period over the limit is still refused.
    let between = PlotSpec {
        period: Period::Between {
            from: "2020-01-01".into(),
            to: "2026-11-24".into(),
        },
        ..s.clone()
    };
    let err = plot_query(&Ctx::new(&snap(rows), &now()), &between).unwrap_err();
    assert!(err.to_string().contains("366"), "{err}");
    // Within the limit nothing is cut.
    let fits = query(&snap(ledger()), &s);
    assert!(fits.subtitle.starts_with("All time ·"), "{}", fits.subtitle);
    let months = query(
        &snap(ledger()),
        &spec(Axis::Month, Period::AllTime, &[Measure::Spent]),
    );
    assert_eq!(months.categories.first().unwrap(), "Mar");
    assert_eq!(total(&months), dec("1050"));
}

#[test]
fn other_with_negative_members_still_sums_to_the_total() {
    let mut rows = thirty_merchants();
    rows.push(named(
        tx("back", "50", Kind::Refund, Some("Shop"), &on("2026-11-11")),
        "Refundo",
    ));
    let mut s = spec(Axis::Merchant, Period::ThisMonth, &[Measure::Spent]);
    s.top = 5;
    let data = query(&snap(rows), &s);
    assert_eq!(at(&data, "Other"), Some(dec("275")));
    assert_eq!(total(&data), dec("415"));
}

#[test]
fn a_split_series_key_never_collides_with_other() {
    let mut rows = thirty_merchants();
    rows.push(named(out("o", "100", Some("Shop"), "2026-11-10"), "Other"));
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.split = Some(Split::Merchant);
    let data = query(&snap(rows), &s);
    let keys: HashSet<&str> = data.series.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys.len(), data.series.len());
    assert_eq!(data.series[0].label, "Other");
    assert_eq!(data.series.last().unwrap().label, "Other");
}

#[test]
fn weekday_split_keeps_seven_weekdays_in_order() {
    let mut s = spec(Axis::Week, Period::Last3Months, &[Measure::Spent]);
    s.split = Some(Split::Weekday);
    s.chart = Chart::Heatmap;
    let data = query(&snap(ledger()), &s);
    let labels: Vec<&str> = data.series.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday"
        ]
    );
    assert_eq!(data.left_out, 0);
}

#[test]
fn year_split_runs_in_calendar_order_keeping_the_recent_years() {
    let rows: Vec<Tx> = (2019..=2026)
        .map(|y| {
            out(
                &format!("y{y}"),
                &(3000 - y).to_string(),
                None,
                &format!("{y}-03-01"),
            )
        })
        .collect();
    let mut s = spec(Axis::Month, Period::AllTime, &[Measure::Spent]);
    s.split = Some(Split::Year);
    s.split_top = 6;
    let data = query(&snap(rows), &s);
    let labels: Vec<&str> = data.series.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["2022", "2023", "2024", "2025", "2026", "Other"]);
}

#[test]
fn quarter_and_year_labels() {
    let snap = snap(ledger());
    let quarters = query(
        &snap,
        &spec(Axis::Quarter, Period::ThisYear, &[Measure::Spent]),
    );
    assert_eq!(
        quarters.categories,
        ["Q1 2026", "Q2 2026", "Q3 2026", "Q4 2026"]
    );
    let years = query(&snap, &spec(Axis::Year, Period::AllTime, &[Measure::Spent]));
    assert_eq!(years.categories, ["2026"]);
}

#[test]
fn a_week_spanning_two_years_carries_its_year() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Week, Period::ThisYear, &[Measure::Spent]),
    );
    assert_eq!(data.categories[0], "29 Dec 2025");
    assert_eq!(data.categories[1], "5 Jan");
}

/// The ledger, October closed early on the 26th, and a purchase on the 28th
/// that the budget counts in November.
fn closed_october() -> Snapshot {
    let mut rows = ledger();
    rows.push(named(
        out("late", "70", Some("Jumbo"), "2026-10-28"),
        "Jumbo",
    ));
    let mut snap = snap(rows);
    snap.month_periods = vec![MonthPeriod {
        cutoff_at: Some("2026-10-26T10:00:00Z".into()),
        ..crate::closure::tests::period("2026-10-01", ClosureStatus::Closed)
    }];
    snap
}

fn reference(data: &PlotData) -> &finance_types::plot::PlotSeries {
    data.series.iter().find(|s| s.reference).unwrap()
}

#[test]
fn budget_months_equal_the_monthly_budget_spent() {
    let closed = closed_october();
    let spent = budget_spent(&closed);
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.period_basis = PeriodBasis::Budget;
    assert_eq!(query(&closed, &s).series[0].values, [Some(spent)]);
    assert_eq!(spent, dec("968"));
    s.period_basis = PeriodBasis::Calendar;
    assert_eq!(query(&closed, &s).series[0].values, [Some(dec("898"))]);
    // Day by day, the budget month adds up to the same.
    let mut days = spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]);
    days.period_basis = PeriodBasis::Budget;
    assert_eq!(total(&query(&closed, &days)), spent);
    days.cumulative = true;
    let running = query(&closed, &days).series[0].values.clone();
    assert_eq!(running[running.len() - 2], Some(spent));
    // Booked in November in its own zone, 31 October here: the budget
    // counts it in November, on its first day.
    let mut rows = ledger();
    rows.push(tx(
        "hel",
        "20",
        Kind::Outflow,
        None,
        "2026-11-01T00:30:00+02:00[Europe/Helsinki]",
    ));
    let open = snap(rows);
    let spent = budget_spent(&open);
    assert_eq!(spent, dec("918"));
    days.cumulative = false;
    let data = query(&open, &days);
    assert_eq!(total(&data), spent);
    assert_eq!(data.series[0].values[0], Some(dec("820")));
}

/// The dashboard's spent of the current budget month, in euros.
fn budget_spent(snap: &Snapshot) -> Decimal {
    let ctx = Ctx::new(snap, &now());
    let payments = plans::planned_payment_views(&ctx);
    let spending = budget::spending_transactions(&ctx, &payments);
    let empty = budget::tests::empty_subscriptions();
    let budgets = budget::monthly_budgets(&ctx, &spending, &payments, &empty, &empty);
    let current = ctx.current_month().to_string();
    budgets
        .iter()
        .find(|b| b.plan.month == current && b.plan.currency == Currency::eur())
        .unwrap()
        .spent
}

#[test]
fn average_per_day_divides_by_days_in_the_period_not_days_with_spending() {
    let snap = snap(ledger());
    let this = query(
        &snap,
        &spec(Axis::Month, Period::ThisMonth, &[Measure::AveragePerDay]),
    );
    // 898 over the 24 days so far.
    assert_eq!(this.series[0].values, [Some(dec("37.42"))]);
    let last = query(
        &snap,
        &spec(Axis::Month, Period::LastMonth, &[Measure::AveragePerDay]),
    );
    // 37 over 31 days.
    assert_eq!(last.series[0].values, [Some(dec("1.19"))]);
}

#[test]
fn weekday_average_counts_each_weekday_in_the_period() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Weekday, Period::ThisMonth, &[Measure::AveragePerDay]),
    );
    // 1–24 November: four Mondays, Tuesdays and Sundays, three of the rest.
    assert_eq!(at(&data, "Mon"), Some(dec("0")));
    assert_eq!(at(&data, "Tue"), Some(dec("10")));
    assert_eq!(at(&data, "Thu"), Some(dec("-3.33")));
    assert_eq!(at(&data, "Fri"), Some(dec("20")));
    assert_eq!(at(&data, "Sun"), Some(dec("202")));
}

#[test]
fn the_running_total_ends_at_the_plain_total() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]);
    let plain = query(&snap, &s);
    s.cumulative = true;
    let running = query(&snap, &s);
    assert_eq!(
        running.series[0].values.last().unwrap(),
        &Some(total(&plain))
    );
    assert_eq!(running.series[0].values[0], Some(dec("800")));
    assert_eq!(running.series[0].values[2], Some(dec("840")));
}

#[test]
fn the_period_before_has_the_same_number_of_buckets() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.compare = Compare::Previous;
    let data = query(&snap, &s);
    let before = query(
        &snap,
        &spec(
            Axis::Month,
            Period::Between {
                from: "2025-12-01".into(),
                to: "2026-05-31".into(),
            },
            &[Measure::Spent],
        ),
    );
    assert_eq!(reference(&data).label, "Period before");
    assert_eq!(reference(&data).values, before.series[0].values);
    let mut days = spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]);
    days.compare = Compare::Previous;
    assert_eq!(reference(&query(&snap, &days)).values.len(), 24);
}

#[test]
fn usual_is_the_mean_of_three_periods() {
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.compare = Compare::AverageOf3;
    let data = query(&snap(ledger()), &s);
    // October 37, September 35, August 30.
    assert_eq!(reference(&data).label, "Usual");
    assert_eq!(reference(&data).values, [Some(dec("34.00"))]);
}

#[test]
fn the_average_line_ignores_the_unfinished_bucket() {
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.average_line = true;
    let data = query(&snap(ledger()), &s);
    // June to October: 0 + 0 + 30 + 35 + 37, over five.
    assert_eq!(reference(&data).values, vec![Some(dec("20.40")); 6]);
}

#[test]
fn change_is_this_period_minus_the_one_before() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::Last3Months, &[Measure::Spent]);
    s.value = Value::Change;
    // September − August, October − September, November − October.
    assert_eq!(
        query(&snap, &s).series[0].values,
        [Some(dec("5")), Some(dec("2")), Some(dec("861"))]
    );
    let mut named = spec(Axis::Category, Period::ThisMonth, &[Measure::Spent]);
    named.value = Value::Change;
    let data = query(&snap, &named);
    assert_eq!(at(&data, "Jumbo"), Some(dec("5")));
    assert_eq!(at(&data, "Uncategorised"), Some(dec("-12")));
}

#[test]
fn change_ranks_by_size_of_change() {
    let mut s = spec(Axis::Category, Period::ThisMonth, &[Measure::Spent]);
    s.value = Value::Change;
    let data = query(&snap(ledger()), &s);
    assert_eq!(
        data.categories,
        ["Rent", "Restaurant", "Uncategorised", "Pharmacy", "Jumbo"]
    );
}

#[test]
fn insight_for_time_uses_the_last_finished_bucket() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Month, Period::Last6Months, &[Measure::Spent]),
    );
    let insight = data.insight.unwrap();
    assert_eq!(insight.kind, InsightKind::LastAgainstAverage);
    assert_eq!(insight.label, "October 2026");
    assert_eq!(insight.value, dec("37"));
    // June to September: 65 over four.
    assert_eq!(insight.against, Some(dec("16.25")));
}

#[test]
fn insight_is_absent_with_two_finished_buckets() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Month, Period::Last3Months, &[Measure::Spent]),
    );
    assert_eq!(data.insight, None);
}

#[test]
fn insight_for_named_axis_is_the_largest_share() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Category, Period::ThisMonth, &[Measure::Spent]),
    );
    let insight = data.insight.unwrap();
    assert_eq!(insight.kind, InsightKind::LargestShare);
    assert_eq!(insight.label, "Rent");
    assert_eq!(insight.value, dec("800"));
    assert_eq!(insight.against, Some(dec("898")));
}

#[test]
fn insight_for_weekday_is_peak_against_lowest() {
    let data = query(
        &snap(ledger()),
        &spec(Axis::Weekday, Period::ThisMonth, &[Measure::Spent]),
    );
    let insight = data.insight.unwrap();
    assert_eq!(insight.kind, InsightKind::PeakAgainstLowest);
    assert_eq!(insight.label, "Sunday");
    assert_eq!(insight.value, dec("808"));
    // Tuesday; Thursday's refund is not a lowest positive day.
    assert_eq!(insight.against, Some(dec("40")));
}

#[test]
fn the_unfinished_bucket_is_never_the_peak() {
    let mut rows = ledger();
    rows.push(tx(
        "big",
        "5000",
        Kind::Outflow,
        None,
        "2026-11-24T08:00:00+00:00[UTC]",
    ));
    let data = query(
        &snap(rows),
        &spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(data.series[0].values.last().unwrap(), &Some(dec("5000")));
    let insight = data.insight.unwrap();
    assert_eq!(insight.label, "Monday 23 November 2026");
    assert_eq!(insight.value, Decimal::ZERO);
}

#[test]
fn insight_with_compare_uses_matching_positions() {
    let mut rows = ledger();
    // After 24 October: no counterpart of the 24 days so far.
    rows.push(out("x", "100", None, "2026-10-28"));
    // Today and its counterpart: today is not finished, so neither counts.
    rows.push(tx(
        "today",
        "5",
        Kind::Outflow,
        None,
        "2026-11-24T08:00:00+00:00[UTC]",
    ));
    rows.push(out("oct24", "200", None, "2026-10-24"));
    let mut s = spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]);
    s.compare = Compare::Previous;
    let insight = query(&snap(rows), &s).insight.unwrap();
    assert_eq!(insight.kind, InsightKind::AgainstPrevious);
    assert_eq!(insight.value, dec("898"));
    assert_eq!(insight.against, Some(dec("37")));
}

#[test]
fn insight_with_compare_on_a_month_axis_leaves_out_this_month() {
    let snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.compare = Compare::Previous;
    assert_eq!(query(&snap, &s).insight, None);
    s.period = Period::Last6Months;
    let insight = query(&snap, &s).insight.unwrap();
    // June to October against December to April.
    assert_eq!(insight.value, dec("102"));
    assert_eq!(insight.against, Some(dec("50")));
}

#[test]
fn year_on_year_average_per_day_divides_by_each_years_days() {
    let mut rows = two_years();
    rows.push(out("n26", "48", None, "2026-11-05"));
    let mut s = spec(
        Axis::MonthOfYear,
        Period::AllTime,
        &[Measure::AveragePerDay],
    );
    s.split = Some(Split::Year);
    let data = query(&snap(rows), &s);
    let (last_year, this_year) = (&data.series[0].values, &data.series[1].values);
    // All time starts on 10 January 2025: 10 over 22 days; then 20 over 31.
    assert_eq!(last_year[0], Some(dec("0.45")));
    assert_eq!(this_year[0], Some(dec("0.65")));
    // November so far: 24 days.
    assert_eq!(this_year[10], Some(dec("2.00")));
}

#[test]
fn weekday_split_on_a_week_axis_divides_by_one_day() {
    let mut s = spec(Axis::Week, Period::ThisMonth, &[Measure::AveragePerDay]);
    s.split = Some(Split::Weekday);
    let data = query(&snap(ledger()), &s);
    let tuesday = &data.series[1];
    assert_eq!(tuesday.label, "Tuesday");
    // Week of 2 November: 40 on Tuesday the 3rd.
    assert_eq!(tuesday.values[1], Some(dec("40.00")));
}

#[test]
fn change_leaves_a_cut_first_bucket_empty() {
    let snap = snap(ledger());
    let mut quarters = spec(Axis::Quarter, Period::Last6Months, &[Measure::Spent]);
    quarters.value = Value::Change;
    let data = query(&snap, &quarters);
    // June alone is cut out of Q2; Q3 (30 + 35) against the whole Q2 (0).
    assert_eq!(data.series[0].values[0], None);
    assert_eq!(data.series[0].values[1], Some(dec("65")));
    let mut weeks = spec(Axis::Week, Period::ThisMonth, &[Measure::Spent]);
    weeks.value = Value::Change;
    let data = query(&snap, &weeks);
    // 1 November is a Sunday. The week of 2 November (40 − 10) against the
    // whole week of 26 October (the 800 rent on the 1st).
    assert_eq!(data.series[0].values[0], None);
    assert_eq!(data.series[0].values[1], Some(dec("-770")));
    // A ledger that starts in November has no whole week of 26 October.
    let fresh = query(
        &self::snap(vec![
            out("a", "10", None, "2026-11-01"),
            out("b", "20", None, "2026-11-03"),
        ]),
        &weeks,
    );
    assert_eq!(fresh.series[0].values[0], None);
    assert_eq!(fresh.series[0].values[1], None);
    assert!(fresh.series[0].values[2].is_some());
}

#[test]
fn a_period_ending_after_today_stops_at_today() {
    let mut s = spec(
        Axis::Day,
        Period::Between {
            from: "2026-11-01".into(),
            to: "2026-12-31".into(),
        },
        &[Measure::Spent],
    );
    let snap = snap(ledger());
    let data = query(&snap, &s);
    assert_eq!(data.categories.len(), 61);
    assert_eq!(data.series[0].values[23], Some(Decimal::ZERO));
    assert!(data.series[0].values[24..].iter().all(Option::is_none));
    assert!(!data.partial_last);
    s.cumulative = true;
    let running = query(&snap, &s);
    assert_eq!(running.series[0].values[23], Some(dec("898")));
    assert!(running.series[0].values[24..].iter().all(Option::is_none));
}

#[test]
fn usual_averages_only_periods_the_ledger_reaches() {
    let rows = vec![
        out("a", "30", None, "2026-10-05"),
        out("b", "40", None, "2026-11-03"),
    ];
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::Spent]);
    s.compare = Compare::AverageOf3;
    let data = query(&snap(rows), &s);
    assert_eq!(reference(&data).values, [Some(dec("30.00"))]);
    let fresh = query(&snap(vec![out("b", "40", None, "2026-11-03")]), &s);
    assert_eq!(reference(&fresh).values, [None]);
}

#[test]
fn insight_sums_only_measures_that_add_up() {
    let snap = snap(ledger());
    let average = query(
        &snap,
        &spec(Axis::Month, Period::Last6Months, &[Measure::Average]),
    );
    assert_eq!(average.insight, None);
    let per_day = query(
        &snap,
        &spec(Axis::Weekday, Period::ThisMonth, &[Measure::AveragePerDay]),
    );
    assert_eq!(
        per_day.insight.unwrap().kind,
        InsightKind::PeakAgainstLowest
    );
}

#[test]
fn peak_against_itself_is_no_insight() {
    let data = query(
        &snap(vec![out("a", "30", None, "2026-11-03")]),
        &spec(Axis::Weekday, Period::ThisMonth, &[Measure::Spent]),
    );
    assert_eq!(data.insight, None);
}

#[test]
fn an_average_line_with_a_running_total_is_refused() {
    let mut s = spec(Axis::Month, Period::Last6Months, &[Measure::Spent]);
    s.average_line = true;
    s.cumulative = true;
    assert!(plot_query(&Ctx::new(&snap(ledger()), &now()), &s).is_err());
}

#[test]
fn the_average_line_follows_the_first_measure() {
    let mut s = spec(
        Axis::Month,
        Period::Last6Months,
        &[Measure::Spent, Measure::Income],
    );
    s.average_line = true;
    let data = query(&snap(ledger()), &s);
    assert_eq!(reference(&data).label, "Average spent");
    assert_eq!(reference(&data).values[0], Some(dec("20.40")));
}

#[test]
fn average_per_day_under_budget_months() {
    let mut s = spec(Axis::Month, Period::ThisMonth, &[Measure::AveragePerDay]);
    s.period_basis = PeriodBasis::Budget;
    let data = query(&closed_october(), &s);
    // 968 over 27 October to 24 November: 29 days.
    assert_eq!(data.series[0].values, [Some(dec("33.38"))]);
}

#[test]
fn usual_with_a_running_total_is_the_running_mean() {
    let mut s = spec(Axis::Day, Period::ThisMonth, &[Measure::Spent]);
    s.compare = Compare::AverageOf3;
    s.cumulative = true;
    let data = query(&snap(ledger()), &s);
    let usual = &reference(&data).values;
    // 2 October's 12 over three periods.
    assert_eq!(usual[1], Some(dec("4.00")));
    // October 37, September 35, August 30, by the 24th.
    assert_eq!(usual[23], Some(dec("34.00")));
}

#[test]
fn month_of_year_under_budget_months() {
    let mut s = spec(Axis::MonthOfYear, Period::ThisYear, &[Measure::Spent]);
    s.period_basis = PeriodBasis::Budget;
    let snap = closed_october();
    let data = query(&snap, &s);
    assert_eq!(at(&data, "Oct"), Some(dec("37")));
    assert_eq!(at(&data, "Nov"), Some(dec("968")));
    s.period_basis = PeriodBasis::Calendar;
    let calendar = query(&snap, &s);
    assert_eq!(at(&calendar, "Oct"), Some(dec("107")));
}

#[test]
fn comparing_or_changing_all_time_is_refused() {
    let ctx_snap = snap(ledger());
    let mut s = spec(Axis::Month, Period::AllTime, &[Measure::Spent]);
    s.compare = Compare::Previous;
    assert!(plot_query(&Ctx::new(&ctx_snap, &now()), &s).is_err());
    s.compare = Compare::None;
    s.value = Value::Change;
    assert!(plot_query(&Ctx::new(&ctx_snap, &now()), &s).is_err());
}

fn two_years() -> Vec<Tx> {
    vec![
        out("j25", "10", None, "2025-01-10"),
        out("d25", "7", None, "2025-12-10"),
        out("j26", "20", None, "2026-01-10"),
        out("m26", "5", None, "2026-03-10"),
    ]
}

#[test]
fn month_of_year_buckets_fold_the_years_together() {
    let data = query(
        &snap(two_years()),
        &spec(Axis::MonthOfYear, Period::AllTime, &[Measure::Spent]),
    );
    assert_eq!(data.categories.len(), 12);
    assert_eq!(data.titles[0], "January");
    assert_eq!(at(&data, "Jan"), Some(dec("30")));
    assert_eq!(at(&data, "Mar"), Some(dec("5")));
    assert_eq!(at(&data, "Dec"), Some(dec("7")));
    assert!(!data.partial_last);
}

#[test]
fn year_on_year_gives_one_series_per_year_over_twelve_months() {
    let mut s = spec(Axis::MonthOfYear, Period::AllTime, &[Measure::Spent]);
    s.split = Some(Split::Year);
    s.chart = Chart::Line;
    let data = query(&snap(two_years()), &s);
    let labels: Vec<&str> = data.series.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["2025", "2026"]);
    let (last_year, this_year) = (&data.series[0].values, &data.series[1].values);
    assert_eq!(last_year.len(), 12);
    assert_eq!(last_year[0], Some(dec("10")));
    assert_eq!(last_year[1], Some(Decimal::ZERO));
    assert_eq!(last_year[11], Some(dec("7")));
    assert_eq!(this_year.len(), 12);
    assert_eq!(this_year[0], Some(dec("20")));
    assert_eq!(this_year[1], Some(Decimal::ZERO));
    // November has started; December has not.
    assert_eq!(this_year[10], Some(Decimal::ZERO));
    assert_eq!(this_year[11], None);
}

#[test]
fn month_of_year_insight_is_peak_against_lowest() {
    let data = query(
        &snap(two_years()),
        &spec(Axis::MonthOfYear, Period::AllTime, &[Measure::Spent]),
    );
    let insight = data.insight.unwrap();
    assert_eq!(insight.kind, InsightKind::PeakAgainstLowest);
    assert_eq!(insight.label, "January");
    assert_eq!(insight.value, dec("30"));
    assert_eq!(insight.against, Some(dec("5")));
}

/// The ledger, November closed early on the 21st, and a purchase on the
/// 22nd that the budget counts in December, the running budget month.
fn closed_november() -> Snapshot {
    let mut rows = ledger();
    rows.push(named(
        out("late", "70", Some("Jumbo"), "2026-11-22"),
        "Jumbo",
    ));
    let mut snap = snap(rows);
    snap.month_periods = vec![MonthPeriod {
        cutoff_at: Some("2026-11-21T10:00:00Z".into()),
        ..crate::closure::tests::period("2026-11-01", ClosureStatus::Closed)
    }];
    snap
}

#[test]
fn budget_months_over_all_time_or_between_run_to_the_running_budget_month() {
    let snap = closed_november();
    let between = Period::Between {
        from: "2026-11-01".into(),
        to: "2026-11-24".into(),
    };
    for (period, expected) in [(Period::AllTime, "1120"), (between, "968")] {
        let budget = |by| {
            let mut s = spec(by, period.clone(), &[Measure::Spent]);
            s.period_basis = PeriodBasis::Budget;
            query(&snap, &s)
        };
        let months = budget(Axis::Month);
        assert_eq!(total(&months), dec(expected), "{period:?}");
        assert_eq!(total(&budget(Axis::Weekday)), dec(expected));
        assert_eq!(total(&budget(Axis::Category)), dec(expected));
        assert_eq!(months.titles.last().unwrap(), "December 2026");
        assert!(months.partial_last, "{period:?}");
        assert_eq!(total(&budget(Axis::Quarter)), dec(expected));
    }
    let mut s = spec(Axis::Month, Period::AllTime, &[Measure::Spent]);
    s.period_basis = PeriodBasis::Budget;
    let insight = query(&snap, &s).insight.unwrap();
    assert_eq!(insight.label, "November 2026");
}

#[test]
fn budget_months_over_all_time_keep_at_most_240_buckets() {
    let mut rows = ledger();
    rows.push(out("ancient", "5", None, "2000-01-01"));
    let mut snap = snap(rows);
    // November 2006 closed early: the oldest month shown, December 2006,
    // starts on 20 November.
    snap.month_periods = vec![MonthPeriod {
        cutoff_at: Some("2006-11-20T10:00:00Z".into()),
        ..crate::closure::tests::period("2006-11-01", ClosureStatus::Closed)
    }];
    let mut s = spec(Axis::Month, Period::AllTime, &[Measure::Spent]);
    s.period_basis = PeriodBasis::Budget;
    let data = query(&snap, &s);
    assert_eq!(data.categories.len(), 240);
    assert_eq!(data.titles[0], "December 2006");
}

/// A ledger that starts on 10 August 2026.
fn young() -> Snapshot {
    snap(vec![
        out("a", "30", None, "2026-08-10"),
        out("s", "60", None, "2026-09-10"),
        out("o", "90", None, "2026-10-05"),
    ])
}

#[test]
fn months_before_the_ledger_starts_are_drawn_as_zero_but_not_averaged() {
    let mut s = spec(Axis::Month, Period::Last12Months, &[Measure::Spent]);
    let data = query(&young(), &s);
    // December 2025 is before the ledger: drawn as zero.
    assert_eq!(data.series[0].values[0], Some(Decimal::ZERO));
    let insight = data.insight.unwrap();
    assert_eq!(insight.label, "October 2026");
    // August and September only.
    assert_eq!(insight.against, Some(dec("45.00")));
    s.average_line = true;
    // August to October.
    assert_eq!(
        reference(&query(&young(), &s)).values[0],
        Some(dec("60.00"))
    );
}

#[test]
fn usual_leaves_out_months_before_the_ledger_starts() {
    let mut s = spec(Axis::Month, Period::Last3Months, &[Measure::Spent]);
    s.compare = Compare::AverageOf3;
    // Against June, July and August: only August is in the ledger.
    assert_eq!(
        reference(&query(&young(), &s)).values,
        [None, None, Some(dec("30.00"))]
    );
}

#[test]
fn average_per_day_counts_days_from_the_first_transaction() {
    let data = query(
        &young(),
        &spec(
            Axis::Category,
            Period::Last12Months,
            &[Measure::AveragePerDay],
        ),
    );
    // 180 over 10 August to 24 November: 107 days.
    assert_eq!(data.series[0].values, [Some(dec("1.68"))]);
}

/// The ledger with 5 spent earlier today and 10 on Saturday 21 November.
fn spent_today() -> Snapshot {
    let mut rows = ledger();
    rows.push(tx(
        "today",
        "5",
        Kind::Outflow,
        None,
        "2026-11-24T08:00:00+00:00[UTC]",
    ));
    rows.push(out("sat", "10", None, "2026-11-21"));
    snap(rows)
}

#[test]
fn the_running_month_of_year_is_never_the_peak_or_the_lowest() {
    let data = query(
        &spent_today(),
        &spec(Axis::MonthOfYear, Period::ThisYear, &[Measure::Spent]),
    );
    let insight = data.insight.unwrap();
    // November (913) is still running.
    assert_eq!(insight.label, "March");
    assert_eq!(insight.against, Some(dec("30")));
}

#[test]
fn a_day_or_weekday_occurring_only_today_is_not_named() {
    let snap = spent_today();
    let days = query(
        &snap,
        &spec(Axis::DayOfMonth, Period::ThisMonth, &[Measure::Spent]),
    );
    // Day 24 (5) is only today; the lowest is day 15 (8).
    let insight = days.insight.unwrap();
    assert_eq!(insight.label, "Day 1");
    assert_eq!(insight.against, Some(dec("8")));
    let weekdays = query(
        &snap,
        &spec(
            Axis::Weekday,
            Period::Between {
                from: "2026-11-20".into(),
                to: "2026-11-24".into(),
            },
            &[Measure::Spent],
        ),
    );
    // Tuesday occurs only today: Friday 60 against Saturday 10.
    let insight = weekdays.insight.unwrap();
    assert_eq!(insight.label, "Friday");
    assert_eq!(insight.against, Some(dec("10")));
}
