use leptos::prelude::*;

use super::{Fmt, Tooltip, ValueTable};

/// 0..=4: 0 for no value (NaN) or zero and below, then the quartile of `max`
/// the value falls in (`max` itself is 4).
pub fn heat_level(value: f64, max: f64) -> u8 {
    if !(value > 0.0 && max > 0.0) {
        return 0;
    }
    (value / max * 4.0).ceil().clamp(1.0, 4.0) as u8
}

/// Cell fills by heat level: `--ds-chart-1` at rising opacity.
const LEVELS: [&str; 5] = [
    "bg-soft",
    "bg-chart-1/30",
    "bg-chart-1/50",
    "bg-chart-1/75",
    "bg-chart-1",
];

/// An unfinished row's cells, like `BarChart`'s unfinished bar: lighter, with a dashed edge.
const PARTIAL: &str = " opacity-45 outline-2 -outline-offset-2 outline-dashed outline-chart-1";

/// Values in a grid of squares, darker for more (`values[row][column]`;
/// `None` is a cell with no data, drawn as an empty outline).
#[component]
pub fn Heatmap(
    rows: Vec<String>,
    columns: Vec<String>,
    values: Vec<Vec<Option<f64>>>,
    format: Fmt,
    #[prop(optional, into)] label: Signal<String>,
    /// The last row is unfinished (this week so far): its cells are drawn lighter.
    #[prop(optional)]
    partial_last: bool,
) -> impl IntoView {
    let cell = |r: usize, c: usize| values.get(r).and_then(|row| row.get(c).copied().flatten());
    let n = columns.len();
    // Only drawn cells set the scale: extra rows or columns in `values` are ignored.
    let max = (0..rows.len())
        .flat_map(|r| (0..n).filter_map(move |c| cell(r, c)))
        .fold(0.0, f64::max);
    let caption = label;
    let hovered = RwSignal::new(None::<(usize, usize)>);
    let grid = format!("grid-template-columns:auto repeat({n},minmax(0,40px))");
    let cells = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells = (0..n)
                .map(|c| {
                    let value = cell(r, c);
                    let partial = partial_last && r + 1 == rows.len();
                    let class = match value {
                        Some(v) => format!(
                            "relative aspect-square rounded-[4px] {}{}",
                            LEVELS[heat_level(v, max) as usize],
                            if partial { PARTIAL } else { "" },
                        ),
                        None => "relative aspect-square rounded-[4px] border border-dashed border-line".to_owned(),
                    };
                    let title = format!("{row} · {}", columns[c]);
                    let text = value.map_or_else(|| "No data".to_owned(), |v| format(v));
                    let at = (c as f64 + 0.5) / n as f64 * 100.0;
                    view! {
                        <div class=class on:mouseenter=move |_| hovered.set(Some((r, c)))>
                            {move || {
                                (hovered.get() == Some((r, c)))
                                    .then(|| {
                                        view! {
                                            <Tooltip
                                                title=title.clone()
                                                rows=vec![(String::new(), text.clone(), "var(--ds-chart-1)".to_owned())]
                                                at=at
                                            />
                                        }
                                    })
                            }}
                        </div>
                    }
                })
                .collect_view();
            view! {
                <span class="truncate pr-2 text-right text-eyebrow leading-none text-chart-axis">{row.clone()}</span>
                {cells}
            }
        })
        .collect_view();
    let table_rows = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            std::iter::once(row.clone())
                .chain(
                    (0..n).map(|c| cell(r, c).map_or_else(|| "No data".to_owned(), |v| format(v))),
                )
                .collect()
        })
        .collect();
    let head = std::iter::once("Name".to_owned())
        .chain(columns.iter().cloned())
        .collect();
    view! {
        <div role="figure" aria-label=move || label.get() class="w-full overflow-x-auto pt-2">
            <div class="grid w-fit items-center gap-[3px]" style=grid on:mouseleave=move |_| hovered.set(None)>
                {cells}
                <span></span>
                {columns
                    .iter()
                    .map(|c| view! { <span class="truncate pt-1 text-center text-eyebrow text-chart-axis">{c.clone()}</span> })
                    .collect_view()}
            </div>
            <ValueTable caption=caption head=head rows=table_rows />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::heat_level;

    #[test]
    fn heat_levels_are_quartiles_of_max() {
        assert_eq!(heat_level(f64::NAN, 100.0), 0, "no value");
        assert_eq!(heat_level(0.0, 100.0), 0);
        assert_eq!(heat_level(-30.0, 100.0), 0, "negative is the lowest level");
        assert_eq!(heat_level(100.0, 100.0), 4);
        assert_eq!(heat_level(25.0, 100.0), 1);
        assert_eq!(heat_level(25.01, 100.0), 2);
        assert_eq!(heat_level(50.0, 100.0), 2);
        assert_eq!(heat_level(75.0, 100.0), 3);
        assert_eq!(heat_level(75.01, 100.0), 4);
        assert_eq!(heat_level(5.0, 0.0), 0, "empty grid");
    }
}
