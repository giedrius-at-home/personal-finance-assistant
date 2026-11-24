//! Edit layout parts: a panel's edit controls, the "Add a panel" gallery and
//! an inline confirm button. Drawn only while Edit layout is on.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::patterns::{ACTIONS, CHIPS, MUTED};
use super::{Button, CardTitle, Heading, Icon, Lucide, Size, Variant, button_class, cn};

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
    on_yes: Callback<()>,
) -> impl IntoView {
    let asking = RwSignal::new(false);
    let cancel = NodeRef::<html::Button>::new();
    // Focus follows the question, on the safe answer: a held Enter cannot repeat into "Yes".
    Effect::new(move |_| {
        if let Some(b) = cancel.get() {
            let _ = b.focus();
        }
    });
    let text = StoredValue::new((label, question));
    move || {
        if asking.get() {
            view! {
                <span class=ACTIONS>
                    <span class=MUTED>{text.with_value(|t| t.1.clone())}</span>
                    <button
                        type="button"
                        class=button_class(Variant::Danger, Size::Sm)
                        on:click=move |_| {
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
                        on:click=move |_| asking.set(false)
                    >
                        "Cancel"
                    </button>
                </span>
            }
            .into_any()
        } else {
            view! {
                <Button variant=Variant::Secondary size=Size::Sm on_click=Callback::new(move |_| asking.set(true))>
                    {text.with_value(|t| t.0.clone())}
                </Button>
            }
                .into_any()
        }
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

/// Focuses the button labelled `label` once the edit is drawn.
pub fn focus_label(label: String) {
    set_timeout(
        move || {
            let sel = format!("button[aria-label=\"{}\"]", label.replace('"', "\\\""));
            if let Some(b) = document()
                .query_selector(&sel)
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
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
    fn the_ends_cannot_move_past_the_list() {
        assert_eq!(moves(0, 3), (None, Some(1)));
        assert_eq!(moves(1, 3), (Some(0), Some(2)));
        assert_eq!(moves(2, 3), (Some(1), None));
        assert_eq!(moves(0, 1), (None, None));
    }
}
