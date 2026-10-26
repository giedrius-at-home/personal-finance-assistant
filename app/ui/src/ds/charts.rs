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

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub key: String,
    pub label: String,
    pub color: String,
    /// One value per category; `None` leaves a gap.
    pub values: Vec<Option<f64>>,
}

impl Series {
    pub fn new(index: usize, key: &str, label: &str, values: Vec<Option<f64>>) -> Self {
        Self {
            key: key.to_owned(),
            label: label.to_owned(),
            color: series_color(index, None),
            values,
        }
    }

    pub fn colored(mut self, named: &str) -> Self {
        self.color = series_color(0, Some(named));
        self
    }
}

/// A "nice" axis like Recharts' default: about five ticks on round steps.
fn nice_axis(min: f64, max: f64) -> (f64, f64, Vec<f64>) {
    let (lo, hi) = (min.min(0.0), max.max(0.0));
    if hi - lo <= f64::EPSILON {
        return (0.0, 1.0, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    }
    let raw = (hi - lo) / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let start = (lo / step).floor() * step;
    let end = (hi / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut t = start;
    while t <= end + step / 2.0 {
        ticks.push(if t.abs() < step / 1e6 { 0.0 } else { t });
        t += step;
    }
    (start, end, ticks)
}

fn pct(v: f64) -> String {
    format!("{:.4}%", v.clamp(-1000.0, 1000.0))
}

#[component]
fn Legend(series: Vec<(String, String)>) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-center justify-center gap-x-4 gap-y-1.5 pt-3 text-xs text-fg-muted">
            {series
                .into_iter()
                .map(|(label, color)| {
                    view! {
                        <div class="flex items-center gap-1.5">
                            <div aria-hidden="true" class="h-2 w-2 shrink-0 rounded-[2px]" style:background-color=color></div>
                            {label}
                        </div>
                    }
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

/// Grouped or stacked bars along a category axis (`BarChart`, layout "horizontal").
#[component]
pub fn BarChart(
    categories: Vec<String>,
    series: Vec<Series>,
    format: Fmt,
    #[prop(default = 240)] height: u32,
    #[prop(default = 56)] y_axis_width: u32,
    #[prop(optional)] stacked: bool,
    #[prop(optional, into)] label: String,
    #[prop(optional, into)] class: String,
    /// Tooltip title per category (defaults to the category label).
    #[prop(optional)]
    titles: Option<Vec<String>>,
) -> impl IntoView {
    let n = categories.len().max(1);
    let totals: Vec<(f64, f64)> = (0..categories.len())
        .map(|i| {
            if stacked {
                let (pos, neg) = series
                    .iter()
                    .filter_map(|s| s.values.get(i).copied().flatten())
                    .fold(
                        (0.0, 0.0),
                        |(p, q), v| if v >= 0.0 { (p + v, q) } else { (p, q + v) },
                    );
                (neg, pos)
            } else {
                series
                    .iter()
                    .filter_map(|s| s.values.get(i).copied().flatten())
                    .fold((0.0_f64, 0.0_f64), |(lo, hi), v| (lo.min(v), hi.max(v)))
            }
        })
        .collect();
    let lo = totals.iter().map(|t| t.0).fold(0.0, f64::min);
    let hi = totals.iter().map(|t| t.1).fold(0.0, f64::max);
    let (start, end, ticks) = nice_axis(lo, hi);
    let span = (end - start).max(f64::EPSILON);
    let y = move |v: f64| 100.0 - (v - start) / span * 100.0;
    let plot_h = height.saturating_sub(24);
    let hovered = RwSignal::new(None::<usize>);
    let band = 100.0 / n as f64;
    let groups = if stacked { 1 } else { series.len().max(1) };
    let bars: Vec<AnyView> = (0..categories.len())
        .flat_map(|i| {
            let mut stack_pos = 0.0;
            let mut stack_neg = 0.0;
            let last_index = series.len().saturating_sub(1);
            series
                .iter()
                .enumerate()
                .filter_map(|(k, s)| {
                    let v = s.values.get(i).copied().flatten()?;
                    let gap = band * 0.1;
                    let width = ((band - 2.0 * gap) / groups as f64).min(band);
                    let slot = if stacked { 0 } else { k };
                    let x = band * i as f64 + gap + width * slot as f64;
                    let (base, top) = if stacked {
                        if v >= 0.0 {
                            let b = stack_pos;
                            stack_pos += v;
                            (b, stack_pos)
                        } else {
                            let b = stack_neg;
                            stack_neg += v;
                            (b, stack_neg)
                        }
                    } else {
                        (0.0, v)
                    };
                    let (y1, y2) = (y(base.max(top)), y(base.min(top)));
                    let cap = !stacked || k == last_index;
                    let radius = if cap { "4px 4px 0 0" } else { "0" };
                    Some(
                        view! {
                            <div
                                class="absolute"
                                style=format!(
                                    "left:{};width:{};top:{};height:{};background:{};border-radius:{radius};max-width:32px;margin-left:max(0px,calc(({} - 32px)/2))",
                                    pct(x),
                                    pct(width),
                                    pct(y1),
                                    pct((y2 - y1).max(0.0)),
                                    s.color,
                                    pct(width),
                                )
                            ></div>
                        }
                        .into_any(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let tick_views = ticks
        .iter()
        .map(|t| {
            let top = pct(y(*t));
            let text = format(*t);
            view! {
                <div class="absolute inset-x-0 border-t border-chart-grid" style:top=top.clone()></div>
                <span
                    class="absolute -translate-y-1/2 pr-2 text-right text-[11px] text-chart-axis tabular-nums"
                    style=format!("top:{top};right:100%;width:{y_axis_width}px")
                >
                    {text}
                </span>
            }
        })
        .collect_view();
    let legend = (series.len() > 1).then(|| {
        view! { <Legend series=series.iter().map(|s| (s.label.clone(), s.color.clone())).collect() /> }
    });
    let titles = titles.unwrap_or_else(|| categories.clone());
    let tooltip_series = series.clone();
    let tooltip_format = format.clone();
    view! {
        <div role="figure" aria-label=label class=cn(["w-full", class.as_str()])>
            <div class="relative" style=format!("height:{plot_h}px;margin-left:{y_axis_width}px;margin-right:8px;margin-top:8px")>
                {tick_views}
                {bars}
                <div class="absolute inset-0 flex" on:mouseleave=move |_| hovered.set(None)>
                    {(0..categories.len())
                        .map(|i| {
                            view! {
                                <div
                                    class="h-full flex-1 transition-colors"
                                    class:bg-soft=move || hovered.get() == Some(i)
                                    style="mix-blend-mode:multiply;opacity:.6"
                                    on:mouseenter=move |_| hovered.set(Some(i))
                                ></div>
                            }
                        })
                        .collect_view()}
                </div>
                {move || {
                    hovered
                        .get()
                        .map(|i| {
                            let rows = tooltip_series
                                .iter()
                                .filter_map(|s| {
                                    s.values
                                        .get(i)
                                        .copied()
                                        .flatten()
                                        .map(|v| (s.label.clone(), tooltip_format(v), s.color.clone()))
                                })
                                .collect();
                            view! {
                                <Tooltip
                                    title=titles.get(i).cloned().unwrap_or_default()
                                    rows=rows
                                    at=band * (i as f64 + 0.5)
                                />
                            }
                        })
                }}
            </div>
            <div class="flex text-[11px] text-chart-axis" style=format!("margin-left:{y_axis_width}px;margin-right:8px")>
                {categories
                    .iter()
                    .map(|c| view! { <span class="flex-1 truncate pt-2 text-center">{c.clone()}</span> })
                    .collect_view()}
            </div>
            {legend}
        </div>
    }
}

/// Lines or filled areas over a category axis (`LineChart` / `AreaChart`).
#[component]
pub fn LineChart(
    categories: Vec<String>,
    series: Vec<Series>,
    format: Fmt,
    #[prop(default = 240)] height: u32,
    #[prop(default = 56)] y_axis_width: u32,
    #[prop(optional)] area: bool,
    #[prop(optional, into)] label: String,
    #[prop(optional, into)] class: String,
    #[prop(optional)] titles: Option<Vec<String>>,
    /// Show every n-th category label (long daily axes).
    #[prop(default = 1)]
    label_every: usize,
) -> impl IntoView {
    let n = categories.len().max(1);
    let all: Vec<f64> = series
        .iter()
        .flat_map(|s| s.values.iter().flatten().copied())
        .collect();
    let (start, end, ticks) = nice_axis(
        all.iter().copied().fold(0.0, f64::min),
        all.iter().copied().fold(0.0, f64::max),
    );
    let span = (end - start).max(f64::EPSILON);
    let y = move |v: f64| 100.0 - (v - start) / span * 100.0;
    // Points sit at band centres like Recharts' category axis with padding.
    let x = move |i: usize| {
        if n == 1 {
            50.0
        } else {
            i as f64 / (n - 1) as f64 * 100.0
        }
    };
    let plot_h = height.saturating_sub(24);
    let hovered = RwSignal::new(None::<usize>);
    let paths = series
        .iter()
        .map(|s| {
            let mut d = String::new();
            let mut fill = String::new();
            let mut segment: Vec<(f64, f64)> = Vec::new();
            let flush = |seg: &mut Vec<(f64, f64)>, d: &mut String, fill: &mut String| {
                if seg.is_empty() {
                    return;
                }
                d.push_str(&smooth(seg));
                if area {
                    let first = seg[0].0;
                    let last = seg[seg.len() - 1].0;
                    fill.push_str(&format!("{} L{last:.3},{:.3} L{first:.3},{:.3} Z ", smooth(seg), y(0.0), y(0.0)));
                }
                seg.clear();
            };
            for (i, v) in s.values.iter().enumerate() {
                match v {
                    Some(v) => segment.push((x(i), y(*v))),
                    None => flush(&mut segment, &mut d, &mut fill),
                }
            }
            flush(&mut segment, &mut d, &mut fill);
            let color = s.color.clone();
            view! {
                {area.then(|| view! { <path d=fill.clone() fill=color.clone() fill-opacity="0.15" stroke="none"></path> })}
                <path d=d fill="none" stroke=color stroke-width="2" vector-effect="non-scaling-stroke"></path>
            }
        })
        .collect_view();
    let tick_views = ticks
        .iter()
        .map(|t| {
            let top = pct(y(*t));
            let text = format(*t);
            view! {
                <div class="absolute inset-x-0 border-t border-chart-grid" style:top=top.clone()></div>
                <span
                    class="absolute -translate-y-1/2 pr-2 text-right text-[11px] text-chart-axis tabular-nums"
                    style=format!("top:{top};right:100%;width:{y_axis_width}px")
                >
                    {text}
                </span>
            }
        })
        .collect_view();
    let legend = (series.len() > 1).then(|| {
        view! { <Legend series=series.iter().map(|s| (s.label.clone(), s.color.clone())).collect() /> }
    });
    let titles = titles.unwrap_or_else(|| categories.clone());
    let hover_series = series.clone();
    let tip_format = format.clone();
    view! {
        <div role="figure" aria-label=label class=cn(["w-full", class.as_str()])>
            <div class="relative" style=format!("height:{plot_h}px;margin-left:{y_axis_width}px;margin-right:8px;margin-top:8px")>
                {tick_views}
                <svg class="absolute inset-0 h-full w-full overflow-visible" viewBox="0 0 100 100" preserveAspectRatio="none">
                    {paths}
                </svg>
                {move || {
                    hovered
                        .get()
                        .map(|i| {
                            let left = pct(x(i));
                            let dots = hover_series
                                .iter()
                                .filter_map(|s| {
                                    s.values.get(i).copied().flatten().map(|v| {
                                        view! {
                                            <div
                                                class="absolute size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card-strong"
                                                style=format!("left:{left};top:{};background:{}", pct(y(v)), s.color)
                                            ></div>
                                        }
                                    })
                                })
                                .collect_view();
                            let rows = hover_series
                                .iter()
                                .filter_map(|s| {
                                    s.values
                                        .get(i)
                                        .copied()
                                        .flatten()
                                        .map(|v| (s.label.clone(), tip_format(v), s.color.clone()))
                                })
                                .collect();
                            view! {
                                <div class="absolute inset-y-0 border-l border-line" style:left=left.clone()></div>
                                {dots}
                                <Tooltip title=titles.get(i).cloned().unwrap_or_default() rows=rows at=x(i) />
                            }
                        })
                }}
                <div class="absolute inset-0 flex" on:mouseleave=move |_| hovered.set(None)>
                    {(0..categories.len())
                        .map(|i| view! { <div class="h-full flex-1" on:mouseenter=move |_| hovered.set(Some(i))></div> })
                        .collect_view()}
                </div>
            </div>
            <div class="relative h-6 text-[11px] text-chart-axis" style=format!("margin-left:{y_axis_width}px;margin-right:8px")>
                {categories
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i % label_every.max(1) == 0)
                    .map(|(i, c)| {
                        view! {
                            <span class="absolute top-2 -translate-x-1/2 whitespace-nowrap" style:left=pct(x(i))>
                                {c.clone()}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
            {legend}
        </div>
    }
}

/// A monotone-ish smooth path through points (Catmull-Rom → Bézier).
fn smooth(points: &[(f64, f64)]) -> String {
    let mut d = format!("M{:.3},{:.3}", points[0].0, points[0].1);
    for i in 1..points.len() {
        let p0 = points[i.saturating_sub(2).min(i - 1)];
        let p0 = if i >= 2 { points[i - 2] } else { p0 };
        let p1 = points[i - 1];
        let p2 = points[i];
        let p3 = points.get(i + 1).copied().unwrap_or(p2);
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        // Keep the curve inside the two points' value range (monotone look).
        let (lo, hi) = (p1.1.min(p2.1), p1.1.max(p2.1));
        d.push_str(&format!(
            " C{:.3},{:.3} {:.3},{:.3} {:.3},{:.3}",
            c1.0,
            c1.1.clamp(lo, hi),
            c2.0,
            c2.1.clamp(lo, hi),
            p2.0,
            p2.1
        ));
    }
    d
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub color: String,
}

/// A donut with a centre label and a legend (`DonutChart`).
#[component]
pub fn Donut(
    data: Vec<Slice>,
    format: Fmt,
    #[prop(optional)] center: Option<Children>,
    #[prop(default = 240)] height: u32,
    #[prop(optional, into)] label: String,
) -> impl IntoView {
    let total: f64 = data
        .iter()
        .map(|s| s.value.max(0.0))
        .sum::<f64>()
        .max(f64::EPSILON);
    let hovered = RwSignal::new(None::<usize>);
    let (r_out, r_in) = (41.0, 29.0);
    let mut angle = -90.0_f64;
    let gap = 1.0;
    let arcs = data
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sweep = s.value.max(0.0) / total * 360.0;
            let (a0, a1) = (angle + gap / 2.0, angle + sweep - gap / 2.0);
            angle += sweep;
            let pt = |r: f64, a: f64| {
                let rad = a.to_radians();
                (50.0 + r * rad.cos(), 50.0 + r * rad.sin())
            };
            let large = if a1 - a0 > 180.0 { 1 } else { 0 };
            let (o0, o1, i1, i0) = (pt(r_out, a0), pt(r_out, a1), pt(r_in, a1), pt(r_in, a0));
            let d = format!(
                "M{:.3},{:.3} A{r_out},{r_out} 0 {large} 1 {:.3},{:.3} L{:.3},{:.3} A{r_in},{r_in} 0 {large} 0 {:.3},{:.3} Z",
                o0.0, o0.1, o1.0, o1.1, i1.0, i1.1, i0.0, i0.1
            );
            let color = s.color.clone();
            view! {
                <path
                    d=d
                    fill=color
                    stroke="var(--ds-card-strong)"
                    stroke-width="0.6"
                    on:mouseenter=move |_| hovered.set(Some(i))
                ></path>
            }
        })
        .collect_view();
    let tip_data = data.clone();
    view! {
        <div role="figure" aria-label=label class="flex w-full flex-col" style:height=format!("{height}px")>
            <div class="relative min-h-0 flex-1" on:mouseleave=move |_| hovered.set(None)>
                <svg class="h-full w-full" viewBox="0 0 100 100">{arcs}</svg>
                {center.map(|c| {
                    view! {
                        <div class="pointer-events-none absolute inset-0 flex items-center justify-center text-center text-sm font-semibold text-fg">
                            {c()}
                        </div>
                    }
                })}
                {move || {
                    hovered
                        .get()
                        .and_then(|i| tip_data.get(i))
                        .map(|s| {
                            view! {
                                <Tooltip
                                    title=String::new()
                                    rows=vec![(s.label.clone(), format(s.value), s.color.clone())]
                                    at=50.0
                                />
                            }
                        })
                }}
            </div>
            <Legend series=data.iter().map(|s| (s.label.clone(), s.color.clone())).collect() />
        </div>
    }
}

/// An axis-free trend line (`Sparkline`).
#[component]
pub fn Sparkline(
    data: Vec<f64>,
    #[prop(optional, into)] color: Option<String>,
    #[prop(default = 32)] height: u32,
    #[prop(into)] label: String,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let (lo, hi) = data
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    let span = (hi - lo).max(f64::EPSILON);
    let n = data.len().max(2);
    let points: Vec<(f64, f64)> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                2.0 + i as f64 / (n - 1) as f64 * 96.0,
                94.0 - (v - lo) / span * 88.0,
            )
        })
        .collect();
    let d = if points.is_empty() {
        String::new()
    } else {
        smooth(&points)
    };
    let color = color.unwrap_or_else(|| series_color(0, None));
    view! {
        <div role="img" aria-label=label class=cn(["w-full min-w-0", class.as_str()]) style:height=format!("{height}px")>
            <svg class="h-full w-full" viewBox="0 0 100 100" preserveAspectRatio="none">
                <path d=d fill="none" stroke=color stroke-width="1.5" vector-effect="non-scaling-stroke"></path>
            </svg>
        </div>
    }
}

fn clamp_percent(value: f64, max: f64) -> f64 {
    if max > 0.0 {
        (value / max * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// Spend against a budget: spent fill, optional forecast band and a limit marker.
#[component]
pub fn BudgetBar(
    spent: f64,
    limit: f64,
    #[prop(optional)] forecast: Option<f64>,
    #[prop(into)] label: String,
    format: Fmt,
    #[prop(optional, into)] class: String,
    /// Hide the label row (the site's `[&>div:first-child>span:first-child]:sr-only`).
    #[prop(optional)]
    hide_label: bool,
) -> impl IntoView {
    let scale = limit.max(spent).max(forecast.unwrap_or(0.0));
    let over = spent > limit;
    let spent_pct = clamp_percent(spent, scale);
    let forecast_pct = forecast.map_or(spent_pct, |f| clamp_percent(f, scale));
    let limit_pct = clamp_percent(limit, scale);
    let remaining = limit - spent;
    let text = format!(
        "{}{}",
        if over {
            format!("{} over limit", format(-remaining))
        } else {
            format!("{} left", format(remaining))
        },
        forecast
            .map(|f| format!(" · forecast {}", format(f)))
            .unwrap_or_default()
    );
    view! {
        <div class=cn(["grid gap-1.5", class.as_str()])>
            <div class="flex items-baseline justify-between gap-3 text-sm">
                <span class=if hide_label { "sr-only" } else { "font-medium text-fg" }>{label.clone()}</span>
                <span class="tabular-nums text-fg-muted">{format!("{} of {}", format(spent), format(limit))}</span>
            </div>
            <div
                role="progressbar"
                aria-label=label
                aria-valuenow=spent
                aria-valuemin="0"
                aria-valuemax=scale
                class="relative py-1"
            >
                <div class="relative h-2.5 overflow-hidden rounded-pill bg-soft">
                    {(forecast_pct > spent_pct).then(|| {
                        view! { <div class="absolute inset-y-0 left-0 rounded-pill bg-chart-1/30" style:width=pct(forecast_pct)></div> }
                    })}
                    <div
                        class=if over { "absolute inset-y-0 left-0 rounded-pill bg-negative" } else { "absolute inset-y-0 left-0 rounded-pill bg-chart-1" }
                        style:width=pct(spent_pct)
                    ></div>
                </div>
                <div aria-hidden="true" class="absolute inset-y-0 w-0.5 -translate-x-1/2 rounded-pill bg-chart-limit" style:left=pct(limit_pct)></div>
            </div>
            <p class=if over { "text-xs tabular-nums font-medium text-negative-fg" } else { "text-xs tabular-nums text-fg-muted" }>{text}</p>
        </div>
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarTone {
    #[default]
    Accent,
    Positive,
    Negative,
    Warning,
}

#[component]
pub fn ProgressBar(
    value: f64,
    max: f64,
    #[prop(into)] label: String,
    #[prop(optional)] tone: BarTone,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let percent = clamp_percent(value, max);
    let fill = match tone {
        BarTone::Accent => "h-full rounded-pill bg-accent transition-[width]",
        BarTone::Positive => "h-full rounded-pill bg-positive transition-[width]",
        BarTone::Negative => "h-full rounded-pill bg-negative transition-[width]",
        BarTone::Warning => "h-full rounded-pill bg-warning transition-[width]",
    };
    view! {
        <div class=cn(["grid gap-1.5", class.as_str()])>
            <div class="flex items-baseline justify-between gap-3 text-sm">
                <span class="font-medium text-fg">{label.clone()}</span>
                <span class="tabular-nums text-fg-muted">{format!("{}%", percent.round())}</span>
            </div>
            <div role="progressbar" aria-label=label class="h-2 overflow-hidden rounded-pill bg-soft">
                <div class=fill style:width=pct(percent)></div>
            </div>
        </div>
    }
}

/// A shared, cheap-to-clone formatter.
pub fn fmt(f: impl Fn(f64) -> String + Send + Sync + 'static) -> Fmt {
    Arc::new(f)
}
