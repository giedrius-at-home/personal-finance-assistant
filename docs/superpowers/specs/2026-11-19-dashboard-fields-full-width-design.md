# Dashboard fields and a full-width layout

## Why

The owner's feedback: the app spoke in small grey sentences ("At this pace you
finish €555.22 over plan."), and every section was a card floating in a narrow
centred column. He wants a normal dashboard: labelled fields with numbers, and
a design that uses the whole window.

## Goals

- A number that mattered inside a sentence is a field: label, value, an
  optional status chip, and labelled facts under it.
- Loose sentences are kept to a minimum everywhere on Overview and on the
  Goals and Adjustments sections of Spending.
- The page fills the window. Sections sit on the page, split by a rule, not in
  boxes.
- Charts take the width of their section, with their key numbers as fields
  beside them.

## Non-goals

- No change to any calculation, report type or write.
- The wish verdict stays one sentence: it carries the reasoning ("€566 on the
  cards, so December starts €566 lighter") that a field cannot. A status chip
  and fields for amount, date and source sit beside it.
- Alerts, save confirmations, empty states and the month-closure wizard's
  instructions stay as text.

## Approaches

1. **Restyle the sentences** (bigger, bolder). Cheapest, but still prose; the
   owner asked for fields.
2. **Fields and a flat, full-width shell** (picked). One `Metric` component
   with `Facts` rows replaces the sentences; `Card` loses its box; the shell
   loses its width cap.
3. **A fixed 12-column widget grid** with draggable tiles. Much more work and
   the sections differ too much in shape to share one tile size.

## Design

- `ds::Metric`: eyebrow label, value, optional `(Tone, text)` chip, optional
  `Facts`. `ds::Facts`: small label/value rows. `ds::METRIC_STRIP`: fields in a
  row with a rule between them.
- `card_class(Default)` is `flat-section` (a top rule and padding);
  `Muted` stays a quiet panel for callouts inside a section.
- The shell drops `max-w-6xl`. In the app window the page tint is thicker
  (62 %) because text now sits on the page, not on a card.
- Overview opens with a strip: Spendable now, Left in plan, Month end vs plan,
  In savings, Next 7 days, Behavior score. The salary-at-risk banner became the
  chip and fact of "Spendable now".
- Outlook and daily forecast: fields in a rail on the left, the chart beside
  it, "Coming up" as a third column on the forecast.
- On-track and trajectory: verdict as fields; "vs budget" is a fourth stat.
- Goals: need, what habits leave and the gap as fields; goals and cuts as two
  tables with status chips.
- Adjustments: totals as fields; every group in one table (budget, typical,
  heading to, above norm, status chip), the drivers as a small table.
- Explanations of method that are not numbers moved to `title` tooltips.

- Plan, Wishes, Transactions, Spending history and Manage: sentences that
  carried data are `Facts`/`FactsRow` fields or chips.
- `ds::Help`: an info mark with the explanation on hover. `Field` hints and
  static explanations use it, so no helper sentence sits under a control.

## Left to the builder

The closure wizard's step instructions and the bank messages that arrive as
text from the sync.
