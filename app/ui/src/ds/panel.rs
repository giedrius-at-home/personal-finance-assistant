//! Foldable page panels: the list a page's panels sit in, and the frame that
//! folds one of them to a single row.

use std::sync::atomic::{AtomicU32, Ordering};

use leptos::prelude::*;

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

#[component]
pub fn PanelList(children: Children) -> impl IntoView {
    view! { <div class=PANEL_LIST>{children()}</div> }
}

/// One panel. `children` mount the first time it is open and stay mounted,
/// hidden, when it folds again; a panel folded from the start loads nothing.
#[component]
pub fn PanelFrame(
    #[prop(into)] title: String,
    #[prop(into)] summary: Signal<Option<String>>,
    #[prop(into)] folded: Signal<bool>,
    on_toggle: Callback<()>,
    #[prop(optional)] half: bool,
    children: ChildrenFn,
) -> impl IntoView {
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
        <section class=cn([if half { PANEL_HALF } else { PANEL_FULL }, EMPTY_OPEN_HIDDEN])>
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
}
