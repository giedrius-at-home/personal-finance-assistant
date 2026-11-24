//! A plot the owner builds: what is measured, along which axis, split by what,
//! over which period, drawn how; and the numbers a plot query returns.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::report::Currency;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    #[default]
    Spent,
    Income,
    Net,
    Count,
    Average,
    Largest,
    AveragePerDay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Draw {
    #[default]
    Bars,
    Line,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MeasureSpec {
    pub measure: Measure,
    pub draw: Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Day,
    Week,
    #[default]
    Month,
    Quarter,
    Year,
    Weekday,
    DayOfMonth,
    /// January to December, every year folded together.
    MonthOfYear,
    Category,
    Group,
    Merchant,
    Account,
    FixedOrVariable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    #[default]
    Category,
    Group,
    Merchant,
    Account,
    Weekday,
    Year,
    FixedOrVariable,
}

impl Split {
    /// The axis that shows the same thing.
    fn axis(self) -> Axis {
        match self {
            Self::Category => Axis::Category,
            Self::Group => Axis::Group,
            Self::Merchant => Axis::Merchant,
            Self::Account => Axis::Account,
            Self::Weekday => Axis::Weekday,
            Self::Year => Axis::Year,
            Self::FixedOrVariable => Axis::FixedOrVariable,
        }
    }
}

/// Dates in `Between` are `YYYY-MM-DD`, both included.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    ThisMonth,
    LastMonth,
    Last3Months,
    #[default]
    Last6Months,
    Last12Months,
    ThisYear,
    AllTime,
    Between {
        from: String,
        to: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeriodBasis {
    #[default]
    Calendar,
    Budget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Filters {
    pub categories: Vec<String>,
    pub groups: Vec<String>,
    pub accounts: Vec<String>,
    pub merchant: String,
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
    pub variable_only: bool,
}

/// In menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Chart {
    #[default]
    Bars,
    StackedBars,
    Line,
    Area,
    Donut,
    Ranked,
    Heatmap,
    Scatter,
    Table,
}

const CHARTS: [Chart; 9] = [
    Chart::Bars,
    Chart::StackedBars,
    Chart::Line,
    Chart::Area,
    Chart::Donut,
    Chart::Ranked,
    Chart::Heatmap,
    Chart::Scatter,
    Chart::Table,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    #[default]
    None,
    Previous,
    AverageOf3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    #[default]
    Amount,
    Change,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlotSpec {
    pub measures: Vec<MeasureSpec>,
    pub by: Axis,
    pub split: Option<Split>,
    /// Items shown on a named axis, 1 to 20.
    pub top: u8,
    /// Series shown for a split, 1 to 6.
    pub split_top: u8,
    pub period: Period,
    pub period_basis: PeriodBasis,
    pub filters: Filters,
    pub chart: Chart,
    pub value: Value,
    pub cumulative: bool,
    pub compare: Compare,
    pub average_line: bool,
    pub currency: Option<String>,
}

impl Default for PlotSpec {
    fn default() -> Self {
        Self {
            measures: vec![MeasureSpec::default()],
            by: Axis::default(),
            split: None,
            top: 10,
            split_top: 5,
            period: Period::default(),
            period_basis: PeriodBasis::default(),
            filters: Filters::default(),
            chart: Chart::default(),
            value: Value::default(),
            cumulative: false,
            compare: Compare::default(),
            average_line: false,
            currency: None,
        }
    }
}

impl PlotSpec {
    /// `Err` with a short reason when the choices don't fit together.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.measures.len();
        if !(1..=3).contains(&n) {
            return Err("Pick one to three measures.".into());
        }
        if n > 1 && self.split.is_some() {
            return Err("Several measures can't be split.".into());
        }
        if !(1..=20).contains(&self.top) {
            return Err("Show 1 to 20 items.".into());
        }
        if !(1..=6).contains(&self.split_top) {
            return Err("Split into 1 to 6 series.".into());
        }
        if self.split.is_some_and(|s| s.axis() == self.by) {
            return Err("Split by something other than the axis.".into());
        }
        if self.value == Value::Change {
            if !self.named_axis() && !self.time_axis() {
                return Err("Change needs a named or time axis.".into());
            }
            if self.compare != Compare::None {
                return Err("Change can't be compared.".into());
            }
        }
        if self.cumulative && !self.time_axis() {
            return Err("A running total needs a time axis.".into());
        }
        if self.cumulative && self.value == Value::Change {
            return Err("A running total can't show change.".into());
        }
        if self.compare != Compare::None && (!self.time_axis() || self.split.is_some()) {
            return Err("Comparing needs a time axis and no split.".into());
        }
        if let Period::Between { from, to } = &self.period {
            let (Some(a), Some(b)) = (day_number(from), day_number(to)) else {
                return Err("Dates must be real days written YYYY-MM-DD.".into());
            };
            if a > b {
                return Err("The period ends before it starts.".into());
            }
            let (count, limit, unit) = match self.by {
                Axis::Day => (b - a + 1, 366, "days"),
                // Days since 1970-01-01, a Thursday: +3 makes Monday start each week.
                Axis::Week => (
                    (b + 3).div_euclid(7) - (a + 3).div_euclid(7) + 1,
                    260,
                    "weeks",
                ),
                Axis::Month => (month_number(to) - month_number(from) + 1, 240, "months"),
                _ => (0, 0, ""),
            };
            if count > limit {
                return Err(format!("A plot can show at most {limit} {unit}."));
            }
        }
        let one_unsplit = n == 1 && self.split.is_none();
        let counts = self
            .measures
            .iter()
            .filter(|m| m.measure == Measure::Count)
            .count();
        // Money and counts can't share one scale.
        let mixed = counts > 0 && counts < n;
        match self.chart {
            Chart::Scatter | Chart::Table => {}
            _ if mixed => return Err("Counts and amounts only go together in a scatter.".into()),
            _ => {}
        }
        match self.chart {
            Chart::Scatter if n != 2 || self.split.is_some() => {
                Err("A scatter needs two measures and no split.".into())
            }
            Chart::Scatter
                if self.compare != Compare::None || self.cumulative || self.average_line =>
            {
                Err("A scatter can't compare, add up or draw an average.".into())
            }
            Chart::Donut | Chart::Ranked if !self.named_axis() || !one_unsplit => {
                Err("This chart needs a named axis, one measure and no split.".into())
            }
            Chart::Donut if self.value == Value::Change => Err("A donut can't show change.".into()),
            Chart::Line | Chart::Area if self.named_axis() => {
                Err("Lines need a time, weekday or day-of-month axis.".into())
            }
            Chart::Heatmap if self.split.is_none() || n != 1 => {
                Err("A heatmap needs a split and one measure.".into())
            }
            Chart::StackedBars if self.split.is_none() && n < 2 => {
                Err("Stacked bars need a split or several measures.".into())
            }
            _ => Ok(()),
        }
    }

    /// The drawings that fit the other choices, in menu order.
    pub fn charts(&self) -> Vec<Chart> {
        CHARTS
            .into_iter()
            .filter(|&chart| {
                PlotSpec {
                    chart,
                    ..self.clone()
                }
                .validate()
                .is_ok()
            })
            .collect()
    }

    pub fn named_axis(&self) -> bool {
        matches!(
            self.by,
            Axis::Category | Axis::Group | Axis::Merchant | Axis::Account | Axis::FixedOrVariable
        )
    }

    pub fn time_axis(&self) -> bool {
        matches!(
            self.by,
            Axis::Day | Axis::Week | Axis::Month | Axis::Quarter | Axis::Year
        )
    }
}

/// `YYYY-MM-DD` as (year, month, day), `None` unless it is a real day.
fn ymd(s: &str) -> Option<(i64, i64, i64)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&d).then_some((y, m, d))
}

/// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
fn day_number(s: &str) -> Option<i64> {
    let (y, m, d) = ymd(s)?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Months since year 0; only called on dates that already parsed.
fn month_number(s: &str) -> i64 {
    ymd(s).map_or(0, |(y, m, _)| y * 12 + m)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Money,
    Count,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PlotSeries {
    pub key: String,
    pub label: String,
    pub draw: Draw,
    /// One per category; `None` where there is nothing to show.
    pub values: Vec<Option<Decimal>>,
    /// A comparison or average line, not one of the measured series.
    pub reference: bool,
    /// Money or a count; series in one plot may differ (a scatter or table).
    pub unit: Unit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InsightKind {
    #[default]
    LastAgainstAverage,
    LargestShare,
    PeakAgainstLowest,
    AgainstPrevious,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Insight {
    pub kind: InsightKind,
    pub label: String,
    pub value: Decimal,
    pub against: Option<Decimal>,
    pub text: String,
}

/// What a plot query returns. `currency` has no natural default, so it is required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlotData {
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub titles: Vec<String>,
    #[serde(default)]
    pub series: Vec<PlotSeries>,
    /// The last bucket is still running (this month, this week, today).
    #[serde(default)]
    pub partial_last: bool,
    #[serde(default)]
    pub insight: Option<Insight>,
    pub currency: Currency,
    /// The unit of the first series.
    #[serde(default)]
    pub unit: Unit,
    #[serde(default)]
    pub subtitle: String,
    /// Items not shown (past `top`, or folded into "Other").
    #[serde(default)]
    pub left_out: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> PlotSpec {
        PlotSpec::default()
    }

    fn m(measure: Measure) -> MeasureSpec {
        MeasureSpec {
            measure,
            draw: Draw::Bars,
        }
    }

    fn between(by: Axis, from: &str, to: &str) -> PlotSpec {
        PlotSpec {
            by,
            period: Period::Between {
                from: from.into(),
                to: to.into(),
            },
            ..spec()
        }
    }

    #[test]
    fn an_empty_object_is_the_default_spec() {
        let s: PlotSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(s.measures, vec![m(Measure::Spent)]);
        assert_eq!(s.by, Axis::Month);
        assert_eq!(s.period, Period::Last6Months);
        assert_eq!(s.period_basis, PeriodBasis::Calendar);
        assert_eq!(s.chart, Chart::Bars);
        assert_eq!((s.top, s.split_top), (10, 5));
        assert_eq!(s, PlotSpec::default());
        assert!(s.validate().is_ok());
    }

    #[test]
    fn one_to_three_measures() {
        let with = |n| PlotSpec {
            measures: vec![m(Measure::Spent); n],
            ..spec()
        };
        assert!(with(0).validate().is_err());
        assert!(with(1).validate().is_ok());
        assert!(with(3).validate().is_ok());
        assert!(with(4).validate().is_err());
    }

    #[test]
    fn several_measures_only_without_a_split() {
        let s = PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Income)],
            ..spec()
        };
        assert!(s.validate().is_ok());
        let split = PlotSpec {
            split: Some(Split::Category),
            ..s
        };
        assert!(split.validate().is_err());
    }

    #[test]
    fn scatter_needs_two_measures_and_no_split() {
        let two = PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Count)],
            by: Axis::Merchant,
            chart: Chart::Scatter,
            ..spec()
        };
        assert!(two.validate().is_ok());
        let one = PlotSpec {
            measures: vec![m(Measure::Spent)],
            ..two.clone()
        };
        assert!(one.validate().is_err());
        let split = PlotSpec {
            split: Some(Split::Year),
            ..one
        };
        assert!(split.validate().is_err());
    }

    #[test]
    fn donut_and_ranked_need_a_named_axis_one_measure_and_no_split() {
        for chart in [Chart::Donut, Chart::Ranked] {
            let ok = PlotSpec {
                by: Axis::Category,
                chart,
                ..spec()
            };
            assert!(ok.validate().is_ok());
            let time = PlotSpec {
                by: Axis::Month,
                ..ok.clone()
            };
            assert!(time.validate().is_err());
            let two = PlotSpec {
                measures: vec![m(Measure::Spent), m(Measure::Income)],
                ..ok.clone()
            };
            assert!(two.validate().is_err());
            let split = PlotSpec {
                split: Some(Split::Account),
                ..ok
            };
            assert!(split.validate().is_err());
        }
    }

    #[test]
    fn heatmap_needs_a_split_and_one_measure() {
        let ok = PlotSpec {
            by: Axis::Weekday,
            split: Some(Split::Year),
            chart: Chart::Heatmap,
            ..spec()
        };
        assert!(ok.validate().is_ok());
        let unsplit = PlotSpec { split: None, ..ok };
        assert!(unsplit.validate().is_err());
    }

    #[test]
    fn stacked_bars_need_a_split_or_several_measures() {
        let alone = PlotSpec {
            chart: Chart::StackedBars,
            ..spec()
        };
        assert!(alone.validate().is_err());
        let split = PlotSpec {
            split: Some(Split::Category),
            ..alone.clone()
        };
        assert!(split.validate().is_ok());
        let two = PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Income)],
            ..alone
        };
        assert!(two.validate().is_ok());
    }

    #[test]
    fn change_needs_a_named_or_time_axis_and_no_compare() {
        let with = |by| PlotSpec {
            by,
            value: Value::Change,
            ..spec()
        };
        assert!(with(Axis::Month).validate().is_ok());
        assert!(with(Axis::Category).validate().is_ok());
        assert!(with(Axis::Weekday).validate().is_err());
        assert!(with(Axis::DayOfMonth).validate().is_err());
        assert!(with(Axis::MonthOfYear).validate().is_err());
        let compared = PlotSpec {
            compare: Compare::Previous,
            ..with(Axis::Month)
        };
        assert!(compared.validate().is_err());
    }

    #[test]
    fn cumulative_needs_a_time_axis() {
        let with = |by| PlotSpec {
            by,
            cumulative: true,
            ..spec()
        };
        assert!(with(Axis::Day).validate().is_ok());
        assert!(with(Axis::Category).validate().is_err());
        assert!(with(Axis::Weekday).validate().is_err());
        assert!(with(Axis::MonthOfYear).validate().is_err());
    }

    #[test]
    fn compare_needs_a_time_axis_and_no_split() {
        let with = |by| PlotSpec {
            by,
            compare: Compare::AverageOf3,
            ..spec()
        };
        assert!(with(Axis::Week).validate().is_ok());
        assert!(with(Axis::Merchant).validate().is_err());
        assert!(with(Axis::MonthOfYear).validate().is_err());
        let split = PlotSpec {
            split: Some(Split::Group),
            ..with(Axis::Week)
        };
        assert!(split.validate().is_err());
    }

    #[test]
    fn top_and_split_top_stay_in_range() {
        let with = |top, split_top| PlotSpec {
            top,
            split_top,
            ..spec()
        };
        assert!(with(1, 1).validate().is_ok());
        assert!(with(20, 6).validate().is_ok());
        assert!(with(0, 5).validate().is_err());
        assert!(with(21, 5).validate().is_err());
        assert!(with(10, 0).validate().is_err());
        assert!(with(10, 7).validate().is_err());
    }

    #[test]
    fn between_dates_must_be_real_days() {
        assert!(
            between(Axis::Month, "2024-02-29", "2026-01-31")
                .validate()
                .is_ok()
        );
        for bad in ["2026-02-30", "2025-02-29", "2026-13-01", "2026-1-01", "x"] {
            assert!(
                between(Axis::Month, bad, "2026-12-31").validate().is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_backwards_period_is_refused() {
        assert!(
            between(Axis::Day, "2026-03-02", "2026-03-01")
                .validate()
                .is_err()
        );
        assert!(
            between(Axis::Day, "2026-03-01", "2026-03-01")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_thirty_year_daily_plot_is_refused_with_the_limit_in_the_message() {
        let err = between(Axis::Day, "1996-01-01", "2025-12-31")
            .validate()
            .unwrap_err();
        assert!(err.contains("366"), "{err}");
    }

    #[test]
    fn buckets_stop_at_the_limit_for_each_axis() {
        // 2024 is a leap year: 366 days.
        assert!(
            between(Axis::Day, "2024-01-01", "2024-12-31")
                .validate()
                .is_ok()
        );
        assert!(
            between(Axis::Day, "2024-01-01", "2025-01-01")
                .validate()
                .is_err()
        );
        // Monday 2021-01-04 + 260 weeks - 1 day: exactly 260 weeks; one more day starts the 261st.
        assert!(
            between(Axis::Week, "2021-01-04", "2025-12-28")
                .validate()
                .is_ok()
        );
        let err = between(Axis::Week, "2021-01-04", "2025-12-29")
            .validate()
            .unwrap_err();
        assert!(err.contains("260"), "{err}");
        assert!(
            between(Axis::Month, "2006-01-15", "2025-12-01")
                .validate()
                .is_ok()
        );
        let err = between(Axis::Month, "2005-12-31", "2025-12-01")
            .validate()
            .unwrap_err();
        assert!(err.contains("240"), "{err}");
        // Coarser axes have no limit.
        assert!(
            between(Axis::Year, "1900-01-01", "2025-12-31")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn split_may_not_equal_the_axis() {
        let with = |by, split| PlotSpec {
            by,
            split: Some(split),
            ..spec()
        };
        assert!(with(Axis::Weekday, Split::Weekday).validate().is_err());
        assert!(with(Axis::Year, Split::Year).validate().is_err());
        assert!(with(Axis::Category, Split::Category).validate().is_err());
        assert!(
            with(Axis::FixedOrVariable, Split::FixedOrVariable)
                .validate()
                .is_err()
        );
        assert!(with(Axis::Category, Split::Year).validate().is_ok());
        assert!(with(Axis::Month, Split::Category).validate().is_ok());
    }

    #[test]
    fn charts_offers_only_what_validates() {
        let samples = [
            spec(),
            PlotSpec {
                by: Axis::Category,
                ..spec()
            },
            PlotSpec {
                by: Axis::Weekday,
                split: Some(Split::Year),
                ..spec()
            },
            PlotSpec {
                measures: vec![m(Measure::Spent), m(Measure::Count)],
                by: Axis::Merchant,
                ..spec()
            },
            PlotSpec {
                measures: vec![m(Measure::Spent), m(Measure::Income), m(Measure::Net)],
                ..spec()
            },
            PlotSpec {
                by: Axis::Group,
                split: Some(Split::Account),
                ..spec()
            },
        ];
        for s in &samples {
            let offered = s.charts();
            assert_eq!(offered.last(), Some(&Chart::Table), "{s:?}");
            for chart in CHARTS {
                let ok = PlotSpec { chart, ..s.clone() }.validate().is_ok();
                assert_eq!(offered.contains(&chart), ok, "{chart:?} for {s:?}");
            }
        }
        use Chart::*;
        assert_eq!(samples[1].charts(), [Bars, Donut, Ranked, Table]);
        assert_eq!(
            samples[2].charts(),
            [Bars, StackedBars, Line, Area, Heatmap, Table]
        );
        assert_eq!(samples[3].charts(), [Scatter, Table]);
        let year_on_year = PlotSpec {
            by: Axis::MonthOfYear,
            split: Some(Split::Year),
            ..spec()
        };
        assert_eq!(
            year_on_year.charts(),
            [Bars, StackedBars, Line, Area, Heatmap, Table]
        );
    }

    #[test]
    fn counts_mixed_with_money_only_scatter_or_table() {
        let mixed = PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Count)],
            ..spec()
        };
        assert_eq!(mixed.charts(), [Chart::Scatter, Chart::Table]);
        let counts = PlotSpec {
            measures: vec![m(Measure::Count), m(Measure::Count)],
            ..spec()
        };
        assert!(counts.validate().is_ok());
    }

    #[test]
    fn a_series_unit_defaults_to_money() {
        let s: PlotSeries = serde_json::from_str("{}").unwrap();
        assert_eq!(s.unit, Unit::Money);
        let c: PlotSeries = serde_json::from_str(r#"{"unit":"count"}"#).unwrap();
        assert_eq!(c.unit, Unit::Count);
    }

    #[test]
    fn change_with_a_running_total_is_refused() {
        let s = PlotSpec {
            value: Value::Change,
            cumulative: true,
            ..spec()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn scatter_refuses_compare_running_total_and_average_line() {
        let ok = PlotSpec {
            measures: vec![m(Measure::Spent), m(Measure::Income)],
            chart: Chart::Scatter,
            ..spec()
        };
        assert!(ok.validate().is_ok());
        for bad in [
            PlotSpec {
                compare: Compare::Previous,
                ..ok.clone()
            },
            PlotSpec {
                cumulative: true,
                ..ok.clone()
            },
            PlotSpec {
                average_line: true,
                ..ok
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn donut_refuses_change() {
        let s = PlotSpec {
            by: Axis::Category,
            value: Value::Change,
            chart: Chart::Donut,
            ..spec()
        };
        assert!(s.validate().is_err());
        assert!(
            PlotSpec {
                chart: Chart::Ranked,
                ..s
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn line_and_area_need_a_time_weekday_or_day_of_month_axis() {
        for chart in [Chart::Line, Chart::Area] {
            let with = |by| PlotSpec {
                by,
                chart,
                ..spec()
            };
            for by in [
                Axis::Day,
                Axis::Quarter,
                Axis::Weekday,
                Axis::DayOfMonth,
                Axis::MonthOfYear,
            ] {
                assert!(with(by).validate().is_ok(), "{chart:?} {by:?}");
            }
            for by in [
                Axis::Category,
                Axis::Group,
                Axis::Merchant,
                Axis::Account,
                Axis::FixedOrVariable,
            ] {
                assert!(with(by).validate().is_err(), "{chart:?} {by:?}");
            }
        }
    }

    /// The ten ready-made plots (plan Task 8).
    fn ready_made() -> Vec<(&'static str, PlotSpec)> {
        let one = |measure| vec![m(measure)];
        vec![
            (
                "Spending calendar",
                PlotSpec {
                    by: Axis::Week,
                    split: Some(Split::Weekday),
                    period: Period::Last3Months,
                    chart: Chart::Heatmap,
                    ..spec()
                },
            ),
            (
                "Weekday pattern",
                PlotSpec {
                    measures: one(Measure::AveragePerDay),
                    by: Axis::Weekday,
                    ..spec()
                },
            ),
            (
                "This month against usual",
                PlotSpec {
                    by: Axis::Day,
                    period: Period::ThisMonth,
                    chart: Chart::Line,
                    cumulative: true,
                    compare: Compare::AverageOf3,
                    ..spec()
                },
            ),
            (
                "Top merchants",
                PlotSpec {
                    by: Axis::Merchant,
                    period: Period::Last3Months,
                    chart: Chart::Ranked,
                    ..spec()
                },
            ),
            (
                "Where it went",
                PlotSpec {
                    split: Some(Split::Group),
                    period: Period::Last12Months,
                    chart: Chart::StackedBars,
                    ..spec()
                },
            ),
            (
                "Income and spending",
                PlotSpec {
                    measures: vec![
                        m(Measure::Income),
                        m(Measure::Spent),
                        MeasureSpec {
                            measure: Measure::Net,
                            draw: Draw::Line,
                        },
                    ],
                    period: Period::Last12Months,
                    ..spec()
                },
            ),
            (
                "Year on year",
                PlotSpec {
                    by: Axis::MonthOfYear,
                    split: Some(Split::Year),
                    period: Period::AllTime,
                    chart: Chart::Line,
                    ..spec()
                },
            ),
            (
                "Small payments",
                PlotSpec {
                    measures: one(Measure::Count),
                    by: Axis::Merchant,
                    filters: Filters {
                        max: Some(Decimal::from(10)),
                        ..Filters::default()
                    },
                    chart: Chart::Ranked,
                    ..spec()
                },
            ),
            (
                "Fixed against variable",
                PlotSpec {
                    split: Some(Split::FixedOrVariable),
                    period: Period::Last12Months,
                    chart: Chart::StackedBars,
                    ..spec()
                },
            ),
            (
                "Biggest changes",
                PlotSpec {
                    by: Axis::Category,
                    period: Period::Last3Months,
                    value: Value::Change,
                    chart: Chart::Ranked,
                    ..spec()
                },
            ),
        ]
    }

    #[test]
    fn every_ready_made_plot_validates_with_its_chart() {
        let all = ready_made();
        assert_eq!(all.len(), 10);
        for (title, s) in all {
            assert_eq!(s.validate(), Ok(()), "{title}");
            assert!(s.charts().contains(&s.chart), "{title}");
        }
    }

    #[test]
    fn a_spec_round_trips_through_json() {
        let s = PlotSpec {
            split: Some(Split::FixedOrVariable),
            period: Period::Between {
                from: "2026-01-01".into(),
                to: "2026-06-30".into(),
            },
            filters: Filters {
                min: Some(Decimal::new(1050, 2)),
                ..Filters::default()
            },
            ..spec()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""split":"fixed_or_variable""#), "{json}");
        assert_eq!(serde_json::from_str::<PlotSpec>(&json).unwrap(), s);
    }
}
