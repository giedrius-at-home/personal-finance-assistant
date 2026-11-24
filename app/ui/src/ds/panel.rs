//! Foldable page panels: the list a page's panels sit in, and the frame that
//! folds one of them to a single row.

use std::sync::atomic::{AtomicU32, Ordering};

use leptos::prelude::*;

use super::arrange::{DROP_TARGET, DropTarget, drag_after, drag_left};
use super::{CardTitle, Heading, Icon, Lucide, Size, Variant, button_class, cn};

/// Two columns from `md`, one below; the gap is `.page-stack`'s 1.5rem.
pub const PANEL_LIST: &str = "grid min-w-0 gap-6 md:grid-cols-2";
/// Safety net: an open panel whose body rendered nothing takes no room. A
/// panel's `shown` is the primary way to draw nothing.
const EMPTY_OPEN_HIDDEN: &str = "has-[>[aria-expanded=true]+div:empty]:hidden";
/// A full-width panel spans both columns; a half one takes one.
const PANEL_FULL: &str = "relative min-w-0 md:col-span-2";
const PANEL_HALF: &str = "relative min-w-0";
/// Open: a small button at the right end of the section's top rule, in the gap above it.
const FOLD_BUTTON: &str = "absolute bottom-full right-0 size-6";
/// Folded: the whole row on the rule is the button.
const FOLDED_ROW: &str = "flat-section flex w-full min-w-0 items-center gap-3 text-left text-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
/// Edit layout, a panel with nothing to show now: one muted row on the rule.
const QUIET_ROW: &str = "flat-section flex w-full min-w-0 items-center gap-3 text-fg-muted";

#[component]
pub fn PanelList(children: Children) -> impl IntoView {
    view! { <div class=PANEL_LIST>{children()}</div> }
}

/// One panel. `children` mount the first time it is open and stay mounted,
/// hidden, when it folds again; a panel folded from the start loads nothing.
#[component]
pub fn PanelFrame(
    /// The panel's id, as `data-panel`, so focus can find its controls when titles repeat.
    #[prop(optional, into)]
    panel: Option<String>,
    #[prop(into)] title: String,
    #[prop(into)] summary: Signal<Option<String>>,
    #[prop(into)] folded: Signal<bool>,
    on_toggle: Callback<()>,
    #[prop(optional, into)] half: Signal<bool>,
    /// Edit layout is on: an open panel with an empty body stays drawn, so it can be edited.
    #[prop(optional, into)]
    editing: Signal<bool>,
    /// Edit layout's controls, drawn above the top rule; empty outside it.
    #[prop(optional, into)]
    edit: ViewFn,
    /// A dragged panel may be let go here.
    #[prop(optional)]
    drop: Option<DropTarget>,
    /// Edit layout, nothing to show now: a muted row with `title`, no body.
    #[prop(optional)]
    quiet: bool,
    children: ChildrenFn,
) -> impl IntoView {
    let over = move || drop.is_some_and(|d| d.over.get());
    let class = move || {
        cn([
            if half.get() { PANEL_HALF } else { PANEL_FULL },
            if editing.get() { "" } else { EMPTY_OPEN_HIDDEN },
            if over() { DROP_TARGET } else { "" },
        ])
    };
    let accepts = move || drop.is_some_and(|d| d.accepts.get_untracked());
    let on_dragover = move |e: leptos::ev::DragEvent| {
        let Some(d) = drop.filter(|_| accepts()) else {
            return;
        };
        e.prevent_default();
        if let Some(after) = drag_after(&e, false) {
            d.on_over.run(after);
        }
    };
    let on_dragleave = move |e: leptos::ev::DragEvent| {
        if let Some(d) = drop.filter(|_| accepts())
            && drag_left(&e)
        {
            d.on_leave.run(());
        }
    };
    let on_drop = move |e: leptos::ev::DragEvent| {
        if let Some(d) = drop.filter(|_| accepts()) {
            e.prevent_default();
            d.on_drop.run(());
        }
    };
    let edit = move || edit.run();
    if quiet {
        return view! {
            <section class=class data-panel=panel.clone() on:dragover=on_dragover on:dragleave=on_dragleave on:drop=on_drop>
                {edit.clone()}
                <div class=QUIET_ROW>
                    <CardTitle level=Heading::Span class="min-w-0 truncate">{title}</CardTitle>
                    <span class="ml-auto truncate text-sm">"Nothing to show now"</span>
                </div>
            </section>
        }
        .into_any();
    }
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let body_id = format!("panel-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let mounted =
        Memo::new(move |was: Option<&bool>| was.copied().unwrap_or(false) || !folded.get());
    let open_class = cn([
        button_class(Variant::Ghost, Size::IconSm).as_str(),
        FOLD_BUTTON,
    ]);
    let label = format!("Fold {title}");
    view! {
        <section class=class data-panel=panel.clone() on:dragover=on_dragover on:dragleave=on_dragleave on:drop=on_drop>
            {edit.clone()}
            // One button for both states, so focus stays on it across a toggle.
            <button
                type="button"
                class=move || if folded.get() { FOLDED_ROW.to_owned() } else { open_class.clone() }
                aria-expanded=move || (!folded.get()).to_string()
                aria-controls=body_id.clone()
                aria-label=move || (!folded.get()).then(|| label.clone())
                on:click=move |_| on_toggle.run(())
            >
                {move || {
                    let title = title.clone();
                    folded
                        .get()
                        .then(move || {
                            view! {
                                <CardTitle level=Heading::Span class="min-w-0 truncate">{title}</CardTitle>
                                <span class="ml-auto truncate text-sm text-fg-muted">{move || summary.get()}</span>
                            }
                        })
                }}
                {move || {
                    let icon = if folded.get() { Icon::ChevronDown } else { Icon::ChevronUp };
                    view! { <span class="inline-flex shrink-0 text-fg-muted"><Lucide icon=icon size=16 /></span> }
                }}
            </button>
            <div id=body_id hidden=move || folded.get()>
                {move || mounted.get().then(|| children())}
            </div>
        </section>
    }
    .into_any()
}
