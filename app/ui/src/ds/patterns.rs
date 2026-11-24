//! Class strings and markup the screens repeat, named for their role so a
//! screen says what a piece is and the look lives in one place.

use leptos::prelude::*;

use super::cn;

/// Secondary body text: empty states, status lines, notes.
pub const MUTED: &str = "text-sm text-fg-muted";

/// Small secondary text: terms, captions, inline hints.
pub const HINT: &str = "text-xs text-fg-muted";

/// Vertical stack of sections or cards.
pub const STACK: &str = "grid gap-4";

/// Tight vertical stack: a title over its subline.
pub const STACK_XS: &str = "grid gap-1";

/// Vertical stack of related blocks.
pub const STACK_MD: &str = "grid gap-3";

/// Vertical stack of rows or lines.
pub const STACK_SM: &str = "grid gap-2";

/// A label over its control.
pub const FIELD: &str = "grid gap-1.5";

/// An amount that never wraps.
pub const FIGURE: &str = "whitespace-nowrap tabular-nums";

/// A row of buttons that wraps.
pub const ACTIONS: &str = "flex flex-wrap items-center gap-2";

/// Secondary line under a row's title.
pub const SUBLINE: &str = "mt-1 block text-xs text-fg-muted";

/// Title of a card or block.
pub const TITLE: &str = "text-base font-semibold";

/// A badge inside a dense row.
pub const BADGE_TIGHT: &str = "px-2 py-0";

/// Small secondary text on its own line.
pub const SMALL: &str = "block text-xs text-fg-muted";

/// Two or three controls side by side.
pub const PAIR: &str = "flex gap-2";

/// A wrapping row of chips or steps.
pub const CHIPS: &str = "flex flex-wrap gap-2";

/// The label of a form field.
pub const FIELD_LABEL: &str = "text-sm font-medium text-fg";

/// Wrapper of a native select and its chevron.
pub const SELECT_WRAP: &str = "relative w-full";

/// The chevron drawn over a native select.
pub const SELECT_CHEVRON: &str =
    "pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted";

/// Text cut to one line inside a flex or grid row.
pub const TRUNCATE: &str = "min-w-0 truncate";

/// Loading or unavailable note under a settings heading.
pub const STATUS_NOTE: &str = "mt-5 text-sm text-fg-muted";

/// A right-aligned secondary figure in a table row.
pub const NUM_MUTED: &str = "text-right text-fg-muted";

/// Heading of a board or panel, under its eyebrow.
pub const BOARD_TITLE: &str = "mt-1 text-lg font-semibold";

/// A name with its amount on the far side.
pub const NAME_ROW: &str = "flex items-baseline justify-between gap-3";

/// A small icon or swatch with its label.
pub const LEGEND_ITEM: &str = "flex items-center gap-1.5";

/// The box of a checkbox beside a multi-line label.
pub const CHECK_SLOT: &str = "mt-1 flex";

/// An error line.
pub const WARN: &str = "text-sm text-negative-fg";

/// Loading placeholder for a card.
pub const SKELETON_CARD: &str = "h-56 rounded-xl";

/// Name on a board card, cut to one line.
pub const CARD_NAME: &str = "min-w-0 truncate font-semibold";

/// A list of blocks under a heading.
pub const LIST_STACK: &str = "mt-4 grid gap-3";

/// A figure under its term in a definition list.
pub const DD_NUM: &str = "mt-1.5 text-sm tabular-nums";

/// A warning line.
pub const WARNING_TEXT: &str = "text-sm text-warning-fg";

/// Title of a list item on its own line.
pub const ITEM_TITLE: &str = "block text-sm font-semibold";

/// Fields and buttons aligned on their baseline, wrapping.
pub const FILTER_ROW: &str = "flex flex-wrap items-end gap-2";

/// A four-column form row.
pub const EVENT_FORM: &str = "grid items-end gap-2 sm:grid-cols-4";

/// A small figure.
pub const NUM: &str = "text-sm tabular-nums";

/// A padded row of a nested list.
pub const SUB_ROW: &str = "grid gap-2 py-3";

/// A heading or summary with controls on the far side, wrapping.
pub const TOOLBAR: &str = "flex flex-wrap items-center justify-between gap-3";

/// A label and value pair in small print.
pub const META_ROW: &str = "flex justify-between gap-3 text-xs text-fg-muted";

/// The value in a small-print pair.
pub const META_VALUE: &str = "font-medium text-fg";

/// Two columns from medium width.
pub const TWO_COLUMNS: &str = "grid gap-4 md:grid-cols-2";

/// A field that grows from 12rem in a wrapping row.
pub const GROW_48: &str = "min-w-0 flex-1 basis-48";

/// A bank payment line: date, text, amount, action.
pub const PAYMENT_ROW: &str = "grid grid-cols-[6.5rem_1fr_auto_auto] items-center gap-x-3 border-t border-line py-1.5 text-sm first:border-t-0";

/// A date in a payment line.
pub const DATE: &str = "text-fg-muted tabular-nums";

/// Stack of a Manage screen's sections.
pub const SECTION_STACK: &str = "mb-6 grid gap-4";

/// A value under its term in a definition list.
pub const DD: &str = "mt-1.5 text-sm";

/// Title of a row on its own line.
pub const ROW_TITLE: &str = "block font-semibold";

/// A block that follows its section at small text size.
pub const SUBSECTION: &str = "mt-4 text-sm";

/// A field that grows from 14rem in a wrapping row.
pub const GROW_56: &str = "min-w-0 flex-1 basis-56";

/// A list without bullets or padding.
pub const PLAIN_LIST: &str = "mb-3 mt-2 list-none p-0";

/// A line of a ledger: label and amount.
pub const LEDGER_ROW: &str =
    "flex justify-between gap-5 border-b border-border py-2.5 text-sm last:border-b-0";

/// A card's heading with its side content.
pub const CARD_HEADER: &str = "flex items-start justify-between gap-4";

/// Emphasised small text.
pub const STRONG_SM: &str = "text-sm font-semibold";

/// A wrapping row of small controls and text.
pub const INLINE_TOOLS: &str = "flex flex-wrap items-center gap-2 text-sm";

/// Heading of a panel, under its eyebrow.
pub const PANEL_TITLE: &str = "mt-2 text-lg font-semibold";

/// A muted inset box.
pub const INSET: &str = "rounded-lg border border-border bg-card-muted px-4 py-3";

/// A nested row, indented.
pub const NESTED: &str = "border-t border-line pl-4 sm:pl-6";

/// A row divided from the one above.
pub const DIVIDED: &str = "border-t border-line first:border-t-0";

/// A horizontally scrolling region.
pub const SCROLL_X: &str = "mt-2 overflow-x-auto";

/// A table that keeps its width and scrolls.
pub const WIDE_TABLE: &str = "min-w-[44rem] text-sm tabular-nums";

/// A section that fills its column.
pub const FILL_MIN: &str = "w-full min-w-0";

/// A colour dot.
pub const DOT: &str = "size-2 rounded-pill";

/// Emphasised text in the main colour.
pub const STRONG: &str = "font-semibold text-fg";

/// Title of a screen.
pub const PAGE_TITLE: &str = "mt-1 text-2xl font-semibold tracking-tight";

/// Title of a step.
pub const STEP_TITLE: &str = "text-lg font-semibold leading-tight";

/// Short loading placeholder.
pub const SKELETON_SHORT: &str = "h-32 rounded-xl";

/// Track of a thin meter.
pub const METER: &str = "block h-1.5 overflow-hidden rounded-pill bg-soft";

/// Labels under a thin meter.
pub const METER_SCALE: &str = "flex justify-between text-xs text-fg-muted";

/// A checkbox label in a form's first column.
pub const CHECK_LABEL_START: &str = "flex items-center gap-2.5 text-sm sm:col-start-1";

/// A four-column form row below other content.
pub const EVENT_FORM_SPACED: &str = "mt-3 grid items-end gap-2 sm:grid-cols-4";

/// A checkbox with a multi-line label, spaced from what follows.
pub const OPTION_SPACED: &str = "mb-5 flex items-start gap-2.5 text-sm leading-relaxed";

/// A definition list in three columns.
pub const DL_THREE: &str = "grid gap-4 sm:grid-cols-3";

/// A wrapping row of facts.
pub const FACT_ROW: &str = "flex flex-wrap items-end gap-x-6 gap-y-2";

/// A one-field form with its button.
pub const FILE_FORM: &str = "mt-4 grid items-end gap-4 sm:grid-cols-[minmax(0,1fr)_auto]";

/// A checkbox with a multi-line label.
pub const OPTION: &str = "flex items-start gap-2.5 text-sm leading-relaxed";

/// A divided part of a settings card.
pub const SUBPANEL: &str = "mt-5 grid gap-3 border-t border-line pt-5";

/// A secret field with its save button.
pub const SECRET_FORM: &str = "flex flex-col gap-3 sm:flex-row sm:items-end";

/// A spaced row of buttons that wraps.
pub const ACTIONS_LOOSE: &str = "flex flex-wrap items-center gap-3";

/// A definition list in three columns below a heading.
pub const DL_THREE_SPACED: &str = "mt-5 grid gap-4 sm:grid-cols-3";

/// A stack below a heading.
pub const SUBSTACK: &str = "mt-5 grid gap-3";

/// Two close columns from medium width.
pub const TWO_COLUMNS_TIGHT: &str = "grid gap-3 md:grid-cols-2";

/// Title of a card.
pub const CARD_TITLE: &str = "text-lg font-semibold";

/// A form laid out as one wrapping row.
pub const INLINE_FORM: &str = "flex flex-wrap items-end gap-3";

/// A small secondary figure.
pub const NUM_HINT: &str = "text-xs tabular-nums text-fg-muted";

/// An emphasised amount that never wraps.
pub const AMOUNT_STRONG: &str = "whitespace-nowrap font-medium tabular-nums";

/// An emphasised amount.
pub const AMOUNT: &str = "font-medium tabular-nums";

/// A heading with an action on the far side.
pub const HEADER_ROW: &str = "flex items-center justify-between gap-3";

/// A scrolling grid of choices.
pub const CHOICE_GRID: &str =
    "my-3 grid max-h-60 grid-cols-2 gap-2.5 overflow-y-auto sm:grid-cols-3";

/// A column of blocks, top-aligned.
pub const COLUMN: &str = "grid content-start gap-6";

/// A disclosure of explanatory text.
pub const EXPLAINER: &str = "grid gap-3 text-sm leading-relaxed [&>*]:mt-3";

/// Spaced vertical stack.
pub const STACK_LG: &str = "grid gap-5";

/// A block divided from what is above.
pub const RULED: &str = "mt-3 border-t border-line pt-3";

/// A compact empty state.
pub const EMPTY_COMPACT: &str = "mt-3 p-5";

/// Facts beside a header, from small width.
pub const SIDE_FACTS: &str = "hidden w-56 sm:grid";

/// A headline figure.
pub const FIGURE_LG: &str = "text-base font-semibold tabular-nums";

/// A block divided from what is above, spaced.
pub const RULED_LOOSE: &str = "mt-4 border-t border-line pt-3";

/// A chart that has nothing to draw.
pub const CHART_EMPTY: &str = "py-10 text-center text-sm text-fg-muted";

/// A form that adds an item.
pub const ADD_FORM: &str = "my-5 grid items-end gap-3 sm:grid-cols-2 lg:grid-cols-3";

/// A line of small print with something on the far side.
pub const META_LINE: &str = "flex items-center justify-between gap-3 text-xs text-fg-muted";

/// The receipt or error under a control.
pub const FEEDBACK_LINE: &str = "mt-2 break-words text-sm";

/// Heading of a large section.
pub const SECTION_HEADING: &str = "mt-1 text-xl font-semibold";

/// Text that wraps inside a flex row.
pub const WRAP_TEXT: &str = "min-w-0 break-words";

/// Content held at a row's far end.
pub const TRAILING: &str = "shrink-0 text-right";

/// A field that grows from 10rem in a wrapping row.
pub const GROW_40: &str = "min-w-0 flex-1 basis-40";

/// Title of a queue item, cut to one line.
pub const QUEUE_TITLE: &str = "block min-w-0 truncate text-sm font-semibold";

/// Heading of the item in focus.
pub const ITEM_HEADING: &str = "break-words text-xl font-semibold";

/// Heading of a panel with a help button.
pub const PANEL_TITLE_HELP: &str = "mt-2 text-lg font-semibold flex items-center gap-2";

/// Head of a tree row: name and amount, wrapping.
pub const TREE_HEAD: &str = "flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1";

/// Name of a tree row with its icon.
pub const TREE_NAME: &str = "flex items-center gap-2 font-semibold";

/// A strip of tabs that scrolls sideways.
pub const TAB_SCROLL: &str = "mb-4 max-w-full overflow-x-auto sm:mb-5";

/// An error box.
pub const ERROR: &str =
    "rounded-lg border border-negative/30 bg-negative/10 px-4 py-3 text-sm text-negative-fg";

/// A fieldset without its browser frame.
pub const FIELDSET: &str = "m-0 min-w-0 border-0 p-0";

/// Three columns of settings.
pub const SETTINGS_GRID: &str = "my-5 grid gap-5 md:grid-cols-3";

/// The three looks of a disclosure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisclosureKind {
    /// A text toggle with the browser's marker; `summary_class` sets its type.
    #[default]
    Text,
    /// A board card that opens in place (`class` carries the card).
    Card,
    /// A muted inset box with a bold toggle.
    Panel,
}

/// A real `<details>`/`<summary>` pair, so `api::keep_details_open` keeps
/// opened ones open across re-renders.
#[component]
pub fn Disclosure(
    summary: impl IntoView + 'static,
    children: Children,
    #[prop(optional)] kind: DisclosureKind,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] summary_class: String,
) -> impl IntoView {
    let (details, head) = match kind {
        DisclosureKind::Text => ("", "cursor-pointer"),
        DisclosureKind::Card => ("", "grid cursor-pointer list-none gap-1.5"),
        DisclosureKind::Panel => (INSET, "cursor-pointer text-sm font-semibold"),
    };
    view! {
        <details class=cn([details, class.as_str()])>
            <summary class=cn([head, summary_class.as_str()])>{summary}</summary>
            {children()}
        </details>
    }
}
