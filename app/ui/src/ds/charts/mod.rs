//! Charts in the style of the site's Recharts wrappers (`components/ui/chart`):
//! horizontal grid lines, 11px axis ticks, rounded bars, a hover tooltip and a
//! legend for several series. Layout is HTML + percent-based SVG, so charts
//! stretch with the window without distorting text.

use std::sync::Arc;

use leptos::prelude::*;

use super::cn;

/// Formats an axis tick or tooltip value.
pub type Fmt = Arc<dyn Fn(f64) -> String + Send + Sync>;

/// `seriesColor`: the palette by index, or a named colour (fixed/free/limit).
pub fn series_color(index: usize, named: Option<&str>) -> String {
    match named {
        Some(name @ ("fixed" | "free" | "limit")) => format!("var(--ds-chart-{name})"),
        Some(other) => other.to_owned(),
        None => format!("var(--ds-chart-{})", index % 6 + 1),
    }
}

/// How a line chart joins a series' points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Line {
    /// A smoothed curve: trends where the shape matters more than each point.
    #[default]
    Smooth,
    /// Point to point, for values that really change linearly.
    Straight,
    /// Flat until the next point, then a jump: running totals.
    Step,
    /// Point to point and dashed: limits and other reference lines.
    Dashed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub key: String,
    pub label: String,
    pub color: String,
    /// One value per category; `None` leaves a gap.
    pub values: Vec<Option<f64>>,
    pub line: Line,
}

impl Series {
    pub fn new(index: usize, key: &str, label: &str, values: Vec<Option<f64>>) -> Self {
        Self {
            key: key.to_owned(),
            label: label.to_owned(),
            color: series_color(index, None),
            values,
            line: Line::Smooth,
        }
    }

    pub fn line(mut self, line: Line) -> Self {
        self.line = line;
        self
    }
}

fn digit_count(value: f64) -> i32 {
    if value == 0.0 {
        1
    } else {
        value.abs().log10().floor() as i32 + 1
    }
}

/// Recharts' `getAdaptiveStep`: the rough step rounded up to 0.05 (0.1 for
/// one-digit steps) of its order of magnitude, plus a correction.
fn adaptive_step(rough: f64, correction: u32) -> f64 {
    if rough <= 0.0 {
        return 0.0;
    }
    let digits = digit_count(rough);
    let magnitude = 10f64.powi(digits);
    let scale = if digits == 1 { 0.1 } else { 0.05 };
    (((rough / magnitude) / scale - 1e-12).ceil() + f64::from(correction)) * scale * magnitude
}

/// Recharts' default value axis (`getNiceTickValues`, five ticks, domain
/// `[0, auto]`), so axes read exactly as on the site.
fn nice_axis(min: f64, max: f64) -> (f64, f64, Vec<f64>) {
    const TICKS: i32 = 5;
    let (lo, hi) = (min.min(0.0), max.max(0.0));
    if hi - lo <= f64::EPSILON {
        return (0.0, 4.0, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }
    let mut correction = 0;
    loop {
        let step = adaptive_step((hi - lo) / f64::from(TICKS - 1), correction);
        let middle = if lo <= 0.0 && hi >= 0.0 {
            0.0
        } else {
            let m = (lo + hi) / 2.0;
            m - m % step
        };
        let mut below = ((middle - lo) / step - 1e-9).ceil() as i32;
        let mut up = ((hi - middle) / step - 1e-9).ceil() as i32;
        let count = below + up + 1;
        if count > TICKS {
            correction += 1;
            continue;
        }
        if count < TICKS {
            if hi > 0.0 {
                up += TICKS - count;
            } else {
                below += TICKS - count;
            }
        }
        let start = middle - f64::from(below) * step;
        let end = middle + f64::from(up) * step;
        let ticks: Vec<f64> = (0..TICKS)
            .map(|i| {
                let t = start + f64::from(i) * step;
                // Trim float noise like 0.30000000000000004.
                (t * 1e9).round() / 1e9
            })
            .collect();
        return (start, end, ticks);
    }
}

fn pct(v: f64) -> String {
    format!("{:.4}%", v.clamp(-1000.0, 1000.0))
}

mod bar;
mod donut;
mod heatmap;
mod line;
mod progress;
mod ranked;
mod scatter;
mod table;

pub use bar::BarChart;
pub use donut::{Donut, Slice};
pub use heatmap::Heatmap;
pub use line::LineChart;
pub use progress::{BarTone, BudgetBar, ProgressBar};
pub use ranked::Ranked;
pub use scatter::Scatter;
pub use table::DataTable;

/// The value axis: grid lines with their labels, each at `top` percent.
#[component]
fn YTicks(ticks: Vec<(f64, String)>, width: u32) -> impl IntoView {
    ticks
        .into_iter()
        .map(|(top, text)| {
            let top = pct(top);
            view! {
                <div class="absolute inset-x-0 border-t border-chart-grid" style:top=top.clone()></div>
                <span
                    class="absolute -translate-y-1/2 pr-2 text-right text-eyebrow text-chart-axis tabular-nums"
                    style=format!("top:{top};right:100%;width:{width}px")
                >
                    {text}
                </span>
            }
        })
        .collect_view()
}

#[component]
fn Legend(
    series: Vec<(String, String)>,
    /// Labels drawn as a dashed line instead of a swatch.
    #[prop(optional)]
    dashed: Vec<String>,
) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-center justify-center gap-x-4 gap-y-1.5 pt-3 text-xs text-fg-muted">
            {series
                .into_iter()
                .map(|(label, color)| {
                    let swatch = if dashed.contains(&label) {
                        view! {
                            <div
                                aria-hidden="true"
                                class="h-0 w-3.5 shrink-0 border-t-2 border-dashed"
                                style:border-color=color
                            ></div>
                        }
                        .into_any()
                    } else {
                        view! { <div aria-hidden="true" class="h-2 w-2 shrink-0 rounded-[2px]" style:background-color=color></div> }
                            .into_any()
                    };
                    view! { <div class="flex items-center gap-1.5">{swatch}{label}</div> }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn Tooltip(
    title: String,
    rows: Vec<(String, String, String)>,
    /// 0–100, horizontal centre of the hovered category.
    at: f64,
) -> impl IntoView {
    let flip = at > 60.0;
    let style = if flip {
        format!("right: calc({} + 12px)", pct(100.0 - at))
    } else {
        format!("left: calc({} + 12px)", pct(at))
    };
    view! {
        <div
            class="pointer-events-none absolute top-2 z-20 grid min-w-32 gap-1.5 rounded-lg border border-border/50 bg-card-strong px-2.5 py-1.5 text-xs text-fg shadow-pop"
            style=style
        >
            <div class="font-medium">{title}</div>
            {rows
                .into_iter()
                .map(|(label, value, color)| {
                    view! {
                        <div class="flex items-center gap-2">
                            <div class="h-2.5 w-2.5 shrink-0 rounded-[2px]" style:background-color=color></div>
                            <span class="flex-1 text-fg-muted">{label}</span>
                            <span class="font-mono font-medium tabular-nums text-fg">{value}</span>
                        </div>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// The chart's values as a visually hidden table, for screen readers: each
/// row's first cell names it; nothing when there are no rows.
#[component]
fn ValueTable(
    #[prop(into)] caption: Signal<String>,
    head: Vec<String>,
    rows: Vec<Vec<String>>,
) -> impl IntoView {
    (!rows.is_empty()).then(|| {
        view! {
            <table class="sr-only">
                <caption>{move || caption.get()}</caption>
                <thead>
                    <tr>{head.into_iter().map(|h| view! { <th scope="col">{h}</th> }).collect_view()}</tr>
                </thead>
                <tbody>
                    {rows
                        .into_iter()
                        .map(|row| {
                            let mut cells = row.into_iter();
                            let name = cells.next().unwrap_or_default();
                            view! {
                                <tr>
                                    <th scope="row">{name}</th>
                                    {cells.map(|c| view! { <td>{c}</td> }).collect_view()}
                                </tr>
                            }
                        })
                        .collect_view()}
                </tbody>
            </table>
        }
    })
}

/// A bar or line chart's values for `ValueTable`: one row per category,
/// one column per series.
fn series_table(
    titles: &[String],
    series: &[Series],
    format: &Fmt,
) -> (Vec<String>, Vec<Vec<String>>) {
    let head = std::iter::once("Name".to_owned())
        .chain(series.iter().map(|s| s.label.clone()))
        .collect();
    let rows = titles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            std::iter::once(t.clone())
                .chain(series.iter().map(|s| {
                    s.values
                        .get(i)
                        .copied()
                        .flatten()
                        .map_or_else(|| "No data".to_owned(), |v| format(v))
                }))
                .collect()
        })
        .collect();
    (head, rows)
}

fn clamp_percent(value: f64, max: f64) -> f64 {
    if max > 0.0 {
        (value / max * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// How often to label a category axis: day-style labels ("23 Nov") need
/// more room than month-style ones ("Nov"). For `label_every`.
pub fn label_step(categories: &[String]) -> usize {
    let long = categories.iter().any(|c| c.chars().count() > 4);
    categories.len().div_ceil(if long { 6 } else { 12 }).max(1)
}

/// A shared, cheap-to-clone formatter.
pub fn fmt(f: impl Fn(f64) -> String + Send + Sync + 'static) -> Fmt {
    Arc::new(f)
}

#[cfg(test)]
mod tests {
    use super::{label_step, nice_axis};

    #[test]
    fn long_axes_label_every_few_categories() {
        let days: Vec<String> = (1..=24).map(|d| format!("{d} Nov")).collect();
        let months: Vec<String> = ["Jan", "Feb", "Mar"].map(String::from).to_vec();
        assert_eq!(label_step(&days), 4);
        assert_eq!(label_step(&days[..6]), 1);
        assert_eq!(label_step(&months), 1);
        assert_eq!(label_step(&[]), 1);
    }

    #[test]
    fn ticks_follow_recharts() {
        assert_eq!(
            nice_axis(0.0, 2300.0).2,
            vec![0.0, 600.0, 1200.0, 1800.0, 2400.0]
        );
        assert_eq!(
            nice_axis(0.0, 236.96).2,
            vec![0.0, 60.0, 120.0, 180.0, 240.0]
        );
        assert_eq!(nice_axis(0.0, 9.0).2, vec![0.0, 3.0, 6.0, 9.0, 12.0]);
        assert_eq!(nice_axis(-10.0, 30.0).2, vec![-10.0, 0.0, 10.0, 20.0, 30.0]);
    }
}
