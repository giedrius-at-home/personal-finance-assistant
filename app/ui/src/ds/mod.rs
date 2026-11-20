//! The personal site's design system (`components/ui`), ported to Leptos with
//! the same Tailwind classes so screens look the same as the web finance UI.

pub mod charts;
mod cn;
pub mod icons;
pub mod money_plan;

pub use cn::cn;
pub use icons::{Icon, Lucide};

use finance_types::report::Currency;
use leptos::html;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::format;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    #[default]
    Primary,
    Secondary,
    Ghost,
    Accent,
    Danger,
    Link,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    Sm,
    #[default]
    Md,
    Icon,
    IconSm,
    Inline,
}

pub fn button_class(variant: Variant, size: Size) -> String {
    let variant = match variant {
        Variant::Primary => "bg-contrast text-fg-on-contrast hover:bg-contrast/90",
        Variant::Secondary => "border border-border bg-card-strong text-fg hover:bg-soft",
        Variant::Ghost => "text-fg-muted hover:bg-soft hover:text-fg",
        Variant::Accent => "bg-accent-strong text-fg-on-accent hover:bg-accent-strong/90",
        Variant::Danger => "border border-negative/40 text-negative-fg hover:bg-negative/10",
        Variant::Link => "underline underline-offset-2 hover:no-underline",
        Variant::Plain => "",
    };
    let size = match size {
        Size::Sm => "h-8 px-3 font-sans text-xs font-medium",
        Size::Md => "h-10 px-4 font-sans text-sm font-medium",
        Size::Icon => "size-10 font-sans text-sm font-medium",
        Size::IconSm => "size-8 font-sans text-sm font-medium",
        Size::Inline => "rounded-sm",
    };
    cn([
        "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-pill transition-colors duration-150 ease-standard focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0",
        variant,
        size,
    ])
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Variant,
    #[prop(optional)] size: Size,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional)] submit: bool,
    #[prop(optional, into)] on_click: Option<Callback<leptos::ev::MouseEvent>>,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            type=if submit { "submit" } else { "button" }
            class=cn([button_class(variant, size).as_str(), class.as_str()])
            disabled=move || disabled.get()
            title=title
            aria-label=label
            on:click=move |e| {
                if let Some(f) = on_click {
                    f.run(e);
                }
            }
        >
            {children()}
        </button>
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardTone {
    #[default]
    Default,
    Muted,
    Contrast,
}

pub fn card_class(tone: CardTone) -> &'static str {
    match tone {
        // A section of the page itself: a rule above it, no box.
        CardTone::Default => "flat-section min-w-0 text-fg",
        // A quiet panel inside a section.
        CardTone::Muted => "rounded-lg bg-card-muted p-5 text-fg sm:p-6",
        CardTone::Contrast => {
            "rounded-xl border border-transparent bg-contrast p-5 text-fg-on-contrast sm:p-6"
        }
    }
}

#[component]
pub fn Card(
    #[prop(optional)] tone: CardTone,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div id=id aria-label=label class=cn([card_class(tone), class.as_str()])>
            {children()}
        </div>
    }
}

#[component]
pub fn CardHeader(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    view! { <div class=cn(["mb-4 flex items-start justify-between gap-3", class.as_str()])>{children()}</div> }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Heading {
    H2,
    #[default]
    H3,
    H4,
    P,
    Span,
}

/// A heading element chosen at runtime, so ports keep the site's levels.
fn heading(level: Heading, class: String, children: Children) -> AnyView {
    match level {
        Heading::H2 => view! { <h2 class=class>{children()}</h2> }.into_any(),
        Heading::H3 => view! { <h3 class=class>{children()}</h3> }.into_any(),
        Heading::H4 => view! { <h4 class=class>{children()}</h4> }.into_any(),
        Heading::P => view! { <p class=class>{children()}</p> }.into_any(),
        Heading::Span => view! { <span class=class>{children()}</span> }.into_any(),
    }
}

#[component]
pub fn CardTitle(
    #[prop(optional)] level: Heading,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    heading(
        level,
        cn(["text-lg font-semibold leading-tight", class.as_str()]),
        children,
    )
}

#[component]
pub fn CardDescription(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    view! { <p class=cn(["mt-1 text-sm text-fg-muted", class.as_str()])>{children()}</p> }
}

#[component]
pub fn Eyebrow(
    #[prop(optional)] level: Option<Heading>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    heading(
        level.unwrap_or(Heading::Span),
        cn([
            "text-eyebrow font-bold uppercase tracking-eyebrow text-fg-muted",
            class.as_str(),
        ]),
        children,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Positive,
    Negative,
    Warning,
    Outline,
}

#[component]
pub fn Badge(
    #[prop(optional)] tone: Tone,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let tone = match tone {
        Tone::Neutral => "border-border bg-soft text-fg",
        Tone::Accent => "border-transparent bg-accent-soft text-accent",
        Tone::Positive => "border-positive/30 bg-positive/10 text-positive-fg",
        Tone::Negative => "border-negative/30 bg-negative/10 text-negative-fg",
        Tone::Warning => "border-warning/30 bg-warning/10 text-warning-fg",
        Tone::Outline => "border-border text-fg",
    };
    view! {
        <span class=cn([
            "inline-flex items-center gap-1 rounded-pill border px-2.5 py-0.5 text-xs font-medium",
            tone,
            class.as_str(),
        ])>{children()}</span>
    }
}

/// `formatUnitMoney`: en-IE currency text with a true minus sign.
pub fn money(value: Decimal, unit: &Currency) -> String {
    format::money_dp(value, &unit.code, u32::from(unit.minor_unit))
}

/// The site's `Money`: tabular figures, red when negative unless `neutral`.
#[component]
pub fn Money(
    value: Decimal,
    unit: Currency,
    #[prop(optional)] neutral: bool,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let negative = value.is_sign_negative() && !value.is_zero();
    let tone = if !neutral && negative {
        "text-negative-fg"
    } else {
        ""
    };
    view! { <span class=cn(["tabular-nums", tone, class.as_str()])>{money(value, &unit)}</span> }
}

#[component]
pub fn Stat(
    #[prop(into)] label: String,
    children: Children,
    #[prop(optional, into)] delta: Option<String>,
    #[prop(optional)] tone: Tone,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let tone = match tone {
        Tone::Positive => "text-positive-fg",
        Tone::Negative => "text-negative-fg",
        _ => "text-fg-muted",
    };
    view! {
        <dl class=class>
            <div class="flex min-w-0 flex-col gap-0.5">
                <dt class="text-xs text-fg-muted">{label}</dt>
                <dd class="truncate text-base font-semibold tabular-nums">{children()}</dd>
                {delta.map(|d| view! { <dd class=format!("text-xs {tone}")>{d}</dd> })}
            </div>
        </dl>
    }
}

/// One dashboard field: a label, its value, an optional status chip and
/// labelled facts under it. Replaces a sentence that carried numbers.
#[component]
pub fn Metric(
    #[prop(into)] label: String,
    children: Children,
    #[prop(optional, into)] chip: Option<(Tone, String)>,
    #[prop(optional)] facts: Vec<(String, String)>,
    /// The headline size (`text-2xl` unless given).
    #[prop(optional, into)]
    size: Option<&'static str>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    view! {
        <div class=cn(["grid min-w-0 content-start gap-1.5", class.as_str()])>
            <span class="text-eyebrow font-bold uppercase tracking-eyebrow text-fg-muted">{label}</span>
            <div class="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                <strong class=cn(["font-semibold tabular-nums tracking-tight", size.unwrap_or("text-2xl")])>
                    {children()}
                </strong>
                {chip.map(|(tone, text)| view! { <Badge tone=tone>{text}</Badge> })}
            </div>
            <Facts rows=facts />
        </div>
    }
}

/// Small label/value rows: the numbers a sentence used to string together.
#[component]
pub fn Facts(rows: Vec<(String, String)>, #[prop(optional, into)] class: String) -> impl IntoView {
    (!rows.is_empty()).then(|| {
        view! {
            <dl class=cn(["grid max-w-72 gap-1 text-xs", class.as_str()])>
                {rows
                    .into_iter()
                    .map(|(label, value)| {
                        view! {
                            <div class="flex items-baseline justify-between gap-4">
                                <dt class="min-w-0 truncate text-fg-muted">{label}</dt>
                                <dd class="whitespace-nowrap font-medium tabular-nums">{value}</dd>
                            </div>
                        }
                    })
                    .collect_view()}
            </dl>
        }
    })
}

/// The same label/value pairs laid in a line: a row's details as fields
/// instead of one string joined with dots.
#[component]
pub fn FactsRow(
    rows: Vec<(String, String)>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    (!rows.is_empty()).then(|| {
        view! {
            <dl class=cn(["flex flex-wrap gap-x-6 gap-y-2 text-sm", class.as_str()])>
                {rows
                    .into_iter()
                    .map(|(label, value)| {
                        view! {
                            <div class="grid gap-0.5">
                                <dt class="text-eyebrow font-semibold uppercase tracking-eyebrow text-fg-muted">{label}</dt>
                                <dd class="tabular-nums">{value}</dd>
                            </div>
                        }
                    })
                    .collect_view()}
            </dl>
        }
    })
}

/// Fields side by side across the page, a rule between neighbours.
pub const METRIC_STRIP: &str = "metric-strip grid grid-cols-2 gap-x-6 gap-y-6 md:grid-cols-3";

#[component]
pub fn EmptyState(
    #[prop(into)] title: String,
    #[prop(optional)] icon: Option<Icon>,
    #[prop(optional, into)] description: Option<String>,
    #[prop(optional, into)] class: String,
    #[prop(optional)] action: Option<Children>,
) -> impl IntoView {
    view! {
        <div class=cn([
            "flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-8 text-center",
            class.as_str(),
        ])>
            {icon.map(|i| view! { <div aria-hidden="true" class="text-fg-muted"><Lucide icon=i size=18 /></div> })}
            <p class="text-base font-semibold">{title}</p>
            {description.map(|d| view! { <p class="max-w-prose text-sm text-fg-muted">{d}</p> })}
            {action.map(|a| view! { <div class="mt-2">{a()}</div> })}
        </div>
    }
}

#[component]
pub fn Skeleton(#[prop(optional, into)] class: String) -> impl IntoView {
    view! { <div aria-hidden="true" class=cn(["animate-pulse rounded-md bg-soft", class.as_str()])></div> }
}

#[component]
pub fn Switch(
    checked: Signal<bool>,
    on_change: Callback<bool>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] label: Option<String>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            role="switch"
            id=id
            aria-label=label
            aria-checked=move || checked.get().to_string()
            data-state=move || if checked.get() { "checked" } else { "unchecked" }
            disabled=move || disabled.get()
            class="inline-flex h-6 w-10 shrink-0 items-center rounded-pill border border-transparent bg-fg-muted p-0.5 data-[state=checked]:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
            on:click=move |_| on_change.run(!checked.get_untracked())
        >
            <span
                data-state=move || if checked.get() { "checked" } else { "unchecked" }
                class="block size-5 rounded-pill bg-card-strong shadow-xs transition-transform data-[state=checked]:translate-x-4"
            ></span>
        </button>
    }
}

pub const CONTROL: &str = "h-10 w-full rounded-sm border border-border bg-card-strong px-3 text-sm text-fg placeholder:text-fg-muted focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring aria-invalid:border-negative disabled:cursor-not-allowed disabled:opacity-50";

/// Keep an input's text equal to `value` without rewriting it on every
/// keystroke. Writing `value` to a number input while the owner types
/// "630." makes WebKit sanitise the text and throw the caret to the start,
/// so the digits that follow land in front; writing only when the element
/// already differs leaves the caret alone.
pub fn sync_value(node: NodeRef<html::Input>, value: impl Fn() -> String + Send + Sync + 'static) {
    Effect::new(move |_| {
        let wanted = value();
        if let Some(input) = node.get()
            && input.value() != wanted
        {
            input.set_value(&wanted);
        }
    });
}

/// A text input bound to `value`.
#[component]
pub fn Input(
    value: RwSignal<String>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(optional, into)] kind: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] step: Option<String>,
    #[prop(optional, into)] min: Option<String>,
    #[prop(optional, into)] max: Option<String>,
) -> impl IntoView {
    let node = NodeRef::<html::Input>::new();
    sync_value(node, move || value.get());
    view! {
        <input
            node_ref=node
            id=id
            type=kind.unwrap_or_else(|| "text".into())
            class=cn([CONTROL, class.as_str()])
            placeholder=placeholder
            aria-label=label
            step=step
            min=min
            max=max
            disabled=move || disabled.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

/// A select bound to `value`; `options` are (value, label) pairs.
#[component]
pub fn Select(
    value: RwSignal<String>,
    #[prop(into)] options: Signal<Vec<(String, String)>>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="relative w-full">
            <select
                id=id
                aria-label=label
                class=cn([CONTROL, "appearance-none pr-9", class.as_str()])
                disabled=move || disabled.get()
                on:change=move |e| value.set(event_target_value(&e))
                prop:value=move || value.get()
            >
                {move || {
                    options
                        .get()
                        .into_iter()
                        .map(|(v, l)| {
                            let selected = v == value.get_untracked();
                            view! { <option value=v selected=selected>{l}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

/// An explanation kept off the page: an info mark that shows `text` on
/// hover or focus, in place of a sentence under a control or a heading.
#[component]
pub fn Help(#[prop(into)] text: String, #[prop(optional, into)] class: String) -> impl IntoView {
    view! {
        <span
            tabindex="0"
            role="note"
            title=text.clone()
            aria-label=text
            class=cn([
                "inline-flex shrink-0 cursor-help rounded-pill text-fg-muted hover:text-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
                class.as_str(),
            ])
        >
            <Lucide icon=Icon::Info size=13 />
        </span>
    }
}

#[component]
pub fn Field(
    #[prop(into)] label: String,
    #[prop(into)] id: String,
    #[prop(optional, into)] hint: Option<String>,
    #[prop(optional, into)] error: Signal<Option<String>>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=cn(["grid gap-1.5", class.as_str()])>
            <span class="flex items-center gap-1.5">
                <label for=id class="text-sm font-medium text-fg">{label}</label>
                {hint.map(|h| view! { <Help text=h /> })}
            </span>
            {children()}
            {move || error.get().map(|e| view! { <p class="text-xs text-negative-fg">{e}</p> })}
        </div>
    }
}

#[component]
pub fn Checkbox(
    checked: Signal<bool>,
    on_change: Callback<bool>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            role="checkbox"
            aria-label=label
            aria-checked=move || checked.get().to_string()
            data-state=move || if checked.get() { "checked" } else { "unchecked" }
            disabled=move || disabled.get()
            class="flex size-4 shrink-0 items-center justify-center rounded-[4px] border border-fg-muted bg-card-strong data-[state=checked]:border-contrast data-[state=checked]:bg-contrast data-[state=checked]:text-fg-on-contrast focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
            on:click=move |_| on_change.run(!checked.get_untracked())
        >
            {move || checked.get().then(|| view! { <Lucide icon=Icon::Check size=12 stroke=3.0 /> })}
        </button>
    }
}

/// The names in a comma list, trimmed, without blanks.
pub fn list_names(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `names` with `name` switched: removed when present (any letter case),
/// added at the end otherwise.
pub fn toggle_name(names: &[String], name: &str) -> Vec<String> {
    let same = |n: &String| n.eq_ignore_ascii_case(name);
    if names.iter().any(same) {
        names.iter().filter(|n| !same(n)).cloned().collect()
    } else {
        names.iter().cloned().chain([name.to_owned()]).collect()
    }
}

/// Pick several of `options` as chips. `value` stays the comma list the
/// forms save; a saved name that is no longer an option keeps its chip.
#[component]
pub fn MultiSelect(
    value: RwSignal<String>,
    options: Vec<String>,
    #[prop(into)] label: String,
    #[prop(optional, into)] id: String,
) -> impl IntoView {
    // A long list shows what is chosen and a search box for the rest.
    let long = options.len() > LONG_LIST;
    let options = StoredValue::new(options);
    let query = RwSignal::new(String::new());
    let chips = move || {
        let chosen = list_names(&value.get());
        let mut all = options.get_value();
        for name in &chosen {
            if !all.iter().any(|o| o.eq_ignore_ascii_case(name)) {
                all.push(name.clone());
            }
        }
        let shown = shown_names(&all, &chosen, long.then(|| query.get()).as_deref());
        shown
            .into_iter()
            .map(|name| {
                let on = chosen.iter().any(|c| c.eq_ignore_ascii_case(&name));
                let text = name.clone();
                view! {
                    <button
                        type="button"
                        aria-pressed=on.to_string()
                        class=cn([
                            "inline-flex items-center gap-1 rounded-pill border px-3 py-1 text-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
                            if on { "border-contrast bg-contrast text-fg-on-contrast" } else { "border-border text-fg-muted hover:text-fg" },
                        ])
                        on:click=move |_| {
                            let next = toggle_name(&list_names(&value.get_untracked()), &name);
                            value.set(next.join(", "));
                        }
                    >
                        {on.then(|| view! { <Lucide icon=Icon::Check size=12 stroke=3.0 /> })}
                        {text}
                    </button>
                }
            })
            .collect_view()
    };
    let search = long.then(|| {
        let placeholder = format!("Search {} to add", options.with_value(Vec::len));
        view! {
            <input
                type="search"
                class=cn([CONTROL, "h-8 max-w-xs basis-full"])
                placeholder=placeholder
                aria-label=format!("Search {label}")
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
            />
        }
    });
    view! {
        <div id=id role="group" aria-label=label.clone() class="flex flex-wrap gap-1.5">
            {search}
            {chips}
            {move || {
                (options.with_value(Vec::is_empty) && list_names(&value.get()).is_empty())
                    .then(|| view! { <p class="text-sm text-fg-muted">"Nothing to choose from yet."</p> })
            }}
        </div>
    }
}

/// More options than this and [`MultiSelect`] searches instead of listing.
const LONG_LIST: usize = 24;

/// The chips to show: all of them for a short list (`query` is `None`);
/// for a long one the chosen names, then the others matching `query`.
pub fn shown_names(all: &[String], chosen: &[String], query: Option<&str>) -> Vec<String> {
    let Some(query) = query else {
        return all.to_vec();
    };
    let query = query.trim().to_lowercase();
    let is_chosen = |n: &String| chosen.iter().any(|c| c.eq_ignore_ascii_case(n));
    let mut out: Vec<String> = all.iter().filter(|n| is_chosen(n)).cloned().collect();
    if !query.is_empty() {
        out.extend(
            all.iter()
                .filter(|n| !is_chosen(n) && n.to_lowercase().contains(&query))
                .cloned(),
        );
    }
    out
}

/// The site's `SegmentedControl`: a pill of mutually exclusive options.
#[component]
pub fn SegmentedControl(
    value: Signal<String>,
    on_change: Callback<String>,
    items: Vec<(String, String)>,
    #[prop(into)] label: String,
    #[prop(optional)] small: bool,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let size = if small { "h-7 px-3 text-xs" } else { "h-8" };
    view! {
        <div
            role="group"
            aria-label=label
            class=cn([
                "glass-card inline-flex items-center gap-1 rounded-pill border border-border bg-card-muted p-1",
                class.as_str(),
            ])
        >
            {items
                .into_iter()
                .map(|(v, l)| {
                    let on = {
                        let v = v.clone();
                        move || value.get() == v
                    };
                    let on2 = on.clone();
                    view! {
                        <button
                            type="button"
                            aria-pressed=move || on().to_string()
                            data-state=move || if on2() { "on" } else { "off" }
                            class=cn([
                                "inline-flex items-center gap-1.5 rounded-pill px-3.5 text-sm text-fg-muted transition-colors hover:text-fg data-[state=on]:bg-card-strong data-[state=on]:text-fg data-[state=on]:shadow-xs focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
                                size,
                            ])
                            on:click=move |_| on_change.run(v.clone())
                        >
                            {l}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

pub const TABS_LIST: &str = "inline-flex items-center gap-1 rounded-pill bg-contrast p-1";
pub const TABS_TRIGGER: &str = "inline-flex min-h-9 items-center rounded-pill px-4 text-xs font-semibold text-fg-on-contrast/75 transition-colors hover:bg-fg-on-contrast/10 hover:text-fg-on-contrast focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-fg-on-contrast data-[state=active]:bg-page data-[state=active]:text-fg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Upcoming,
}

pub fn step_class(state: StepState) -> String {
    let state = match state {
        StepState::Done => "border-border text-fg",
        StepState::Current => "border-accent bg-accent-soft text-accent",
        StepState::Upcoming => "border-border text-fg-muted",
    };
    cn([
        "inline-flex min-h-8 items-center rounded-pill border px-3 text-xs font-medium transition-colors duration-150 ease-standard hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50",
        state,
    ])
}

/// A modal dialog over a dimmed backdrop; `open` controls it.
#[component]
pub fn Dialog(
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    #[prop(optional, into)] description: Option<String>,
    #[prop(optional, into)] class: String,
    children: ChildrenFn,
) -> impl IntoView {
    let description = StoredValue::new(description);
    let title = StoredValue::new(title);
    let class = StoredValue::new(class);
    view! {
        <Show when=move || open.get()>
            <div class="fixed inset-0 z-50 bg-contrast/40 backdrop-blur-sm" on:click=move |_| open.set(false)></div>
            <div
                role="dialog"
                aria-modal="true"
                aria-label=title.get_value()
                class=cn([
                    "glass-card fixed left-1/2 top-1/2 z-50 grid max-h-[90vh] w-[min(92vw,32rem)] -translate-x-1/2 -translate-y-1/2 gap-4 overflow-y-auto rounded-xl border border-border bg-card-strong p-6 text-fg shadow-pop",
                    class.get_value().as_str(),
                ])
                on:keydown=move |e| {
                    if e.key() == "Escape" {
                        open.set(false);
                    }
                }
            >
                <div class="grid gap-1.5 pr-10">
                    <h2 class="text-lg font-semibold">{title.get_value()}</h2>
                    {description.get_value().map(|d| view! { <p class="text-sm text-fg-muted">{d}</p> })}
                </div>
                {children()}
                <button
                    type="button"
                    aria-label="Close"
                    class=format!("absolute right-4 top-4 {}", button_class(Variant::Ghost, Size::IconSm))
                    on:click=move |_| open.set(false)
                >
                    <Lucide icon=Icon::X size=16 />
                </button>
            </div>
        </Show>
    }
}

pub const TABLE: &str = "group/table w-full border-collapse text-sm text-fg";
pub const TH: &str =
    "px-2 pb-2 text-eyebrow font-semibold uppercase tracking-eyebrow text-fg-muted";
pub const TH_NUM: &str = "px-2 pb-2 text-eyebrow font-semibold uppercase tracking-eyebrow text-fg-muted text-right tabular-nums";
pub const TROW: &str = "border-t border-line";
pub const TCELL: &str = "px-2 py-2.5 align-top group-data-[density=sm]/table:py-1.5";
pub const TCELL_NUM: &str =
    "px-2 py-2.5 align-top group-data-[density=sm]/table:py-1.5 text-right tabular-nums";
pub const TABLE_CONTAINER: &str = "w-full overflow-x-auto rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

/// `YYYY-MM` → "October 2026".
pub fn format_month(month: &str) -> String {
    format::month_long(month)
}

#[component]
pub fn MonthSwitcher(
    month: Signal<String>,
    on_change: Callback<String>,
    #[prop(optional, into)] min: Option<String>,
    #[prop(optional, into)] max: Option<String>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let previous = move || format::add_months(&month.get(), -1);
    let next = move || format::add_months(&month.get(), 1);
    view! {
        <div class=cn(["inline-flex items-center gap-2", class.as_str()])>
            <button
                type="button"
                aria-label="Previous month"
                class=button_class(Variant::Secondary, Size::IconSm)
                disabled=move || min.as_ref().is_some_and(|m| previous() < *m)
                on:click=move |_| on_change.run(previous())
            >
                <Lucide icon=Icon::ChevronLeft size=16 />
            </button>
            <span aria-live="polite" class="min-w-32 text-center text-sm font-semibold">
                {move || format_month(&month.get())}
            </span>
            <button
                type="button"
                aria-label="Next month"
                class=button_class(Variant::Secondary, Size::IconSm)
                disabled=move || max.as_ref().is_some_and(|m| next() > *m)
                on:click=move |_| on_change.run(next())
            >
                <Lucide icon=Icon::ChevronRight size=16 />
            </button>
        </div>
    }
}

/// `<details>` disclosure styled like the site's rows.
#[component]
pub fn Disclosure(
    summary: Children,
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(optional)] open: bool,
) -> impl IntoView {
    view! {
        <details class=cn(["group border-t border-line py-3.5", class.as_str()]) open=open>
            <summary class="flex cursor-pointer list-none items-center justify-between gap-3 text-sm [&::-webkit-details-marker]:hidden">
                {summary()}
            </summary>
            {children()}
        </details>
    }
}
