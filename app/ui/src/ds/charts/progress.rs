use leptos::prelude::*;

use super::{Fmt, clamp_percent, cn, pct};

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
    // Each number is its own field: a label, then the bare amount.
    let rest = if over {
        ("Over limit", format(-remaining))
    } else {
        ("Left", format(remaining))
    };
    let forecast_text = forecast.map(|f| format(f));
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
                        class=if over { "absolute inset-y-0 left-0 rounded-pill bg-over" } else { "absolute inset-y-0 left-0 rounded-pill bg-chart-1" }
                        style:width=pct(spent_pct)
                    ></div>
                </div>
                <div aria-hidden="true" class="absolute inset-y-0 w-0.5 -translate-x-1/2 rounded-pill bg-chart-limit" style:left=pct(limit_pct)></div>
            </div>
            <p class="flex items-baseline justify-between gap-3 text-xs tabular-nums text-fg-muted">
                <span>{rest.0}" "<span class=if over { "font-medium text-over-fg" } else { "font-medium text-fg" }>{rest.1}</span></span>
                {forecast_text.map(|f| view! { <span>"Forecast "<span class="font-medium text-fg">{f}</span></span> })}
            </p>
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
        BarTone::Negative => "h-full rounded-pill bg-over transition-[width]",
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
