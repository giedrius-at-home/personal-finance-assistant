//! Edit layout parts: a panel's edit controls, the "Add a panel" gallery, an
//! inline confirm button and a tab of the tab editor. Drawn only while Edit
//! layout is on.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::patterns::{ACTIONS, CHIPS, MUTED};
use super::{Button, CardTitle, Heading, Icon, Input, Lucide, Size, Variant, button_class, cn};

/// The edit controls: in the gap above the section's top rule, left of the fold button.
const EDIT_BAR: &str = "absolute bottom-full right-7 flex items-center gap-0.5";
/// An icon control of the edit bar, as small as the fold button.
const EDIT_ICON: &str = "size-6";
/// A word control of the edit bar ("Half" / "Full").
const EDIT_WORD: &str = "h-6 px-2";
const GRIP: &str = "size-6 cursor-grab active:cursor-grabbing";
/// The frame a dragged panel would land on.
pub const DROP_TARGET: &str = "ring-2 ring-accent/50";
/// The "Add a panel" box.
const GALLERY: &str = "grid gap-3 rounded-xl border border-dashed border-border p-5";
/// The tab bar while editing: each tab with its controls, wrapping.
pub const TAB_EDITOR: &str = "flex w-full min-w-0 flex-wrap items-center gap-1 rounded-xl border border-dashed border-border bg-card-strong p-1";
/// One tab of the tab editor with its controls.
const TAB_CHIP: &str = "inline-flex flex-wrap items-center gap-0.5 rounded-pill";
/// A hidden tab in the tab editor.
const TAB_HIDDEN: &str = "opacity-60";
/// A tab's name in the tab editor; it is also what a tab is dragged by.
const TAB_NAME: &str = "data-[state=active]:bg-soft data-[state=active]:text-fg";
/// The rename field of a tab.
const TAB_NAME_FIELD: &str = "h-8 w-40";
/// A refused tab name, on its own line under the tabs.
pub const TAB_ALERT: &str = "basis-full px-2";

/// A frame's part as a drop target while a panel is dragged.
#[derive(Clone, Copy)]
pub struct DropTarget {
    /// A panel is being dragged.
    pub accepts: Signal<bool>,
    /// Show the drop highlight.
    pub over: Signal<bool>,
    /// A dragged panel is over the frame; true: past its middle.
    pub on_over: Callback<bool>,
    pub on_leave: Callback<()>,
    pub on_drop: Callback<()>,
}

/// The edit controls of one panel. The grip drags the whole `<section>` it
/// sits in; `on_grab` runs with true when a drag starts and false when it ends.
#[component]
pub fn EditBar(
    #[prop(into)] title: String,
    #[prop(into)] first: Signal<bool>,
    #[prop(into)] last: Signal<bool>,
    #[prop(into)] half: Signal<bool>,
    on_grab: Callback<bool>,
    on_up: Callback<()>,
    on_down: Callback<()>,
    on_half: Callback<()>,
    on_remove: Callback<()>,
) -> impl IntoView {
    let icon = cn([
        button_class(Variant::Ghost, Size::IconSm).as_str(),
        EDIT_ICON,
    ]);
    let grip = cn([button_class(Variant::Ghost, Size::IconSm).as_str(), GRIP]);
    let word = cn([button_class(Variant::Ghost, Size::Sm).as_str(), EDIT_WORD]);
    view! {
        <div class=EDIT_BAR>
            <button
                type="button"
                class=grip
                draggable="true"
                tabindex="-1"
                aria-label=format!("Drag {title}")
                on:dragstart=move |e| {
                    if let Some(data) = e.data_transfer() {
                        let _ = data.set_data("text/plain", "panel");
                        // The whole frame follows the pointer, not just the grip.
                        if let Some(frame) = e
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                            .and_then(|b| b.closest("section").ok().flatten())
                        {
                            let r = frame.get_bounding_client_rect();
                            let (x, y) = (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top());
                            data.set_drag_image(&frame, x as i32, y as i32);
                        }
                    }
                    on_grab.run(true);
                }
                on:dragend=move |_| on_grab.run(false)
            >
                <Lucide icon=Icon::GripVertical size=14 />
            </button>
            <button
                type="button"
                class=icon.clone()
                aria-label=format!("Move {title} up")
                disabled=move || first.get()
                on:click=move |_| on_up.run(())
            >
                <Lucide icon=Icon::ArrowUp size=14 />
            </button>
            <button
                type="button"
                class=icon.clone()
                aria-label=format!("Move {title} down")
                disabled=move || last.get()
                on:click=move |_| on_down.run(())
            >
                <Lucide icon=Icon::ArrowDown size=14 />
            </button>
            <button
                type="button"
                class=word
                aria-label={
                    let title = title.clone();
                    move || half_label(&title, half.get())
                }
                on:click=move |_| on_half.run(())
            >
                {move || if half.get() { "Full" } else { "Half" }}
            </button>
            <button
                type="button"
                class=icon
                aria-label=format!("Remove {title}")
                on:click=move |_| on_remove.run(())
            >
                <Lucide icon=Icon::X size=14 />
            </button>
        </div>
    }
}

/// The dashed "Add a panel" box: one button per panel that may be added.
#[component]
pub fn Gallery(items: Signal<Vec<(String, String)>>, on_add: Callback<String>) -> impl IntoView {
    view! {
        <section aria-label="Add a panel" class=GALLERY>
            <CardTitle level=Heading::H3>"Add a panel"</CardTitle>
            {move || {
                let items = items.get();
                if items.is_empty() {
                    view! { <p class=MUTED>"Every panel is on this page."</p> }.into_any()
                } else {
                    view! {
                        <div class=CHIPS>
                            {items
                                .into_iter()
                                .map(|(id, title)| {
                                    view! {
                                        <Button
                                            variant=Variant::Secondary
                                            size=Size::Sm
                                            label=format!("Add {title}")
                                            on_click=Callback::new(move |_| on_add.run(id.clone()))
                                        >
                                            <Lucide icon=Icon::Plus size=14 />
                                            {title}
                                        </Button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

/// A button that asks once inline: `question`, "Yes", "Cancel".
#[component]
pub fn Confirm(
    #[prop(into)] label: String,
    #[prop(into)] question: String,
    /// The button's `aria-label`, when `label` alone does not say what it acts on.
    #[prop(optional, into)]
    aria: Option<String>,
    on_yes: Callback<()>,
) -> impl IntoView {
    let asking = RwSignal::new(false);
    let cancel = NodeRef::<html::Button>::new();
    let ask = NodeRef::<html::Button>::new();
    let answered = StoredValue::new(false);
    // Focus follows the question, on the safe answer: a held Enter cannot repeat into "Yes".
    Effect::new(move |_| {
        if let Some(b) = cancel.get() {
            let _ = b.focus();
        }
    });
    // After an answer, back on the button that asked (when it is still there).
    Effect::new(move |_| {
        if let Some(b) = ask.get()
            && answered.get_value()
        {
            answered.set_value(false);
            let _ = b.focus();
        }
    });
    let text = StoredValue::new((label, question, aria));
    move || {
        if asking.get() {
            view! {
                <span class=ACTIONS>
                    <span class=MUTED>{text.with_value(|t| t.1.clone())}</span>
                    <button
                        type="button"
                        class=button_class(Variant::Danger, Size::Sm)
                        on:click=move |_| {
                            answered.set_value(true);
                            asking.set(false);
                            on_yes.run(());
                        }
                    >
                        "Yes"
                    </button>
                    <button
                        type="button"
                        node_ref=cancel
                        class=button_class(Variant::Ghost, Size::Sm)
                        on:click=move |_| {
                            answered.set_value(true);
                            asking.set(false);
                        }
                    >
                        "Cancel"
                    </button>
                </span>
            }
            .into_any()
        } else {
            view! {
                <button
                    type="button"
                    node_ref=ask
                    class=button_class(Variant::Secondary, Size::Sm)
                    aria-label=text.with_value(|t| t.2.clone())
                    on:click=move |_| asking.set(true)
                >
                    {text.with_value(|t| t.0.clone())}
                </button>
            }
            .into_any()
        }
    }
}

/// One tab of the tab editor: its name (click: show it; drag: move it, or drop
/// a panel on it), "Move left"/"Move right", and Rename/Delete for an own tab
/// or Hide for a built-in one. A hidden tab is muted and shows only "Show".
#[component]
pub fn TabChip(
    #[prop(into)] name: String,
    #[prop(into)] active: Signal<bool>,
    /// An own tab: rename and delete.
    own: bool,
    /// Some(hidden): a built-in tab that may be hidden (every one but Manage).
    hide: Option<bool>,
    #[prop(into)] first: Signal<bool>,
    #[prop(into)] last: Signal<bool>,
    /// The rename field is open; its text is `draft`.
    #[prop(into)]
    renaming: Signal<bool>,
    draft: RwSignal<String>,
    drop: DropTarget,
    on_grab: Callback<bool>,
    on_pick: Callback<()>,
    on_left: Callback<()>,
    on_right: Callback<()>,
    on_hide: Callback<bool>,
    on_rename: Callback<()>,
    on_save: Callback<()>,
    on_cancel: Callback<()>,
    on_delete: Callback<()>,
) -> impl IntoView {
    let hidden = hide == Some(true);
    let icon = cn([
        button_class(Variant::Ghost, Size::IconSm).as_str(),
        EDIT_ICON,
    ]);
    let word = cn([button_class(Variant::Ghost, Size::Sm).as_str(), EDIT_WORD]);
    let accepts = move || drop.accepts.get_untracked();
    let on_dragover = move |e: leptos::ev::DragEvent| {
        if accepts() {
            e.prevent_default();
            if let Some(after) = drag_after(&e, true) {
                drop.on_over.run(after);
            }
        }
    };
    let on_dragleave = move |e: leptos::ev::DragEvent| {
        if accepts() && drag_left(&e) {
            drop.on_leave.run(());
        }
    };
    let on_drop = move |e: leptos::ev::DragEvent| {
        if accepts() {
            e.prevent_default();
            drop.on_drop.run(());
        }
    };
    let class = move || {
        cn([
            TAB_CHIP,
            if hidden { TAB_HIDDEN } else { "" },
            if drop.over.get() { DROP_TARGET } else { "" },
        ])
    };
    let name = StoredValue::new(name);
    let label = move |verb: &str| name.with_value(|n| format!("{verb} {n}"));
    let tab = move || {
        if renaming.get() {
            view! {
                <span on:keydown=move |e: leptos::ev::KeyboardEvent| match e.key().as_str() {
                    "Enter" => {
                        e.prevent_default();
                        on_save.run(());
                    }
                    // Esc closes the field only, not Edit layout.
                    "Escape" => {
                        e.stop_propagation();
                        on_cancel.run(());
                    }
                    _ => {}
                }>
                    <Input value=draft class=TAB_NAME_FIELD label=label("Name of") />
                </span>
            }
            .into_any()
        } else {
            view! {
                <button
                    type="button"
                    class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), TAB_NAME])
                    data-state=move || if active.get() { "active" } else { "inactive" }
                    aria-current=move || active.get().then_some("page")
                    draggable=if hidden { "false" } else { "true" }
                    on:dragstart=move |e| {
                        if let Some(data) = e.data_transfer() {
                            let _ = data.set_data("text/plain", "tab");
                        }
                        on_grab.run(true);
                    }
                    on:dragend=move |_| on_grab.run(false)
                    on:click=move |_| on_pick.run(())
                >
                    {name.get_value()}
                </button>
            }
            .into_any()
        }
    };
    let tools = if hidden {
        view! {
            <button type="button" class=icon aria-label=label("Show") on:click=move |_| on_hide.run(false)>
                <Lucide icon=Icon::Eye size=14 />
            </button>
        }
        .into_any()
    } else {
        view! {
            <button
                type="button"
                class=icon.clone()
                aria-label=label("Move")+" left"
                disabled=move || first.get()
                on:click=move |_| on_left.run(())
            >
                <Lucide icon=Icon::ArrowLeft size=14 />
            </button>
            <button
                type="button"
                class=icon.clone()
                aria-label=label("Move")+" right"
                disabled=move || last.get()
                on:click=move |_| on_right.run(())
            >
                <Lucide icon=Icon::ArrowRight size=14 />
            </button>
            {own.then(|| view! {
                <button type="button" class=word.clone() aria-label=label("Rename") on:click=move |_| on_rename.run(())>
                    "Rename"
                </button>
                <Confirm
                    label="Delete"
                    aria=label("Delete")
                    question="Delete this tab? Its panels stay available."
                    on_yes=on_delete
                />
            })}
            {hide.is_some().then(|| view! {
                <button type="button" class=icon.clone() aria-label=label("Hide") on:click=move |_| on_hide.run(true)>
                    <Lucide icon=Icon::EyeOff size=14 />
                </button>
            })}
        }
        .into_any()
    };
    view! {
        <span class=class on:dragover=on_dragover on:dragleave=on_dragleave on:drop=on_drop>
            {tab}
            {tools}
        </span>
    }
}

/// Where a dragged item lands: `over` is the item under the pointer, `after` whether the pointer is past its middle. Indexes are positions in the list before the move; the result is the `to` for `panel_moved`.
pub fn drop_index(from: usize, over: usize, after: bool) -> usize {
    let at = over + usize::from(after);
    if from < at { at - 1 } else { at }
}

/// The `to` for `panel_moved` of a drop; None when the panel would stay where it is.
pub fn drop_move(from: usize, over: usize, after: bool) -> Option<usize> {
    Some(drop_index(from, over, after)).filter(|&to| to != from)
}

/// The pointer at `pos` is past the middle of the span from `start`, `size` long.
pub fn past_middle(pos: f64, start: f64, size: f64) -> bool {
    pos > start + size / 2.0
}

/// For a dragover on a drop target: whether the pointer is past the middle of
/// the target (across: by X, else by Y).
pub fn drag_after(e: &leptos::ev::DragEvent, across: bool) -> Option<bool> {
    let r = e
        .current_target()?
        .dyn_into::<web_sys::Element>()
        .ok()?
        .get_bounding_client_rect();
    Some(if across {
        past_middle(f64::from(e.client_x()), r.left(), r.width())
    } else {
        past_middle(f64::from(e.client_y()), r.top(), r.height())
    })
}

/// For a dragleave: the pointer left the target, not just moved onto one of its children.
pub fn drag_left(e: &leptos::ev::DragEvent) -> bool {
    let target = e
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Node>().ok());
    let into = e
        .related_target()
        .and_then(|t| t.dyn_into::<web_sys::Node>().ok());
    target.is_some_and(|t| !t.contains(into.as_ref()))
}

/// The Half/Full control's label.
pub fn half_label(title: &str, half: bool) -> String {
    format!(
        "Make {title} {}",
        if half { "full width" } else { "half width" }
    )
}

/// What was just done to the panel titled `title`, for `focus_after`.
#[derive(Clone, Copy, Debug)]
pub enum Edited {
    Up,
    Down,
    Half,
    /// It was at this index.
    Removed(usize),
    Added,
}

/// The first enabled edit control of the panel at `at` of `len` (the grip is not tabbable).
fn first_control(title: &str, at: usize, len: usize, half: bool) -> String {
    match moves(at, len) {
        (Some(_), _) => format!("Move {title} up"),
        (None, Some(_)) => format!("Move {title} down"),
        (None, None) => half_label(title, half),
    }
}

/// The `aria-label` of the control that takes focus after an edit, so it is
/// not lost when the pressed control is redrawn, disabled or gone. `items`
/// are the page's (title, half) after the edit.
pub fn focus_after(edited: Edited, title: &str, items: &[(&str, bool)]) -> String {
    let len = items.len();
    let at = items.iter().position(|(t, _)| *t == title);
    let half = at.is_some_and(|p| items[p].1);
    match (edited, at) {
        (Edited::Up, Some(0)) => format!("Move {title} down"),
        (Edited::Up, _) => format!("Move {title} up"),
        (Edited::Down, Some(p)) if p + 1 == len => format!("Move {title} up"),
        (Edited::Down, _) => format!("Move {title} down"),
        (Edited::Half, _) => half_label(title, half),
        (Edited::Removed(i), _) => match items.get(i) {
            Some((next, h)) => first_control(next, i, len, *h),
            // Nothing after it: its button in "Add a panel".
            None => format!("Add {title}"),
        },
        (Edited::Added, Some(p)) => first_control(title, p, len, half),
        (Edited::Added, None) => format!("Add {title}"),
    }
}

/// Focuses the button or field labelled `label` once the edit is drawn; a
/// field's text is selected.
pub fn focus_label(label: String) {
    let label = label.replace('"', "\\\"");
    focus_on(format!(
        "button[aria-label=\"{label}\"], input[aria-label=\"{label}\"]"
    ));
}

/// Focuses the first element matching `selector` once the edit is drawn.
pub fn focus_on(selector: String) {
    set_timeout(
        move || {
            let Some(el) = document().query_selector(&selector).ok().flatten() else {
                return;
            };
            if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
                let _ = input.focus();
                input.select();
            } else if let Some(b) = el.dyn_ref::<web_sys::HtmlElement>() {
                let _ = b.focus();
            }
        },
        std::time::Duration::ZERO,
    );
}

/// The `to` of "Move up" and "Move down" for the item at `at` of `len`; None
/// disables the control.
pub fn moves(at: usize, len: usize) -> (Option<usize>, Option<usize>) {
    (at.checked_sub(1), (at + 1 < len).then_some(at + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_index_lands_where_the_pointer_is() {
        assert_eq!(drop_index(0, 2, true), 2);
        assert_eq!(drop_index(0, 2, false), 1);
        assert_eq!(drop_index(3, 1, false), 1);
        assert_eq!(drop_index(3, 1, true), 2);
        assert_eq!(drop_index(1, 1, false), 1);
        assert_eq!(drop_index(1, 1, true), 1);
    }

    #[test]
    fn a_move_to_the_same_index_is_not_an_edit() {
        assert_eq!(drop_move(1, 1, false), None);
        assert_eq!(drop_move(1, 1, true), None);
        assert_eq!(drop_move(1, 0, true), None);
        assert_eq!(drop_move(0, 2, true), Some(2));
    }

    const AFTER: [(&str, bool); 3] = [("A", false), ("B", true), ("C", false)];

    #[test]
    fn focus_stays_on_the_pressed_control_or_its_partner() {
        assert_eq!(focus_after(Edited::Up, "A", &AFTER), "Move A down");
        assert_eq!(focus_after(Edited::Up, "B", &AFTER), "Move B up");
        assert_eq!(focus_after(Edited::Down, "C", &AFTER), "Move C up");
        assert_eq!(focus_after(Edited::Down, "B", &AFTER), "Move B down");
        assert_eq!(focus_after(Edited::Half, "B", &AFTER), "Make B full width");
        assert_eq!(focus_after(Edited::Half, "C", &AFTER), "Make C half width");
    }

    #[test]
    fn focus_after_remove_and_add() {
        // "X" was removed from index 0: A is first now.
        assert_eq!(focus_after(Edited::Removed(0), "X", &AFTER), "Move A down");
        assert_eq!(focus_after(Edited::Removed(2), "X", &AFTER), "Move C up");
        // The last one went: its button in "Add a panel".
        assert_eq!(focus_after(Edited::Removed(3), "X", &AFTER), "Add X");
        assert_eq!(focus_after(Edited::Removed(0), "X", &[]), "Add X");
        assert_eq!(focus_after(Edited::Added, "C", &AFTER), "Move C up");
        assert_eq!(
            focus_after(Edited::Added, "A", &[("A", true)]),
            "Make A full width"
        );
    }

    #[test]
    fn the_far_half_of_a_target_drops_after_it() {
        assert!(!past_middle(10.0, 0.0, 100.0));
        assert!(!past_middle(50.0, 0.0, 100.0));
        assert!(past_middle(51.0, 0.0, 100.0));
        assert!(past_middle(190.0, 100.0, 100.0));
    }

    #[test]
    fn the_ends_cannot_move_past_the_list() {
        assert_eq!(moves(0, 3), (None, Some(1)));
        assert_eq!(moves(1, 3), (Some(0), Some(2)));
        assert_eq!(moves(2, 3), (Some(1), None));
        assert_eq!(moves(0, 1), (None, None));
    }
}
