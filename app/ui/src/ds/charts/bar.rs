use leptos::prelude::*;

use super::line::join;
use super::{
    Fmt, Legend, Line, Series, Tooltip, ValueTable, YTicks, cn, nice_axis, pct, series_table,
};

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
    /// The last category is unfinished (this month so far): drawn lighter.
    #[prop(optional)]
    partial_last: bool,
    /// Also a visually hidden table of the values, for screen readers.
    #[prop(optional)]
    table: bool,
    /// Series drawn as lines over the bars, on the same axis, through the
    /// middle of each category.
    #[prop(optional)]
    lines: Vec<Series>,
    /// Show every n-th category label (long daily axes; see `label_step`).
    #[prop(default = 1)]
    label_every: usize,
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
    let line_values = || {
        lines
            .iter()
            .flat_map(|s| s.values.iter().flatten().copied())
    };
    let lo = totals
        .iter()
        .map(|t| t.0)
        .chain(line_values())
        .fold(0.0, f64::min);
    let hi = totals
        .iter()
        .map(|t| t.1)
        .chain(line_values())
        .fold(0.0, f64::max);
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
                    let partial = if partial_last && i + 1 == categories.len() {
                        let edge = if v >= 0.0 { "top" } else { "bottom" };
                        let dash = if cap { format!(";border-{edge}:2px dashed {}", s.color) } else { String::new() };
                        format!(";background:color-mix(in oklab,{} 45%,transparent){dash}", s.color)
                    } else {
                        String::new()
                    };
                    Some(
                        view! {
                            <div
                                class="absolute"
                                style=format!(
                                    "left:{};width:{};top:{};height:{};background:{};border-radius:{radius};max-width:32px;margin-left:max(0px,calc(({} - 32px)/2)){partial}",
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
    let has_lines = !lines.is_empty();
    // Like LineChart: an unfinished last category's segment is dashed.
    let last = categories.len().checked_sub(1).filter(|_| partial_last);
    let overlay = lines
        .iter()
        .map(|s| {
            let at = |i: usize| s.values.get(i).copied().flatten().map(|v| (band * (i as f64 + 0.5), y(v)));
            let mut d = String::new();
            let mut run = Vec::new();
            for i in 0..last.unwrap_or(categories.len()) {
                match at(i) {
                    Some(p) => run.push(p),
                    None if !run.is_empty() => {
                        d.push_str(&join(&run, s.line));
                        d.push(' ');
                        run.clear();
                    }
                    None => {}
                }
            }
            if !run.is_empty() {
                d.push_str(&join(&run, s.line));
            }
            let tail = last.and_then(|l| Some(join(&[at(l.checked_sub(1)?)?, at(l)?], s.line)));
            let width = if s.line == Line::Dashed { "1.5" } else { "2" };
            let color = s.color.clone();
            view! {
                <path
                    d=d
                    fill="none"
                    stroke=s.color.clone()
                    stroke-width=width
                    stroke-dasharray=(s.line == Line::Dashed).then_some("5 4")
                    vector-effect="non-scaling-stroke"
                ></path>
                {tail.map(|t| {
                    view! {
                        <path d=t fill="none" stroke=color stroke-width=width stroke-dasharray="5 4" vector-effect="non-scaling-stroke"></path>
                    }
                })}
            }
        })
        .collect_view();
    let tick_views = ticks.iter().map(|t| (y(*t), format(*t))).collect();
    let dashed = lines
        .iter()
        .filter(|s| s.line == Line::Dashed)
        .map(|s| s.label.clone())
        .collect::<Vec<_>>();
    let series: Vec<Series> = series.into_iter().chain(lines).collect();
    let legend = (series.len() > 1).then(|| {
        view! { <Legend series=series.iter().map(|s| (s.label.clone(), s.color.clone())).collect() dashed=dashed /> }
    });
    let titles = titles.unwrap_or_else(|| categories.clone());
    let table = table.then(|| {
        let (head, rows) = series_table(&titles, &series, &format);
        view! { <ValueTable caption=label.clone() head=head rows=rows /> }
    });
    let tooltip_series = series.clone();
    let tooltip_format = format.clone();
    view! {
        <div role="figure" aria-label=label class=cn(["w-full", class.as_str()])>
            <div class="relative" style=format!("height:{plot_h}px;margin-left:{y_axis_width}px;margin-right:8px;margin-top:8px")>
                <YTicks ticks=tick_views width=y_axis_width />
                {bars}
                {has_lines.then(|| view! {
                    <svg
                        class="pointer-events-none absolute inset-0 h-full w-full overflow-visible"
                        viewBox="0 0 100 100"
                        preserveAspectRatio="none"
                    >
                        {overlay}
                    </svg>
                })}
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
            {if label_every > 1 {
                // Thinned labels sit at their band's centre and may be wider than it.
                view! {
                    <div class="relative h-6 text-eyebrow text-chart-axis" style=format!("margin-left:{y_axis_width}px;margin-right:8px")>
                        {categories
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| i % label_every == 0)
                            .map(|(i, c)| {
                                view! {
                                    <span class="absolute top-2 -translate-x-1/2 whitespace-nowrap" style:left=pct(band * (i as f64 + 0.5))>
                                        {c.clone()}
                                    </span>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="flex text-eyebrow text-chart-axis" style=format!("margin-left:{y_axis_width}px;margin-right:8px")>
                        {categories
                            .iter()
                            .map(|c| view! { <span class="flex-1 truncate pt-2 text-center">{c.clone()}</span> })
                            .collect_view()}
                    </div>
                }
                    .into_any()
            }}
            {legend}
            {table}
        </div>
    }
}
