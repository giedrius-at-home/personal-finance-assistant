# v0.3 part 1: style cleanup and foldable panels: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the existing pages' styling under the design system with no visible change, then make every panel on every tab a registered, foldable object whose state is saved, and ship that as 0.3.0.

**Architecture:** A saved `UiLayout` (JSON, one row in the ledger database) says which panels each page shows and whether each is folded. The UI holds a registry of panel definitions; each page renders its resolved list through one `PanelFrame` from the design system. Page-level state moves from function arguments to context so panels can be separate registry entries.

**Tech Stack:** Rust; Leptos 0.8 CSR (WASM, Trunk, Tailwind 4); Tauri commands mirrored in `finance-devserver`; SQLite through `finance-storage`.

**Spec:** `docs/superpowers/specs/2026-11-24-layout-and-plots-design.md`. This plan covers its phases 0 and 1. Phases 2 (Arrange), 3 (Plots) and 4 (Insights) each get their own plan once the phase before has merged, because they build on the interfaces this one creates.

## Global Constraints

- Same look as today: only parts from `app/ui/src/ds` and tokens from `app/ui/styles`. No new colours, type sizes, radii or shadows.
- Every new piece of UI is a component or named constant in `app/ui/src/ds`. A page does not list its panels in code; it renders what its layout says.
- No new dependency.
- Every backend command is added in both `app/src-tauri/src/main.rs` (and its `generate_handler!` list) and `app/devserver/src/main.rs`.
- Saving the layout must not drop the cached snapshot, bump `AppState::revision`, or cause a ledger backup.
- Reading the layout when the `ui_layout` table does not exist gives the default layout, not an error.
- Tests cover calculations and writes, not wording or layout of screens.
- `cargo` is not on PATH: start each build shell with `export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"`.
- The checks CI runs, and every task must leave green:
  `cargo fmt --all --check`;
  `cargo clippy --locked -p finance-types -p finance-storage -p finance-core -p finance-migrate -p finance-secrets -p finance-bank -p finance-notify -p finance-timesfm -p finance-ai -p finance-devserver --all-targets -- -D warnings`;
  `cargo clippy --locked -p finance-ui --target wasm32-unknown-unknown -- -D warnings`;
  `cargo test --locked -p finance-storage -p finance-types -p finance-core -p finance-bank -p finance-notify -p finance-timesfm -p finance-ai`;
  `cargo test --locked -p finance-ui --bins`.
- Never run the devserver without `DEV_DB`: without it, it writes the real ledger.
- Commit messages end with `Co-Authored-By: Claude Fable 5.2 <noreply@anthropic.com>`.

## Review Focus

1. **A stored layout the app cannot read** (hand-edited, or a shape from a later version): the app starts with the default layout and the next save replaces the row. Test in Task 2.
2. **Two fold clicks in quick succession**: both end up saved, the later state wins, and no save is lost while another is in flight. Test in Task 4 (`SaveQueue`).
3. **Typing in a form, folding its panel, opening it again**: the text is still there. A panel's body is mounted on first open and then kept, hidden, when folded. Manual check in Task 11.
4. **No data yet, or a save that fails**: folding still works on screen for the session; nothing is shown as an error outside Edit layout (which does not exist yet), and the app does not panic. Test in Task 2 (`no-data`), manual check in Task 11.
5. **A fold-only session followed by a real edit**: the ledger backup still happens, exactly once, before the first ledger write. Test in Task 2.

---

## Work order

- Task 0 first (baseline), by the main session.
- Then in parallel: **Task C** (cleanup, second agent, own worktree and branch from `main`) and **Tasks 1 to 5** (main session, this branch). They share no page files; both add one `mod` line to `app/ui/src/ds/mod.rs`.
- Task C is verified and merged (Task C-verify) by the main session, not by the agent that made the change. This branch is then rebased onto `main`.
- Tasks 6 to 11 follow, on the cleaned-up pages.

---

### Task 0: Baseline of every screen

**Files:**
- Create (scratchpad, not in the repo): `fingerprint.js`, `baseline/<page>-<width>.json`, `baseline/<page>-<width>.png`

- [ ] **Step 1: Build `main` and start it on a copy of the ledger**

```bash
cp "$HOME/Library/Application Support/Finance/finance.sqlite3" "$SCRATCH/ledger-copy.sqlite3"
(cd app/ui && trunk build)
DEV_DB="$SCRATCH/ledger-copy.sqlite3" cargo run --release -p finance-devserver -- app/ui/dist 1430
```

- [ ] **Step 2: Save `fingerprint.js`**

It lists every rendered element with its box and the styles a cleanup could change:

```js
JSON.stringify([...document.querySelectorAll('body *')].filter(e => e.getClientRects().length).map(e => {
  const r = e.getBoundingClientRect(), c = getComputedStyle(e);
  return [e.tagName, Math.round(r.x), Math.round(r.y + document.querySelector('.app-surface').scrollTop),
    Math.round(r.width), Math.round(r.height), c.fontSize, c.fontWeight, c.lineHeight, c.color,
    c.backgroundColor, c.opacity, c.borderTopWidth, c.borderRadius, c.letterSpacing, c.textTransform];
}))
```

- [ ] **Step 3: Capture the 13 pages at two widths**

Pages: Overview, Transactions, Spending (Habits & goals, Organise categories, All spending history), Plan, Wishes, Events, Manage (Banks, Close month, Import, Status, Settings). Widths: 1440 and 900. For each: open `http://127.0.0.1:1430`, go to the page, wait until no skeleton is shown, run `fingerprint.js`, save the result; take one screenshot of the top of the page.

Expected: 26 JSON files and 26 screenshots in `baseline/`. Keep `ledger-copy.sqlite3` unchanged for Task C-verify and Task 11 (make a second copy for anything that writes).

---

### Task C: Style cleanup of the existing pages (second agent)

**Files:**
- Create: `app/ui/src/ds/patterns.rs` (new shared class constants and small components)
- Modify: `app/ui/src/ds/mod.rs` (one line: `pub mod patterns;`, plus re-exports), every file under `app/ui/src/pages/`, `app/ui/src/main.rs`
- Branch: `chore/ui-style-under-ds`, from `main`, in its own worktree

**Interfaces:**
- Produces: constants and components in `ds::patterns`; no change to any existing `ds` signature.

Rules, from the spec's "Style cleanup of existing pages":

1. A class string or a piece of markup that appears in two or more places becomes one named constant or component in `ds::patterns`; the pages use it.
2. Every `<details>` disclosure uses one component, `Disclosure { summary, class, children }`, rendering a real `<details>`/`<summary>` so `api::keep_details_open` keeps working. Where today's three looks differ, the component takes a `kind` so each keeps its look.
3. One-off sizes (`text-[11px]`, `text-[10px]`, `text-[0.65rem]`, `text-[0.68rem]` and the like) and one-off opacities on text become the nearest existing token.
4. What is used once and is already built from tokens stays.
5. No visible change other than rule 3. No copy, layout or behaviour change. No change to any calculation.

- [ ] **Step 1: List the repeats.** For every `class="…"` literal and `cn([...])` first element under `app/ui/src/pages` and `main.rs`, count identical strings; list those with a count of two or more, and every arbitrary-value class (`[` … `]`). Commit the list as the body of the first commit message, not as a file.
- [ ] **Step 2: Add `ds/patterns.rs`** with one constant or component per repeat, each with a one-line doc comment saying what it is for, named for its role (`ROW_LABEL`, `AMOUNT`, `FIELD_GRID`), not its looks.
- [ ] **Step 3: Replace, one page module per commit** (`overview`, `transactions`, `spending` with `organise`/`goals`/`adjustments`/`category_trends`, `plan`, `wishes`, `events`, `closure`, `manage`, `main.rs`). After each: the two clippy commands and `cargo test --locked -p finance-ui --bins`.
- [ ] **Step 4: Report** the list of rule-3 snaps (file, old class, new class), so the verifier knows which differences are expected. Do not open the pull request.

---

### Task C-verify: Check the cleanup and merge it (main session)

- [ ] **Step 1:** In the cleanup worktree, run all five CI commands. Expected: all pass.
- [ ] **Step 2:** `trunk build` there, start the devserver on the same `ledger-copy.sqlite3`, port 1431, and capture the 26 fingerprints as in Task 0 into `after-cleanup/`.
- [ ] **Step 3:** Diff each pair. Expected: the element count and order are equal, and every differing row differs only in `fontSize`, `lineHeight`, `opacity` or `color`-from-opacity on an element the agent's snap list names, and by at most what the snap implies. Any other difference is a defect: send it back to the agent with the page, width and row.
- [ ] **Step 4:** Look at the 26 screenshot pairs side by side for anything the fingerprint cannot see (clipping, overflow).
- [ ] **Step 5:** Open the pull request `chore(ui): put page styling under the design system`, wait for CI, squash-merge. Rebase this branch onto `main`.

---

### Task 1: `UiLayout` types

**Files:**
- Create: `crates/types/src/ui.rs`
- Modify: `crates/types/src/lib.rs` (`pub mod ui;`)

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiLayout { #[serde(default)] pub tabs: Vec<TabLayout> }
pub struct TabLayout { pub id: String, pub name: String, pub built_in: bool,
                       #[serde(default)] pub hidden: bool, #[serde(default)] pub pages: Vec<PageLayout> }
pub struct PageLayout { pub id: String, #[serde(default)] pub items: Vec<PanelItem>,
                        #[serde(default)] pub removed: Vec<String> }
pub struct PanelItem { pub panel: String, #[serde(default)] pub folded: bool, #[serde(default)] pub half: bool }

impl UiLayout {
    pub fn page(&self, page_id: &str) -> Option<&PageLayout>;
    /// The page's entry, created (with its tab) when missing. `tab_name` is used only on creation.
    pub fn page_mut(&mut self, tab_id: &str, tab_name: &str, page_id: &str) -> &mut PageLayout;
    /// `Err` with a short reason when the layout breaks a rule below.
    pub fn validate(&self) -> Result<(), String>;
}
pub const MAX_TABS: usize = 50;
pub const MAX_NAME: usize = 40;
```

All four structs derive the same traits as `UiLayout` (`TabLayout` and the two below without `Default` where a field has none). Page ids: `overview`, `transactions`, `spending.habits`, `spending.organise`, `spending.history`, `plan`, `wishes`, `events`, `manage.banks`, `manage.close`, `manage.import`, `manage.status`, `manage.settings`. Tab ids: `overview`, `transactions`, `spending`, `plan`, `wishes`, `events`, `manage`.

- [ ] **Step 1: Write the failing tests** in `ui.rs`:

```rust
#[test] fn empty_json_is_the_default() { assert_eq!(serde_json::from_str::<UiLayout>("{}").unwrap(), UiLayout::default()); }
#[test] fn page_mut_creates_tab_and_page_once() {
    let mut l = UiLayout::default();
    l.page_mut("spending", "Spending", "spending.habits").items.push(PanelItem { panel: "spending.habits.outlook".into(), folded: true, half: false });
    l.page_mut("spending", "Spending", "spending.history");
    assert_eq!(l.tabs.len(), 1);
    assert_eq!(l.tabs[0].pages.len(), 2);
    assert!(l.tabs[0].built_in);
    assert!(l.page("spending.habits").unwrap().items[0].folded);
    assert!(l.page("nope").is_none());
}
#[test] fn validate_rules() {
    let tab = |id: &str, name: &str, hidden| TabLayout { id: id.into(), name: name.into(), built_in: true, hidden, pages: vec![] };
    assert!(UiLayout { tabs: vec![tab("plan", "Plan", true)] }.validate().is_ok());
    assert!(UiLayout { tabs: vec![tab("manage", "Manage", true)] }.validate().is_err());      // Manage may not be hidden
    assert!(UiLayout { tabs: vec![tab("plan", "", false)] }.validate().is_err());             // name 1..=40 chars
    assert!(UiLayout { tabs: vec![tab("plan", &"x".repeat(41), false)] }.validate().is_err());
    assert!(UiLayout { tabs: vec![tab("plan", "Plan", false), tab("plan", "Plan 2", false)] }.validate().is_err()); // unique tab ids
    assert!(UiLayout { tabs: (0..51).map(|i| tab(&format!("t{i}"), "T", false)).collect() }.validate().is_err());
}
#[test] fn validate_rejects_a_panel_listed_twice_on_a_page() { /* same `panel` twice in one page's items → Err; the same panel in items and removed → Err */ }
```

- [ ] **Step 2:** `cargo test -p finance-types ui::` → FAIL (module missing).
- [ ] **Step 3:** Implement. `page_mut` sets `built_in: true` on a tab it creates (own tabs are created by phase 2's code, not here).
- [ ] **Step 4:** `cargo test -p finance-types ui::` → PASS.
- [ ] **Step 5:** Commit `feat(types): saved UI layout`.

---

### Task 2: Store and service

**Files:**
- Create: `crates/storage/src/ui_layout.sql`, `crates/core/src/ui_layout.rs`
- Modify: `crates/storage/src/lib.rs` (append `Migration::Sql(include_str!("ui_layout.sql"))`; the test at line 321 expects version 7), `crates/core/src/lib.rs` (`pub mod ui_layout;`), `crates/core/src/service.rs`

**Interfaces:**
- Consumes: `finance_types::ui::UiLayout` (Task 1).
- Produces:

```rust
// crates/core/src/ui_layout.rs
pub fn load(conn: &Connection) -> Result<UiLayout>;               // default when the table or the row is missing, or the payload does not parse
pub fn save(w: &Writer<'_>, layout: &UiLayout) -> Result<()>;      // validate(), payload ≤ 256 * 1024 bytes, upsert singleton row
// crates/core/src/service.rs
impl Service {
    pub fn ui_layout(&self) -> Result<UiLayout, String>;
    pub fn save_ui_layout(&self, layout: UiLayout) -> Result<UiLayout, String>;
}
```

SQL: `CREATE TABLE IF NOT EXISTS ui_layout (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), payload_json TEXT NOT NULL);`

`save_ui_layout` goes through a new private write path: like `write_raw` (no snapshot, cached snapshot kept) but without the backup. Today the backup runs when the writer store is first opened; change that to a flag on the opened writer so that the backup runs before the first write through `write`, `write_if_changed` or `write_raw`, whichever comes first, even when a layout save opened the store earlier.

- [ ] **Step 1: Write the failing tests** (in `ui_layout.rs` and next to the existing service tests, using their temp-database helper):

```rust
#[test] fn load_without_the_table_is_default()        // read-only store that never migrated past 6
#[test] fn load_of_garbage_payload_is_default()       // payload_json = "not json" and = "[1,2]"
#[test] fn save_then_load_round_trips()
#[test] fn save_refuses_hidden_manage_and_keeps_the_old_row()
#[test] fn save_refuses_a_payload_over_256_kib()
#[test] fn saving_the_layout_keeps_the_cached_snapshot() // dashboard(); save_ui_layout(); assert the snapshot is still cached
#[test] fn saving_the_layout_makes_no_backup()           // backups dir empty after save_ui_layout()
#[test] fn the_first_ledger_write_after_a_layout_save_still_backs_up() // save_ui_layout(); then any `write`; exactly one file in backups
#[test] fn no_database_gives_no_data()                   // ui_layout() and save_ui_layout() return Err("no-data"), no panic
```

- [ ] **Step 2:** `cargo test -p finance-core ui_layout` and `-p finance-storage` → FAIL.
- [ ] **Step 3:** Implement. Reuse the upsert helper on `Writer` (the one `save_wish_settings` uses) and the `table_exists` pattern from `wishes/store.rs`.
- [ ] **Step 4:** Same commands → PASS; the storage version test asserts 7.
- [ ] **Step 5:** Commit `feat(core): store the UI layout without touching the snapshot or the backups`.

---

### Task 3: Commands

**Files:**
- Modify: `app/src-tauri/src/main.rs`, `app/devserver/src/main.rs`

**Interfaces:**
- Produces: command `ui_layout` (no arguments) → `UiLayout`; command `save_ui_layout` with `{ "layout": UiLayout }` → `UiLayout`. Same names and JSON in both hosts, written in the style of `save_wish_settings`.

- [ ] **Step 1:** Add both commands to Tauri (and to `generate_handler!`) and to the devserver's match, with a `UiLayoutArgs { layout }` struct there.
- [ ] **Step 2:** Verify against a copy: start the devserver with `DEV_DB` on a second copy of the ledger, then

```bash
curl -s -X POST localhost:1430/invoke/ui_layout -d '{}'
curl -s -X POST localhost:1430/invoke/save_ui_layout -d '{"layout":{"tabs":[{"id":"plan","name":"Plan","built_in":true,"pages":[{"id":"plan","items":[{"panel":"plan.bills","folded":true}]}]}]}}'
curl -s -X POST localhost:1430/invoke/ui_layout -d '{}'
```

Expected: `{"tabs":[]}`, then the saved layout echoed twice.
- [ ] **Step 3:** The native clippy command and `cargo check -p finance-app` → clean. Commit `feat(app): ui_layout commands`.

---

### Task 4: Layout logic in the UI (no rendering)

**Files:**
- Create: `app/ui/src/layout.rs`
- Modify: `app/ui/src/main.rs` (`mod layout;`)

**Interfaces:**
- Consumes: `finance_types::ui::*`, `finance_types::report::Dashboard`.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Initial { Open, Folded, Absent }           // on its home page, first run

pub struct PanelDef {
    pub id: &'static str,                            // "overview.budget"
    pub title: &'static str,
    pub page: &'static str,                          // home page id
    pub movable: bool,                               // may be added to other tabs (phase 2 uses it)
    pub initial: Initial,
    pub half: bool,
    pub render: fn() -> AnyView,
    pub summary: Option<fn(&Dashboard) -> Option<String>>,
}

/// The panels a page shows, in order. `defs` are the registry entries whose home is this page, in registry order.
pub fn resolve(defs: &[&PanelDef], saved: Option<&PageLayout>) -> Vec<PanelItem>;

/// Coalesces saves: at most one in flight, the latest state sent after it.
#[derive(Default)] pub struct SaveQueue { in_flight: bool, dirty: bool }
impl SaveQueue {
    pub fn changed(&mut self) -> bool;               // true: start a save now
    pub fn finished(&mut self) -> bool;              // true: start another save with the current state
}
```

`resolve` rules: with no saved page, every def that is not `Absent`, folded when `Initial::Folded`, `half` from the def. With a saved page: saved items in saved order, dropping ids no def (and, later, no plot) knows; then each def that is in neither `items` nor `removed` is inserted, folded, right after the nearest earlier def (in registry order) that is present, or at the start when none is.

- [ ] **Step 1: Write the failing tests** (native, in `layout.rs`), with three test defs `a` (Open), `b` (Folded), `c` (Absent) and a no-op `render`:

```rust
#[test] fn first_run_uses_the_defaults()            // [a open, b folded]; c not listed
#[test] fn saved_order_and_fold_state_win()         // saved [b open, a folded] → the same
#[test] fn an_unknown_saved_panel_is_skipped()      // saved [a, "gone.panel"] → [a, b folded]
#[test] fn a_removed_panel_stays_removed()          // saved items [a], removed [b] → [a]
#[test] fn a_new_panel_lands_folded_after_its_registry_neighbour() // defs a,n,b; saved [b, a] → [b, a, n folded]
#[test] fn a_new_first_panel_lands_at_the_start()   // defs n,a; saved [a] → [n folded, a]
#[test] fn queue_coalesces() {
    let mut q = SaveQueue::default();
    assert!(q.changed());       // first change saves at once
    assert!(!q.changed());      // second waits
    assert!(!q.changed());      // third folds into the second
    assert!(q.finished());      // one follow-up save, with the latest state
    assert!(!q.finished());     // nothing left
    assert!(q.changed());       // idle again
}
```

- [ ] **Step 2:** `cargo test -p finance-ui --bins layout::` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS. Commit `feat(ui): panel registry types and layout resolution`.

---

### Task 5: `PanelFrame`, `PanelList`, and the page view

**Files:**
- Create: `app/ui/src/ds/panel.rs`
- Modify: `app/ui/src/ds/mod.rs` (`pub mod panel;` and re-exports), `app/ui/src/ds/icons.rs` (a chevron, if none exists), `app/ui/src/layout.rs`, `app/ui/src/main.rs`, `app/ui/styles/glass.css` (only if `.page-stack` needs a rule for the list; no new tokens)

**Interfaces:**
- Consumes: Task 4; commands from Task 3 through `api::invoke`.
- Produces:

```rust
// ds/panel.rs
pub const PANEL_LIST: &str;      // two columns from `md`, one below; gap equal to `.page-stack`'s 1.5rem
#[component] pub fn PanelList(children: Children) -> impl IntoView;
#[component] pub fn PanelFrame(
    #[prop(into)] title: String,
    #[prop(into)] summary: Signal<Option<String>>,
    #[prop(into)] folded: Signal<bool>,
    on_toggle: Callback<()>,
    #[prop(optional)] half: bool,
    children: ChildrenFn,
) -> impl IntoView;

// layout.rs
#[derive(Clone, Copy)] pub struct LayoutState { /* RwSignal<UiLayout>, StoredValue<SaveQueue> */ }
impl LayoutState {
    pub fn load() -> Self;                                   // reads `ui_layout` once; default until it arrives or when it fails
    pub fn toggle(&self, tab: &'static str, tab_name: &'static str, page: &'static str, panel: &str);
    pub fn set_all(&self, tab: &'static str, tab_name: &'static str, page: &'static str, folded: bool);
}
pub fn registry() -> impl Iterator<Item = &'static PanelDef>; // chains each page module's `PANELS`
#[component] pub fn PageView(tab: &'static str, tab_name: &'static str, page: &'static str) -> impl IntoView;
```

Decisions the spec left open:

- **One header per panel.** Open: the panel renders as it does today, and `PanelFrame` adds one small fold button at the right end of the section's top rule (`.flat-section`'s border), sitting in the gap above the section, so it adds no row and collides with nothing in the panel's own header. Folded: one row on that rule: the title on the left (`CardTitle` at `H2` size class used by section titles today), the summary on the right in muted text, then the same button. The whole folded row is the button.
- The button is a real `<button>` with `aria-expanded` and `aria-controls`; Enter and Space work because it is a button.
- **Body kept after first open** (Review Focus 3): `PanelFrame` mounts `children` the first time `folded` is false and afterwards hides it with the `hidden` attribute when folded. A panel folded from the start is not mounted and loads nothing.
- A change writes the page's full resolved list into the layout (`page_mut`), so the saved page is self-contained from then on.
- `PageView` renders "Fold all / Open all" (one `Button`, `Variant::Ghost`, right-aligned above the list) and then `PanelList` of `PanelFrame`s. `LayoutState` is created in `App` and provided by context; it never touches `AppState::revision`.
- A failed save is ignored on screen in this phase; the state stays as clicked.

- [ ] **Step 1:** Implement `ds/panel.rs`, `LayoutState`, `registry()` (empty chain for now), `PageView`.
- [ ] **Step 2:** Wasm clippy and `cargo test --locked -p finance-ui --bins` → clean. Commit `feat(ui): panel frame, panel list and page view`.

---

### Tasks 6 to 10: Turn each page's cards into registry panels

The same recipe for each page; the tables give what the implementer cannot guess. Run after the rebase onto the cleaned-up `main`.

Recipe:

1. Create the page's state once in the page component and `provide_context` it (as `closure::Wizard` does today); panels read it with `expect_context`. Popups, the payment picker, banners and message lines stay rendered by the page, outside the list.
2. Give each panel a `fn() -> AnyView` that renders exactly what the card renders today (a per-currency repeat stays inside the one panel; a panel with nothing to show renders nothing).
3. Add `pub const PANELS: &[PanelDef]` to the page module, in the order of the table, and chain it into `layout::registry()`.
4. Replace the page's own wrapper (`grid gap-4`, leftover outer margins) with `<PageView tab=… tab_name=… page=… />`.
5. Wasm clippy; `cargo test --locked -p finance-ui --bins`; `trunk build`; open the page in the devserver on a ledger copy, "Open all", and check against the Task 0 screenshot that every card is present and unchanged inside; fold and reopen each; reload and see the state kept. Commit.

`summary` is `None` unless the table names one. A named summary reuses the helper the panel already uses for that number; it does not recompute it.

### Task 6: Overview

**Files:** `app/ui/src/pages/overview.rs`, `overview/{kpis,nudges,monthly,budget,forecast,visuals,extras}.rs`

`budget::MonthlyBudgetPanel` is split: its `PanelState` is created in `Overview` and provided by context; its section header and month selection stay with the budget table panel. `monthly::CurrencyOverview` is split into its two blocks.

| id | title | initial | movable | summary |
|---|---|---|---|---|
| `overview.kpis` | This month at a glance | Open | yes | – |
| `overview.nudges` | Do this now | Open | yes | number of nudges ("3 things"), none when zero |
| `overview.spendable` | Spendable and From salary | Folded | yes | "Spendable €X" |
| `overview.heading` | Where this month is heading | Folded | yes | – |
| `overview.budget` | Monthly budget | Open | no | "€spent of €planned" |
| `overview.week` | The next seven days | Folded | yes | "≈ €X expected" |
| `overview.trajectory` | Spending trajectory | Folded | no | – |
| `overview.extras` | This month only | Folded | no | – |

`overview.week`, `.trajectory` and `.extras` read the selected month from the provided `PanelState` and fall back to the current month when the budget panel is not mounted.

### Task 7: Transactions and Spending

**Files:** `app/ui/src/pages/transactions/mod.rs`, `pages/spending.rs`, `pages/organise.rs`, `pages/category_trends.rs`, `pages/goals.rs`, `pages/adjustments.rs`, `pages/overview/outlook.rs`, `app/ui/src/main.rs` (`SpendingTabs`)

The `variable` signal of Habits & goals moves into the weekday panel, with its switch. The "Review shared spending" link stays above the list.

| page | id | title | initial | summary |
|---|---|---|---|---|
| `transactions` | `transactions.search` | All transactions | Open | – |
| `transactions` | `transactions.personal` | Shared spending and money back | Folded | – |
| `transactions` | `transactions.inbox` | Transaction inbox | Open | – |
| `spending.habits` | `spending.habits.outlook` | Three-month outlook | Open | the end balance the card's headline shows |
| `spending.habits` | `spending.habits.goals` | Goals | Folded | – |
| `spending.habits` | `spending.habits.adjustments` | Adjustments | Open | – |
| `spending.habits` | `spending.habits.weekday` | Spending by weekday | Folded | – |
| `spending.organise` | `spending.organise.coverage` | Category coverage | Open | – |
| `spending.organise` | `spending.organise.bulk` | Purchases paid together later | Folded | – |
| `spending.organise` | `spending.organise.groups` | Organise spending | Open | – |
| `spending.history` | `spending.history.monthly` | Monthly spending | Open | – |
| `spending.history` | `spending.history.compare` | Compare categories | Open | – |

All `movable: true`.

### Task 8: Plan

**Files:** `app/ui/src/pages/plan.rs`

`plan::Ui` and `Shared` are provided by context. The month switcher renders inside the payments panel, above the board.

| id | title | initial | movable |
|---|---|---|---|
| `plan.salary` | Salary and where the money goes | Open | yes |
| `plan.payments` | Payments | Open | no |
| `plan.add` | Add a payment or income | Folded | no |
| `plan.bills` | Bills | Folded | no |

### Task 9: Wishes and Events

**Files:** `app/ui/src/pages/wishes.rs`, `pages/events.rs`

`wishes::Ui`, `Context` and `SettingsForms` (Wishes) and `wishes::Ui`, `Own` and `Context` (Events) are provided by context by each page after its data has loaded. All `movable: false`.

| page | id | title | initial |
|---|---|---|---|
| `wishes` | `wishes.board` | Wishes | Open |
| `wishes` | `wishes.add` | Add a wish | Folded |
| `wishes` | `wishes.done` | Bought and dropped | Folded |
| `wishes` | `wishes.months` | Month by month | Folded |
| `wishes` | `wishes.cards` | Credit cards | Folded |
| `wishes` | `wishes.fund` | Emergency fund | Folded |
| `events` | `events.board` | Events | Open |
| `events` | `events.add` | New event | Folded |

### Task 10: Manage

**Files:** `app/ui/src/pages/manage.rs`, `manage/{banks,import,status,settings,email,ai,ui}.rs`, `pages/closure.rs`, `closure/{panels,plan}.rs`, `app/ui/src/main.rs` (`ManageTabs`)

`manage::ui::Ops` and each section's loaded workspace are provided by context; `frame()`'s messages and loading card stay above the list. All `movable: false`.

| page | id | title | initial | half |
|---|---|---|---|---|
| `manage.banks` | `manage.banks.connections` | Bank connections | Open | |
| `manage.banks` | `manage.banks.consent` | Connect or renew | Folded | |
| `manage.banks` | `manage.banks.key` | Application key | Folded | |
| `manage.close` | `manage.close.wizard` | Close the month | Open | |
| `manage.close` | `manage.close.plan` | Where the money goes | Open | |
| `manage.close` | `manage.close.accounts` | Accounts | Folded | |
| `manage.import` | `manage.import.csv` | Import transactions | Open | |
| `manage.status` | `manage.status.connections` | Connections | Open | |
| `manage.status` | `manage.status.review` | Category review | Open | yes |
| `manage.status` | `manage.status.health` | Account data health | Open | yes |
| `manage.settings` | `manage.settings.preferences` | Finance preferences | Open | |
| `manage.settings` | `manage.settings.email` | Daily spending email | Folded | |
| `manage.settings` | `manage.settings.ai` | AI suggestions | Folded | |
| `manage.settings` | `manage.settings.data` | Local data | Folded | |
| `manage.settings` | `manage.settings.forecast` | Forecast model | Folded | |
| `manage.settings` | `manage.settings.updates` | Updates | Folded | |

---

### Task 11: Registry check, version, and the whole app

**Files:**
- Modify: `app/ui/src/layout.rs` (test), `Cargo.toml` (`[workspace.package] version = "0.3.0"`), `app/src-tauri/tauri.conf.json` (`"version": "0.3.0"`), `Cargo.lock`

- [ ] **Step 1: Write the registry test**

```rust
#[test] fn registry_is_sound() {
    let defs: Vec<_> = registry().collect();
    let ids: std::collections::HashSet<_> = defs.iter().map(|d| d.id).collect();
    assert_eq!(ids.len(), defs.len());                              // ids are unique
    assert_eq!(defs.len(), 48);                                     // every panel of the spec's tables
    for d in &defs { assert!(d.id.starts_with(d.page) && PAGES.contains(&d.page), "{}", d.id); }
    for page in PAGES { assert!(defs.iter().any(|d| d.page == *page && d.initial == Initial::Open), "{page} opens empty"); }
}
```

with `pub const PAGES: [&str; 13]` in `layout.rs` holding the page ids of Task 1.

- [ ] **Step 2:** Raise the version in both files; `cargo check` to refresh `Cargo.lock`.
- [ ] **Step 3:** All five CI commands → pass.
- [ ] **Step 4: The whole app, on a ledger copy in the devserver, at 1440 and 900 wide.** For each of the 13 pages:
  - first run (no saved layout) shows the table's Open panels open and the rest as one-line rows, and fits in about one screen at 1440×900;
  - "Open all" shows every card of the Task 0 screenshot, unchanged inside;
  - fold two panels quickly, reload: both are still folded (Review Focus 2);
  - at 900 wide the two half panels of Status stack.
  Then, once: type in "Add a wish", fold the panel, open it: the text is there (Review Focus 3). Tab to a fold button, press Space: it folds; focus stays on the button. Press ⌘R (refresh): no panel changes state. Click a "Do this now" link that goes to another tab: it lands there. Stop the devserver, start it on an empty path so commands return `no-data`: folding on the error screen does not panic (Review Focus 4).
- [ ] **Step 5:** Send the owner before/after screenshots of Overview and one other tab.
- [ ] **Step 6:** Commit `feat: foldable, saved panels on every tab (0.3.0)`; open the pull request `v0.3: foldable panels`; after CI, squash-merge; confirm the macOS Release run produced `v0.3.0`.
