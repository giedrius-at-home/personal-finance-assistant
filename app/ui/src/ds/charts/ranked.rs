use leptos::prelude::*;

use super::{Fmt, ValueTable, clamp_percent, pct};

/// A bar's length: percent of its track (the half track when two-sided),
/// by size, so a negative value is as long as its positive.
pub fn ranked_width(value: f64, max_abs: f64) -> f64 {
    if value.is_finite() && max_abs.is_finite() {
        clamp_percent(value.abs(), max_abs)
    } else {
        0.0
    }
}

/// One row per item, in the order given: label, bar, value. Two-sided bars
/// grow from a centre line, negative to the left and positive to the right.
#[component]
pub fn Ranked(
    items: Vec<(String, f64)>,
    format: Fmt,
    #[prop(optional)] two_sided: bool,
    #[prop(optional, into)] label: Signal<String>,
    /// The last item is "Other": drawn muted.
    #[prop(optional)]
    other_last: bool,
) -> impl IntoView {
    let caption = label;
    let max_abs = items
        .iter()
        .map(|(_, v)| v.abs())
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    let other = items.len().checked_sub(1).filter(|_| other_last);
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, (name, v))| {
            let width = ranked_width(*v, max_abs);
            let fill = match (*v < 0.0, Some(i) == other) {
                (true, _) => "bg-negative",
                (false, true) => "bg-chart-limit",
                (false, false) => "bg-chart-1",
            };
            let style = match (two_sided, *v < 0.0) {
                (false, _) => format!("left:0;width:{}", pct(width)),
                (true, true) => format!("right:50%;width:{}", pct(width / 2.0)),
                (true, false) => format!("left:50%;width:{}", pct(width / 2.0)),
            };
            view! {
                <div class="flex items-center gap-3 text-sm">
                    <span class="w-1/4 min-w-20 truncate font-medium text-fg" title=name.clone()>{name.clone()}</span>
                    <div aria-hidden="true" class="relative h-2 flex-1 rounded-pill bg-soft">
                        <div class=format!("absolute inset-y-0 rounded-pill {fill}") style=style></div>
                        {two_sided.then(|| view! { <div class="absolute -inset-y-1 left-1/2 w-px bg-chart-axis"></div> })}
                    </div>
                    <span class="w-20 shrink-0 text-right tabular-nums text-fg-muted">{format(*v)}</span>
                </div>
            }
        })
        .collect_view();
    let table_rows = items
        .iter()
        .map(|(name, v)| vec![name.clone(), format(*v)])
        .collect();
    view! {
        <div role="figure" aria-label=move || label.get() class="grid w-full gap-2.5">
            {rows}
            <ValueTable caption=caption head=vec!["Name".to_owned(), "Value".to_owned()] rows=table_rows />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::ranked_width;

    #[test]
    fn ranked_widths_are_percent_of_the_largest() {
        assert_eq!(ranked_width(80.0, 80.0), 100.0);
        assert_eq!(ranked_width(40.0, 80.0), 50.0);
        assert_eq!(ranked_width(-40.0, 80.0), 50.0);
        assert_eq!(ranked_width(40.0, 0.0), 0.0);
        assert_eq!(ranked_width(f64::NAN, 80.0), 0.0);
        assert_eq!(ranked_width(40.0, f64::INFINITY), 0.0);
    }
}
