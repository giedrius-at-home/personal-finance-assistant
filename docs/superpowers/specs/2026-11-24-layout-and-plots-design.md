# v0.3: foldable panels, your own tabs, and a plot builder

Date: 2026-11-24
Status: approved by the owner in chat ("it is approved, I like ti"). "My calls" stand as written.

## Intent

Owner: "i think the app can be more compactful, like there are many things
displayed on the screen in many tabs. But I dont wanna remove everything but
maybe redesign in a collapsaple way or to create on the go. and also some
plots could be created on the go if I choose, so it should be able to build
things too, add new tab, design, drag and drop if needed."

Then: "like add or remove parts as well, but it should be a bit neater …
it should be possible to create myself form the data, like a few basic ones
from selected access or data we have. and maybe prettier with more insights
plots." On plotting any number against any number: "you can go heavy". On
the release: "you can put this redesign as v3, instead of v2".

Why: every tab is a fixed stack of cards, all open, all the time. The UX
audit of 21 Nov counted about 14 screens of scrolling on Overview. The owner
wants to keep what exists, see less of it at once, and shape the app
himself, including charts the app does not ship with.

## Decided with the owner

1. Nothing is deleted from the app. Parts fold away or are removed from a
   tab, and can always be put back.
2. Parts can be added and removed, and moved by drag and drop.
3. New tabs can be made.
4. Plots can be built by the owner from the app's data, and saved.
5. The plot builder is the heavy version: free choice of what is measured,
   what it is split by, and how it is drawn.
6. A set of ready-made, better-looking "insight" plots comes with it.
7. It ships as **0.3.0** (the latest release is 0.2.20).
8. **The design style stays the same** ("but keep the same design style").
   The redesign changes how panels are arranged, not how they look.
9. **Everything is a reusable object under the style** ("everything is an
   object, no? … not just hardcoding things but putting under the style").
   Nothing new is hand-wired per page or styled in place.
10. **The existing pages are cleaned up the same way, as part of this
    work**, by a second agent, and the result is verified ("while you make
    the change, do that too, with another agent and verify"). I had
    proposed leaving it for later; the owner pulled it in.

## My calls (not yet agreed; correct any of them)

These are the choices the owner left to me. Each is a default he can reject.

1. **A tab is an ordered list of panels**, not a free canvas (approach A
   below).
2. The layout is saved **in the ledger database**, so it survives a
   reinstall and is the same in the app and in the devserver.
3. Each tab opens **compact by default**: the one to three panels that
   answer the tab's main question are open, the rest are folded to one line.
   The defaults per tab are in the panel table below.
4. A folded panel shows **its title and one short summary** (for example
   "Monthly budget: €1,480 of €1,256, €224 over"), so folding does not hide
   the headline number.
5. Panels are **full width or half width**. No free resizing.
6. Built-in tabs can be reordered and hidden, **except Manage**, which
   always stays (it holds Settings and the reset buttons). Own tabs can be
   renamed and deleted.
7. A panel that only reads the shared report can be **added to any tab**,
   and to more than one. A panel that depends on its page's own controls
   stays on its home tab, where it can still be folded, moved and removed.
8. Plots are built from **transactions**. Other data (forecasts, wishes,
   balances) is reached by adding the existing panel that already charts it.
9. Insight sentences under a plot are **computed, not written by the AI**:
   the same input always gives the same sentence.
10. Clicking a bar to open the matching transactions is **left for later**.

## Goals

- Every tab fits on about one screen when the owner opens it.
- Every panel on every tab can be folded, and the app remembers it.
- In "Edit layout", a panel can be removed, added back, dragged to a new
  place, set to half or full width, or moved to another tab.
- The owner can make, name, reorder and delete his own tabs.
- The owner can build a plot from transactions in a few choices, see it
  change as he chooses, save it, and place it on any tab like any panel.
- Ten ready-made insight plots can be added in one click and then edited.
- A plot's numbers agree with the same numbers elsewhere in the app.

## Non-goals

- A free grid with resize handles (approach B).
- Changing what is inside a panel. The disclosures inside panels stay as
  they are.
- AI-written commentary on plots.
- Plotting data that is not transactions.
- Drill-down from a plot to its transactions.
- Sharing or exporting a layout or a plot.

## Anti-goals

- **A number that disagrees.** A plot of "spent by month" that differs from
  Spending → history would undo the audit's P0 work. Plot sums use the same
  rules as Trends.
- **A lost panel.** Anything removed is listed in "Add a panel". "Reset this
  tab" and "Reset everything" restore the shipped layout.
- **A lost layout.** An app update never resets it. A panel added in a later
  version appears on its home tab, folded; a panel the update dropped is
  skipped without an error.
- **A second visual language.** A plot builder or edit mode that looks
  like a different app.
- **More clutter in normal use.** Grips, remove buttons and the tab "+" show
  only in Edit layout. Outside it, a panel gains one fold control.
- **Slow folding.** Saving the layout must not reload the report or rebuild
  the data snapshot.

## Constraints

- Same look as today. Everything new is built from the design system in
  `app/ui/src/ds` (`Card`, `Button`, `Badge`, `SegmentedControl`, `Popup`,
  `Select`, `Field`, the tab classes, `BOARD_CARD`) and the tokens in
  `app/ui/styles` (`design-tokens.css`, `glass.css`). No new colours, type
  sizes, radii or shadows. The header of a folded panel, the edit controls
  and the builder read as parts the app already has. New charts follow the
  existing charts' grid, ticks, tooltip, legend and palette.
- Reusable objects, not one-offs:
  - every new piece of UI (the panel frame, the fold header, the edit
    controls, the add-a-panel gallery, the tab editor, the builder's
    controls, each new chart) is a component in `app/ui/src/ds`, used by
    every tab, with its classes defined once there, as `TABS_TRIGGER` and
    `BOARD_CARD` are today;
  - a panel is one registry entry (data plus a render function). A page
    does not list its panels in code; it renders whatever its layout says.
    Adding a panel to the app later is adding one entry;
  - a plot is data (`PlotSpec`) drawn by one plot panel component. The
    ready-made insight plots are ten such descriptions, not ten components;
  - a tab, built-in or own, is the same object rendered by the same code.
  Existing pages are brought under the same rule; see "Style cleanup of
  existing pages".
- The UI is Leptos (Rust, WASM) with no JavaScript chart or drag library.
  Charts are the house components in `app/ui/src/ds/charts.rs` (bars, line
  and area, donut). Drag and drop already exists on the Plan, Wishes and
  Events boards (`BoardColumn` in `app/ui/src/ds/mod.rs`); the same browser
  drag events are used here. No new dependency.
- Every backend command exists twice: as a Tauri command in
  `app/src-tauri/src/main.rs` and in `app/devserver/src/main.rs`.
- Money is summed as `Decimal` on the backend. `f64` is for chart geometry
  only (`pages::f`).
- A write through `Service::write` drops the cached snapshot. Layout saves
  use `Service::write_raw`, which leaves it in place.
- New tables come in through a numbered migration in
  `crates/storage/src/lib.rs`, in the style of `wishes.sql`.
- Each merge to main ships a release. The first pull request raises
  `app/src-tauri/tauri.conf.json` and `[workspace.package]` to 0.3.0.
- Tests cover calculations and writes, not wording on screen.

## Approaches

### Layout

**A. A tab is an ordered list of panels (chosen).** Each panel has a name
in a registry. A tab is a list of panel names, each with folded or open and
half or full width. Own tabs are the same list. A saved plot is a panel like
any other. Editing is a mode: drag to reorder, × to remove, + to add from a
gallery. It stays tidy because panels can only stack and pair, and it reuses
the drag code the boards have.

**B. A free grid.** Panels are dragged and resized anywhere on a 12-column
grid, as in Grafana. Most freedom. But the panels here have natural sizes,
collision and resize logic is a lot of hand-written code without a library,
and the result is easy to make messy, which works against "a bit neater".
The useful part of B, two panels side by side, is kept in A as half width.

**C. Fold arrows only.** Every card gets a fold arrow and the state is
remembered. Small. It does not give add, remove, own tabs or plots, so it
is phase 1 of A, not an alternative.

### Plots

**1. Build from the numbers already in the report.** No backend work, but
only the monthly series the report happens to carry. No weekday, merchant
or custom period.

**2. A query over all transactions (chosen).** A new backend command takes
a plot description (measure, axis, split, filters, period) and returns the
series. It covers the "any number against any number" ask for everything
that is a transaction, and the sums are done once, in Rust, with the rules
Trends uses.

**3. A box for raw SQL.** Most power, but it bypasses the app's rules for
refunds, transfers and shared spending, so it would produce numbers that
disagree with every other screen.

### Where the layout lives

**In the ledger database (chosen)**, one row of JSON, like `wish_settings`.
The alternative is the web view's local storage, which is per origin: the
installed app and the devserver would each have their own layout, and
clearing web data or a reinstall would lose it.

## What the owner sees

### A panel

    ▾ Monthly budget                      [as today, open]
    ▸ Next 7 days             ≈ €335 expected
    ▸ Three-month outlook     €5,508 by February

One header per panel: a fold arrow, the title, and when folded the summary
on the right. An open panel does not get a second title or an extra row
above the one it has today. A "Fold all / Open all" control sits at the top
of each tab. Fold state is remembered per tab.

### Edit layout

A button at the right end of the tab bar, "Edit layout", turns to "Done".
While it is on:

- each panel shows a grip, "Half / Full", and ×;
- panels reorder by drag, or with "Move up" and "Move down" buttons for the
  keyboard;
- dropping a panel on a tab's name moves it to that tab, if it may move;
- a dashed "Add a panel" box at the bottom lists what is not on this tab:
  removed panels, panels from other tabs that may move, saved plots, the
  ready-made insight plots, and "New plot";
- the tab bar shows "+" for a new tab; tabs reorder by drag; an own tab
  shows rename and delete; a built-in tab other than Manage shows hide;
- "Reset this tab" and "Reset everything" are at the bottom, each asking
  once before it acts.

⌘1 to ⌘9 follow the order of the visible tabs.

### Building a plot

"New plot" opens the builder in a popup (the existing `Popup`), with the
plot redrawing on every change:

    Show [Spent ▾]  by [Weekday ▾]  split by [None ▾]  over [Last 6 months ▾]
    as   (Bars) (Line) (Donut) (Heatmap) (Ranked) (Scatter)
    only [+ filter]
    [ ] Running total   [ ] Compare with the period before   [ ] Average line

    ▇ ▅ ▆ ▇ █ ██ ▇
    Saturday is highest: €96 a day, 2.5× a Tuesday.

    Title [Spent by weekday]        [Show numbers]   [Save]  [Cancel]

- **Show** (the measure): Spent, Income, Net, Number of payments, Average
  payment, Largest payment. Up to three measures can be shown together when
  there is no split (for example Income and Spent).
- **by** (the axis): Day, Week, Month, Quarter, Year, Weekday, Day of the
  month, Category, Group, Merchant, Account, or Bills against everything
  else.
- **split by** (the series): none, or Category, Group, Merchant, Account,
  Weekday or Year. Splitting months by year gives a year-on-year plot. The
  five largest are drawn and the rest are summed as "Other"; the count can
  be changed.
- **over** (the period): this month, last month, the last 3, 6 or 12
  months, this year, all time, or two dates.
- **only** (filters): categories, groups, accounts, merchant text, smallest
  and largest amount.
- **as** (the drawing): the builder offers only the drawings that fit the
  choices. Donut and Ranked need a named axis (Category, Merchant…) and no
  split. Heatmap needs two axes. Scatter needs two measures.
- **Show numbers** swaps the drawing for the table behind it.

A saved plot is a panel. Its header carries a small "Edit plot" control
that opens the same builder. Its folded summary is its insight sentence.
Deleting a plot from the gallery removes it from every tab, after asking.

### Ready-made insight plots

Each is a saved description the owner can add and then edit:

| Name | What it draws |
|---|---|
| Spending calendar | Heatmap of spend per day, weeks down, weekdays across, last 3 months |
| Weekday pattern | Average spend per weekday, last 6 months |
| This month against usual | Running total by day, this month against the average of the 3 before |
| Top merchants | Ranked, last 3 months |
| Where it went | Groups stacked per month, last 12 months |
| Income and spending | Income and Spent per month with Net as a line, last 12 months |
| Year on year | Spent per month, one line per year |
| Small payments | Payments under €10, counted per merchant, ranked |
| Bills against the rest | Stacked per month, last 12 months |
| Biggest changes | Change per category against the period before, ranked up and down |

### The insight sentence

One sentence under each plot, picked by the plot's shape:

- over time: the last full bucket against the one before, and against the
  average ("October: €1,420, 19% above your 6-month average");
- named axis: the largest and its share ("Jumbo is 31% of groceries");
- weekday or day of month: the peak and its ratio to the lowest;
- with "compare with the period before": the difference.

The current, unfinished bucket (this month, this week) is drawn lighter and
is never called highest or lowest.

## How it is built

### Panel registry (UI)

A registry lists every panel: a stable id (for example `overview.budget`),
title, home tab, whether it may move to other tabs, its default state on
its home tab (open, folded, or not shown), default width, a function that
renders it, and an optional function from the report to its folded summary.
A page renders its layout's list through one `Panel` wrapper that supplies
the header, the fold, and the edit controls.

Spending and Manage keep their sub-section switch. Each sub-section is its
own page in the layout (`spending.habits`, `manage.banks`, …).

Components that render several cards today are split so that one card is
one panel, where the cards do not share local state.

### Saved layout (types, storage, commands)

    UiLayout {
      tabs:  [ { id, name, built_in, hidden, pages: [ PageLayout ] } ],
      plots: [ { id, title, spec: PlotSpec } ],
    }
    PageLayout { id, items: [ { panel, folded, half } ], removed: [ panel ] }

- `removed` is what tells "the owner removed this" from "this panel is new
  in this version". A registry panel that is in neither list is new, and is
  placed at its default position, folded.
- A plot panel's id is `plot:<plot id>`.
- Table `ui_layout (singleton, payload_json)`, added by a new migration.
- Commands `ui_layout` (read; an empty layout when none is saved) and
  `save_ui_layout` (write through `write_raw`, no snapshot reload, no
  revision bump). Both in Tauri and in the devserver.
- Validation on save: names 1 to 40 characters, ids unique, Manage neither
  hidden nor missing, at most 50 tabs and 200 plots, payload under 256 KB.
  An invalid save is refused and the UI keeps the last good layout.
- The UI loads the layout once at start, keeps it in a signal, and saves
  after each change. Saves are sent in order; a failed save shows one line
  of error in Edit layout and does not undo what is on screen.

### Plot query (types, core, commands)

    PlotSpec { measures, by, split, top, period, filters,
               chart, cumulative, compare_previous, average_line, currency }
    PlotData { categories, series: [ { label, values } ], previous,
               partial_last, insight, currency }

- Command `plot_query(spec) -> PlotData`, a read, in Tauri and devserver.
- It reads `Snapshot::transactions` (after the shared-spending rewrites),
  as Trends does, and uses the same helpers for what counts as spending
  (`groups::net`, `forecast::signed`): transfers between own accounts are
  out, refunds reduce spending.
- One currency per plot: the main currency unless the owner picks another.
  The picker shows only when there is more than one.
- Dates use the transaction's month date (`Tx::effective_on`), as budgets
  do. Weeks start on Monday.
- The insight sentence is built in the backend from the same sums, so it is
  tested with them.
- An impossible description (Donut with a split, Scatter with one measure)
  is refused with a short error; the builder does not offer it.

### Charts (UI)

Bars, line, area and donut exist. New in `ds/charts.rs`, in the same style
(percent-based SVG, the `--ds-chart-*` palette, hover tooltip, legend):
Heatmap, Ranked (horizontal bars, also two-sided for changes) and Scatter.
The lighter drawing of an unfinished bucket is added to bars and line.
Each split value keeps one colour across a plot; groups use the fixed group
colours where the app already has them.

## Style cleanup of existing pages

Today the pages use the design system's components but also type a lot of
styling in place: the audit counted 118 uses of `text-xs` and one-off sizes
such as `text-[11px]`, `text-[10px]`, `text-[0.65rem]`, and the same label,
row, field-grid and disclosure markup is repeated across pages.

The cleanup:

- a class string or a piece of markup that appears in more than one place
  becomes one named constant or component in `app/ui/src/ds`, and the pages
  use it;
- disclosures (`<details>` with three different looks today) use one
  component;
- one-off sizes and opacities are replaced by the nearest existing token;
- what is used once and is already built from tokens stays where it is.

Rule: **no visible change**, except where a one-off size snaps to the
nearest token (a pixel or so). No copy changes, no layout changes, no
behaviour changes. (The snapping is my call; the alternative is to keep
every odd size under a new name, which tidies nothing.)

It is done by a second agent on its own branch, as its own pull request,
while the backend and the new design-system parts of this work are built,
because those do not touch the page files. The panel work on the pages
starts from the cleaned-up pages, so two writers never edit a page at once.

Verified by: the build, clippy and the test suite; and a screenshot of
every tab and sub-section, at a wide and a narrow window, before and after,
on a copy of the ledger in the devserver, compared side by side. Any
difference that is not a snapped size is a defect to fix before merging.
The comparison is done by someone other than the agent that made the
change.

## Phases

Each phase is one pull request and ships on merge.

0. **Cleanup.** The style cleanup above, by the second agent, in parallel
   with the backend half of phases 1 and 3 (saved layout table and
   commands, `plot_query`) and the new `ds` components.
1. **Fold (0.3.0).** Version raise, panel registry, `Panel` wrapper, split
   of multi-card components, folded summaries, compact defaults, Fold all,
   saved layout table and commands.
2. **Arrange.** Edit layout: remove, add, reorder, half width, move between
   tabs, own tabs, hide and reorder tabs, the two resets.
3. **Plots.** `plot_query`, the builder, the three new chart kinds, saved
   plots as panels.
4. **Insights.** The ten ready-made plots, insight sentences, running
   total, compare with the period before, average line.

## Tests

- Layout: a saved layout reads back the same; an unknown panel id is
  skipped; a new registry panel lands folded on its home tab; a removed one
  stays removed; an invalid layout is refused; saving does not drop the
  snapshot.
- Plot query: "Spent by month, last 6 months" equals `time_trends` month by
  month; "Spent by weekday" equals its weekday totals; refunds and own
  transfers are handled as in Trends; top-N plus Other sums to the total;
  the running total's last point equals the plain total; the compared
  period has the same number of buckets; an impossible description is
  refused.
- Insight: the unfinished bucket is never the peak; the sentence for each
  shape on a small fixed ledger.
- No tests on wording or layout of the screens.

## Panels per tab

From an inventory of the current screens. "First run" is the proposed
default (my call 3). "Moves" says whether the panel may be added to other
tabs: yes when it reads only the shared report or loads its own data, no
when it needs its page's state.

Not panels, and always shown where they are today: the month-closure
banner on Overview, popups and the payment picker, error and notice lines,
the sub-section switch of Spending and Manage, and the "Review shared
spending" link on Habits & goals.

A panel that has nothing to show (no data for its condition) draws nothing
outside Edit layout, as today, and a muted one-line placeholder inside it.
A panel that repeats per currency is one panel.

### Overview

| Panel | From | First run | Moves |
|---|---|---|---|
| This month at a glance (KPI strip) | `kpis::OverviewKpis` | Open | yes |
| Do this now | `nudges::OverviewNudges` | Open | yes |
| Spendable and From salary | `monthly::CurrencyOverview` | Folded | yes |
| Where this month is heading | `monthly.rs:592` | Folded | yes |
| Budget table | rows in `budget::MonthlyBudgetPanel` | Open | no |
| The next seven days | `forecast::DailyForecastPanel` | Folded | yes |
| Spending trajectory, Are you on track | `visuals::BudgetVisuals` | Folded | no |
| This month only (extra income and payments) | `extras::BudgetExtras` | Folded | no |

`MonthlyBudgetPanel` is one section today whose four blocks share a local
`PanelState`. It is split into the four panels above; the state moves to
the page (see "Page state" below).

### Transactions

| Panel | From | First run | Moves |
|---|---|---|---|
| All transactions | `search::TransactionSearch` | Open | yes |
| Shared spending and money back | `personal::PersonalSpending` | Folded | yes |
| Transaction inbox | `review::OwnerReview` | Open | yes |

### Spending › Habits & goals

| Panel | From | First run | Moves |
|---|---|---|---|
| Three-month outlook | `outlook::SavingsOutlookPanel` | Open | yes |
| Goals | `goals::GoalsCard` | Folded | yes |
| Adjustments | `adjustments::AdjustmentsCard` | Open | yes |
| Spending by weekday (with its "Variable patterns" switch) | `spending::SpendingAnalysis` | Folded | yes |

### Spending › Organise categories

| Panel | From | First run | Moves |
|---|---|---|---|
| Category coverage | `organise::SpendingOrganization` | Open | yes |
| Purchases paid together later | `organise::BulkPayments` | Folded | yes |
| Organise spending | `organise::OwnerCategoryGroups` | Open | yes |

### Spending › All spending history

| Panel | From | First run | Moves |
|---|---|---|---|
| Monthly spending | `spending.rs:197` | Open | yes |
| Compare categories | `category_trends::CategoryTrends` | Open | yes |

### Plan

| Panel | From | First run | Moves |
|---|---|---|---|
| Salary and Where the money goes | `plan.rs:435` | Open | yes |
| Payments board (with its month switcher) | `plan.rs:597` | Open | no |
| Add a payment or income | `plan.rs:619` | Folded | no |
| Bills | `plan.rs:625` | Folded | no |

### Wishes

| Panel | From | First run | Moves |
|---|---|---|---|
| Wishes board | `wishes.rs:1109` | Open | no |
| Add a wish | `wishes.rs:1127` | Folded | no |
| Bought and dropped | `wishes.rs:1133` | Folded | no |
| Month by month | `wishes.rs:1141` | Folded | no |
| Credit cards | `wishes::card_strip` | Folded | no |
| Emergency fund | `wishes::fund_block` | Folded | no |

### Events

| Panel | From | First run | Moves |
|---|---|---|---|
| Events board | `events.rs:291` | Open | no |
| New event | `events.rs:300` | Folded | no |

### Manage

| Section | Panel | From | First run | Moves |
|---|---|---|---|---|
| Banks | Bank connections | `banks.rs:252` | Open | no |
| Banks | Connect or renew | `banks::ConsentCard` | Folded | no |
| Banks | Application key | `banks::KeyCard` | Folded | no |
| Close month | Close the month (header and the open step) | `closure.rs:844`, `step_view` | Open | no |
| Close month | Where the money goes | `closure::plan::MoneyPlanPanel` | Open | no |
| Close month | Accounts | `closure::panels::AccountsPanel` | Folded | no |
| Import | Import transactions | `import::ImportCard` | Open | no |
| Status | Connections | `status::Connections` | Open | no |
| Status | Category review | `status::CategoryReview` | Open, half | no |
| Status | Account data health | `status.rs:37` | Open, half | yes |
| Settings | Finance preferences | `settings::SettingsCard` | Open | no |
| Settings | Daily spending email | `email::EmailCard` | Folded | no |
| Settings | AI suggestions | `ai::AiSuggestions` | Folded | no |
| Settings | Local data | `settings::LocalData` | Folded | no |
| Settings | Forecast model | `settings::ForecastModel` | Folded | no |
| Settings | Updates | `settings::Updates` | Folded | no |

Manage panels do not move to other tabs even where they could: settings
have one home (the audit's finding that settings were spread over pages).

### Page state

Plan, Wishes, Events, Banks, Close month and the Overview budget each have
a state object their panels share (`plan::Ui`, `wishes::Ui`, `events::Own`,
`manage::ui::Ops`, `closure::Wizard`, `budget::PanelState`). Today most of
them are passed down as function arguments from one page function. For
panels to be separate registry entries, the page creates its state once
and provides it by context, as Close month already does, and each panel
reads it. A page's popups stay rendered by the page, next to its panel
list.

### The panel list

Pages lay out their children in different ways today (`.page-stack` with a
1.5rem gap; Plan, Events, Wishes and Manage inside their own `grid gap-4`;
leftover outer margins on some cards). One list component in `ds` replaces
these: a two-column grid on wide windows where a full panel spans both
columns and a half panel takes one, and a single column on narrow ones.

## Left to the builder

- How an open panel gets its fold control without a second title: either
  the `Panel` wrapper takes over each card's header, or it overlays the
  control on the header that is there. One header per panel is the rule.
- The exact module layout of the registry and how a panel's render function
  reaches page-level state.
- The summary text of each built-in panel, taken from numbers the panel
  already shows.
- How saves are queued so that two quick changes arrive in order.
- The look of the heatmap scale and the scatter points within the tokens.
