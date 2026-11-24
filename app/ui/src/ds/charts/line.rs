use leptos::prelude::*;

use super::{
    Fmt, Legend, Line, Series, Tooltip, ValueTable, YTicks, cn, nice_axis, pct, series_table,
};

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
    /// A labelled vertical line at one category, e.g. today.
    #[prop(default = None)]
    marker: Option<(usize, String)>,
    /// The last category is unfinished (this month so far): its segment is dashed.
    #[prop(optional)]
    partial_last: bool,
    /// Also a visually hidden table of the values, for screen readers.
    #[prop(optional)]
    table: bool,
) -> impl IntoView {
    let last = categories.len().checked_sub(1).filter(|_| partial_last);
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
            let line = s.line;
            let flush = |seg: &mut Vec<(f64, f64)>, d: &mut String, fill: &mut String| {
                if seg.is_empty() {
                    return;
                }
                let path = join(seg, line);
                if area {
                    let first = seg[0].0;
                    let last = seg[seg.len() - 1].0;
                    fill.push_str(&format!("{path} L{last:.3},{:.3} L{first:.3},{:.3} Z ", y(0.0), y(0.0)));
                }
                d.push_str(&path);
                d.push(' ');
                seg.clear();
            };
            for (i, v) in s.values.iter().enumerate().take(last.unwrap_or(usize::MAX)) {
                match v {
                    Some(v) => segment.push((x(i), y(*v))),
                    None => flush(&mut segment, &mut d, &mut fill),
                }
            }
            flush(&mut segment, &mut d, &mut fill);
            let point = |i: usize| s.values.get(i).copied().flatten().map(|v| (x(i), y(v)));
            let tail = last.and_then(|l| Some(join(&[point(l.checked_sub(1)?)?, point(l)?], line)));
            let color = s.color.clone();
            let tail = tail.map(|path| {
                let fill = format!("{path} L{:.3},{:.3} L{:.3},{:.3} Z", x(n - 1), y(0.0), x(n - 2), y(0.0));
                view! {
                    {area.then(|| view! { <path d=fill fill=color.clone() fill-opacity="0.07" stroke="none"></path> })}
                    <path
                        d=path
                        fill="none"
                        stroke=color.clone()
                        stroke-width=if line == Line::Dashed { "1.5" } else { "2" }
                        stroke-dasharray="5 4"
                        vector-effect="non-scaling-stroke"
                    ></path>
                }
            });
            view! {
                {area.then(|| view! { <path d=fill.clone() fill=color.clone() fill-opacity="0.15" stroke="none"></path> })}
                {tail}
                <path
                    d=d
                    fill="none"
                    stroke=color
                    stroke-width=if line == Line::Dashed { "1.5" } else { "2" }
                    stroke-dasharray=(line == Line::Dashed).then_some("5 4")
                    vector-effect="non-scaling-stroke"
                ></path>
            }
        })
        .collect_view();
    let marker = marker.filter(|(i, _)| *i < n).map(|(i, text)| {
        let flip = x(i) > 85.0;
        view! {
            <div class="absolute inset-y-0 border-l border-dashed border-fg-muted/60" style:left=pct(x(i))>
                <span class=if flip {
                    "absolute -top-1.5 right-1.5 whitespace-nowrap text-eyebrow leading-none text-fg-muted"
                } else {
                    "absolute -top-1.5 left-1.5 whitespace-nowrap text-eyebrow leading-none text-fg-muted"
                }>{text}</span>
            </div>
        }
    });
    let tick_views = ticks.iter().map(|t| (y(*t), format(*t))).collect();
    let legend = (series.len() > 1).then(|| {
        let dashed = series
            .iter()
            .filter(|s| s.line == Line::Dashed)
            .map(|s| s.label.clone())
            .collect::<Vec<_>>();
        view! { <Legend series=series.iter().map(|s| (s.label.clone(), s.color.clone())).collect() dashed=dashed /> }
    });
    let titles = titles.unwrap_or_else(|| categories.clone());
    let table = table.then(|| {
        let (head, rows) = series_table(&titles, &series, &format);
        view! { <ValueTable caption=label.clone() head=head rows=rows /> }
    });
    let hover_series = series.clone();
    let tip_format = format.clone();
    view! {
        <div role="figure" aria-label=label class=cn(["w-full", class.as_str()])>
            <div class="relative" style=format!("height:{plot_h}px;margin-left:{y_axis_width}px;margin-right:8px;margin-top:8px")>
                <YTicks ticks=tick_views width=y_axis_width />
                <svg class="absolute inset-0 h-full w-full overflow-visible" viewBox="0 0 100 100" preserveAspectRatio="none">
                    {paths}
                </svg>
                {marker}
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
            <div class="relative h-6 text-eyebrow text-chart-axis" style=format!("margin-left:{y_axis_width}px;margin-right:8px")>
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
            {table}
        </div>
    }
}

/// The path through one unbroken run of points in a series' line style.
pub(super) fn join(points: &[(f64, f64)], line: Line) -> String {
    match line {
        Line::Smooth => smooth(points),
        Line::Straight | Line::Dashed => {
            let mut d = format!("M{:.3},{:.3}", points[0].0, points[0].1);
            for p in &points[1..] {
                d.push_str(&format!(" L{:.3},{:.3}", p.0, p.1));
            }
            d
        }
        Line::Step => {
            let mut d = format!("M{:.3},{:.3}", points[0].0, points[0].1);
            for p in &points[1..] {
                d.push_str(&format!(" H{:.3} V{:.3}", p.0, p.1));
            }
            d
        }
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
