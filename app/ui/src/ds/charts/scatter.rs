use leptos::prelude::*;

use super::{Fmt, Tooltip, ValueTable, YTicks, nice_axis, pct};

/// The x and y axes `(start, end, ticks)`: each from zero to past its
/// furthest point; an empty set gets the default 0..4 frame.
fn scatter_axes(points: &[(String, f64, f64)]) -> [(f64, f64, Vec<f64>); 2] {
    let range = |values: Vec<f64>| {
        let finite = || values.iter().copied().filter(|v| v.is_finite());
        let lo = finite().fold(0.0, f64::min);
        let hi = finite().fold(0.0, f64::max);
        nice_axis(lo, hi)
    };
    [
        range(points.iter().map(|p| p.1).collect()),
        range(points.iter().map(|p| p.2).collect()),
    ]
}

/// Named points on two value axes, in the `LineChart` frame.
#[component]
pub fn Scatter(
    points: Vec<(String, f64, f64)>,
    x_format: Fmt,
    y_format: Fmt,
    #[prop(optional, into)] x_label: String,
    #[prop(optional, into)] y_label: String,
    /// The last point is an unfinished bucket: drawn as an outline, not filled.
    #[prop(optional)]
    partial_last: bool,
) -> impl IntoView {
    const Y_AXIS_WIDTH: u32 = 56;
    const PLOT_H: u32 = 216;
    // A point without two finite values can't be placed.
    let points: Vec<_> = points
        .into_iter()
        .filter(|p| p.1.is_finite() && p.2.is_finite())
        .collect();
    let [(x0, x1, x_ticks), (y0, y1, y_ticks)] = scatter_axes(&points);
    let x = move |v: f64| (v - x0) / (x1 - x0).max(f64::EPSILON) * 100.0;
    let y = move |v: f64| 100.0 - (v - y0) / (y1 - y0).max(f64::EPSILON) * 100.0;
    let hovered = RwSignal::new(None::<usize>);
    let y_ticks = y_ticks.iter().map(|t| (y(*t), y_format(*t))).collect();
    let last = points.len().checked_sub(1).filter(|_| partial_last);
    let dots = points
        .iter()
        .enumerate()
        .map(|(i, (_, px, py))| {
            let class = if Some(i) == last {
                "absolute size-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-chart-1 bg-card-strong"
            } else {
                "absolute size-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card-strong bg-chart-1"
            };
            view! {
                <div
                    class=class
                    style=format!("left:{};top:{}", pct(x(*px)), pct(y(*py)))
                    on:mouseenter=move |_| hovered.set(Some(i))
                    on:mouseleave=move |_| hovered.set(None)
                ></div>
            }
        })
        .collect_view();
    let axis_name = |name: &str, fallback: &str| {
        if name.is_empty() {
            fallback.to_owned()
        } else {
            name.to_owned()
        }
    };
    let (x_name, y_name) = (axis_name(&x_label, "x"), axis_name(&y_label, "y"));
    let caption = format!("{y_name} against {x_name}");
    let table_rows = points
        .iter()
        .map(|(name, px, py)| vec![name.clone(), x_format(*px), y_format(*py)])
        .collect();
    let head = vec!["Name".to_owned(), x_name.clone(), y_name.clone()];
    let table_caption = caption.clone();
    let tip_points = points.clone();
    let (tip_x, tip_y) = (x_format.clone(), y_format.clone());
    view! {
        <div role="figure" aria-label=caption class="w-full">
            {(!y_label.is_empty())
                .then(|| view! { <div class="text-eyebrow text-chart-axis" style=format!("margin-left:{Y_AXIS_WIDTH}px")>{y_label}</div> })}
            <div class="relative" style=format!("height:{PLOT_H}px;margin-left:{Y_AXIS_WIDTH}px;margin-right:8px;margin-top:8px")>
                <YTicks ticks=y_ticks width=Y_AXIS_WIDTH />
                {dots}
                {move || {
                    hovered
                        .get()
                        .and_then(|i| tip_points.get(i))
                        .map(|(name, px, py)| {
                            let rows = vec![
                                (x_name.clone(), tip_x(*px), "var(--ds-chart-1)".to_owned()),
                                (y_name.clone(), tip_y(*py), "var(--ds-chart-1)".to_owned()),
                            ];
                            view! { <Tooltip title=name.clone() rows=rows at=x(*px) /> }
                        })
                }}
            </div>
            <div class="relative h-6 text-eyebrow text-chart-axis" style=format!("margin-left:{Y_AXIS_WIDTH}px;margin-right:8px")>
                {x_ticks
                    .iter()
                    .map(|t| {
                        view! {
                            <span class="absolute top-2 -translate-x-1/2 whitespace-nowrap tabular-nums" style:left=pct(x(*t))>
                                {x_format(*t)}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
            {(!x_label.is_empty())
                .then(|| {
                    view! { <div class="pt-1 text-center text-eyebrow text-chart-axis" style=format!("margin-left:{Y_AXIS_WIDTH}px")>{x_label}</div> }
                })}
            <ValueTable caption=table_caption head=head rows=table_rows />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::scatter_axes;

    #[test]
    fn scatter_axes_include_zero_and_every_point() {
        let points = vec![
            ("a".to_owned(), 12.0, 340.0),
            ("b".to_owned(), -3.0, 90.0),
            ("c".to_owned(), 30.0, 1200.0),
        ];
        let [(x0, x1, _), (y0, y1, _)] = scatter_axes(&points);
        assert!(x0 <= 0.0 && x1 >= 0.0 && y0 <= 0.0 && y1 >= 0.0);
        for (_, x, y) in &points {
            assert!((x0..=x1).contains(x) && (y0..=y1).contains(y));
        }
        let [(x0, x1, _), (y0, y1, _)] =
            scatter_axes(&[("nan".to_owned(), f64::NAN, f64::INFINITY)]);
        assert!(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite());
        let [(x0, x1, xt), (y0, y1, _)] = scatter_axes(&[]);
        assert!(
            x1 > x0 && y1 > y0 && xt.len() == 5,
            "empty set draws a frame"
        );
    }
}
