use leptos::prelude::*;

use super::{Fmt, Legend, Tooltip};

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
    #[prop(optional, into)] label: Signal<String>,
) -> impl IntoView {
    let total: f64 = data
        .iter()
        .map(|s| s.value.max(0.0))
        .sum::<f64>()
        .max(f64::EPSILON);
    let hovered = RwSignal::new(None::<usize>);
    let (r_out, r_in) = (41.0, 29.0);
    // Recharts' Pie starts at 3 o'clock and runs counter-clockwise.
    let mut angle = 0.0_f64;
    let gap = 1.0;
    let arcs = data
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sweep = s.value.max(0.0) / total * 360.0;
            let (a0, a1) = (angle - gap / 2.0, angle - sweep + gap / 2.0);
            angle -= sweep;
            let pt = |r: f64, a: f64| {
                let rad = a.to_radians();
                (50.0 + r * rad.cos(), 50.0 + r * rad.sin())
            };
            let large = if a0 - a1 > 180.0 { 1 } else { 0 };
            let (o0, o1, i1, i0) = (pt(r_out, a0), pt(r_out, a1), pt(r_in, a1), pt(r_in, a0));
            let d = format!(
                "M{:.3},{:.3} A{r_out},{r_out} 0 {large} 0 {:.3},{:.3} L{:.3},{:.3} A{r_in},{r_in} 0 {large} 1 {:.3},{:.3} Z",
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
        <div role="figure" aria-label=move || label.get() class="flex w-full flex-col" style:height=format!("{height}px")>
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
