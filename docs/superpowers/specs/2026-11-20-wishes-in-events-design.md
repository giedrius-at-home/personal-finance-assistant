# Wishes in events: move a stored wish into an event

Date: 2026-11-20
Status: approved by the owner in chat ("Yes, build it").

## Intent

Owner: "some wishes can be part of the events, I dont think we have it
there". A wish can become a whole event ("Plan as event") and a part can
be typed in on Events, but a wish that already exists cannot be put under
an event that already exists. Today that means dropping it and typing it
again, losing its saved money.

## Goals

- A write `move_wish(wish_id, parent_id | none)` that sets or clears the
  wish's parent and keeps everything else (amount, date, source, saved
  money).
- Rules: the wish is one-off, still planned, has no parts of its own and
  is not itself an event; the target is a planned event (a one-off wish
  with event details, not a part) in the same currency that has not
  ended. The must flag is dropped when the event is not a must.
- Moving out (`none`) is allowed for a planned part of an event.
- Wishes row: "Add to event" with a picker of the running and upcoming
  events. Events part row: "Back to wishes".
- The "Part of" picker of the new-wish form lists events too.

## Added by the owner while it was built

- "we also have done wishes, so it should be able to select from those
  too": a bought wish can be put into an event, where it counts as spent
  (a bought part). It may also go into an event that is over; a planned
  wish may not.
- "the done wishes should have the same design as normal wishes, not a
  string and also auto load with the scroll, first 10 wishes": a done wish
  is a name, a chip (Bought, Started, Dropped, Not bought) and fields.
  The Done list shows ten and loads ten more when its end scrolls into
  view; a "Show more" button does the same (designer's call, as a
  fallback).

## Non-goals

- Moving a wish under a plain parent wish, or a wish with parts into an
  event (that one uses "Plan as event").
- Raising the event's planned amount: a part sits inside it, as parts do
  today (designer's call, shown to the owner before approval).

## Anti-goals

- A wish counted twice (once on Wishes, once in the event).

## Constraints

- Every command needs Tauri, devserver and a permission file.
- Tests cover the write and its rules, not wording.

## Approaches

- **Re-parent the stored wish** (chosen): a part of an event is already a
  sub-wish, so only the move is missing.
- A tag linking a wish to an event without a parent: the ladder would
  judge the wish and the event's total both. Dropped.
- A separate "event item" object: a second store, against the owner's
  "the object can be the same". Dropped.
