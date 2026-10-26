# Finance Assistant: local Rust macOS app — design

Date: 2026-10-26
Status: approved design, awaiting spec review

## 1. Why

The finance stack ("Rafael") runs on `laptopthree` as nine Docker containers
(Python gateway, Firefly III + MariaDB, Firefly data importer, Ghostfolio +
Postgres + Redis, cron) behind Tailscale and a Cloudflare tunnel. Reaching it
depends on that network path; on 2026-10-26 Tailscale on `laptopthree` was
logged out after a reboot and every remote route failed.

The owner wants the finance assistant to run on their Mac, which is always
on and online, as a native app with the macOS glass look, with no Firefly and
no Docker, and with old data compressed and kept on Google Drive.

## 2. Goals

1. A single macOS app (`Finance.app`) built in Rust: Tauri 2 shell, Leptos UI,
   native glass window material.
2. Finance business logic rewritten in Rust. Python stays only for external
   connectors, behind a small local HTTP API.
3. No Firefly, no data importer, no Ghostfolio, no Docker. Bank data comes
   from Enable Banking straight into local SQLite.
4. Existing data is copied once over the LAN from `laptopthree.home`; nothing
   is re-imported or recomputed.
5. Old data lives only on Google Drive as encrypted, heavily compressed
   Parquet, and is fetched and decompressed on demand.
6. Latest stable toolchain and crates, no duplicated definitions, memory-safe
   code, secrets never in plaintext on disk or in logs.

## 3. Non-goals

- Wardrobe, writing, design pages and the rest of `personal-site` stay where
  they are. Only finance moves.
- No phone or remote access. `finance.giedriusathome.com` is retired.
- No investments view (Ghostfolio is dropped). Manual investment valuations
  already in SQLite are kept as data.
- No multi-user support.

## 4. Repository

This repo (`giedrius-at-home/personal-finance-assistant`) is reused. Its old
Python/Firefly contents are removed on the `rust-rewrite` branch; history is
kept. The current Python gateway source of truth is
`personal-site/services/finance`, used as reference and for test fixtures.
The remote branch `experiment/direct-enable-banking-compare` holds an earlier
direct Enable Banking experiment and is reference material for §7.

```
Cargo.toml                 workspace, edition 2024, shared lints + deps
crates/
  types/                   shared domain types (serde), used by every crate and the UI
  storage/                 SQLite (hot), migrations, cold archive (Parquet/zstd/age), Drive paths
  core/                    finance logic: closure, forecasts, categorisation, review, search, alerts
  bank/                    Enable Banking client + sync into storage
  secrets/                 Keychain access, zeroizing secret wrappers
  migrate/                 one-off LAN import from laptopthree (CLI)
app/
  src-tauri/               Tauri shell: window, glass, menu bar, commands, supervisor
  ui/                      Leptos (CSR, WASM) screens and components
connectors/                Python (uv): Ollama, Google Calendar, email — local HTTP API
design/tokens/             design tokens (copied from personal-site), single source
fixtures/                  golden fixtures exported from the Python gateway
docs/specs/
```

### Versions (checked 2026-10-26, pinned in `Cargo.lock`)

| Component | Version |
|---|---|
| Rust | 1.99.1 stable, edition 2024 |
| tauri | 2.12.5 |
| leptos | 0.8.22 |
| rusqlite (bundled SQLite) | 0.41.0 |
| arrow / parquet | 60.0.0 |
| duckdb | 1.20000.0 (DuckDB 1.2) |
| age | 0.12.1 |
| zeroize | 1.9.1 |
| security-framework (Keychain) | 3.7.0 |
| reqwest (rustls) | 0.13.5 |
| tokio | 1.54.1 |
| rust_decimal | 1.43.0 |
| jiff | 0.2.38 |
| thiserror | 2.0.21 |
| tracing | 0.1.44 |

Python connectors: latest CPython stable via `uv`, FastAPI. Dependabot (cargo,
uv, github-actions) keeps all of these current.

## 5. Architecture

```
┌──────────────── Finance.app (one process) ────────────────┐
│ Leptos UI (WASM in WKWebView, glass window material)      │
│        │ Tauri commands (typed, from crates/types)        │
│ Rust: core ── storage (SQLite hot + Drive cold) ── bank   │
│        │                                                   │
│        └─ HTTP 127.0.0.1 ─> connectors (Python, child)    │
└───────────────────────────────────────────────────────────┘
launchd agent: `finance sync` (same binary, headless) on a schedule
```

- **UI ↔ Rust**: Tauri commands only. No HTTP server in Rust; nothing listens
  on a port except the connectors process on `127.0.0.1` with a random port
  and a per-launch bearer token passed via environment.
- **Glass**: Tauri window with transparent background; the native shim sets
  `NSGlassEffectView` (macOS 26) behind sidebar and title bar, falling back to
  `NSVisualEffectView`. Content panels use the design-token surfaces with CSS
  backdrop blur so they read as glass in light and dark mode.
- **Menu bar item**: last sync time, Sync banks now, Open, Quit.
- **Headless mode**: `Finance.app/Contents/MacOS/finance sync` runs bank sync,
  alert evaluation and daily review without opening a window. A launchd agent
  installed by the app runs it on the schedule the owner sets (default 07:00
  and 19:00).
- **Connectors process**: started on demand by the app or the headless run,
  stopped when idle. It holds no state; it gets inputs per request and returns
  results.

### What stays in Python (`connectors/`)

| Connector | Why Python |
|---|---|
| Ollama (categorisation suggestions, vision) | Prompt code and evaluation already tuned there |
| Google Calendar (planned payments ↔ events) | Google client libraries |
| Spending email (HTML forecast mail) | Existing templates |

macOS notifications move to Rust (Tauri notification plugin).

## 6. Data model and storage

### Hot store — SQLite

One database at `~/Library/Application Support/Finance/finance.sqlite3`
(WAL mode, `secure_delete=ON`, file mode 0600, folder excluded from Spotlight).

- The existing gateway schema (owner settings, month periods, planned payments,
  pots, goals, category proposals/corrections, spending groups, daily review,
  alerts, subscriptions, research cache, etc.) is kept and copied as is.
  `schema_migrations` continues from the current version.
- New tables replace Firefly: `accounts`, `categories`, `bills`, `transactions`
  and `postings` (one signed row per account a transaction touches, so every
  balance is an exact sum), a trigram FTS index `transactions_search`, and
  `import_runs`. Phase 2 adds `bank_connections` and `sync_runs`; Firefly had
  no budget limits in use, so none are migrated.
  Firefly-specific tables (`firefly_sync_health`, `webhook_deliveries`) are
  dropped after migration.
- Money is `rust_decimal::Decimal` stored as TEXT (exact), never floats.
  Dates use `jiff`.
- "Hot" = current month + the last 12 closed months.

### Cold store — Google Drive only

- Location: a `Finance Archive` folder in Google Drive, accessed through the
  Google Drive desktop app in **streaming mode**, so files are online-only and
  downloaded only when opened.
- Format: one Parquet file per table per closed month, zstd level 19,
  dictionary encoding on; then encrypted with `age` (X25519). File name
  `<table>/<YYYY-MM>.parquet.zst.age`. A small encrypted `manifest.json.age`
  lists files, row counts and SHA-256 of the plaintext.
- Old SQLite snapshots (`/var/lib/pfa/audits/**`, `before-*.sqlite3`,
  ~3.5 GB) are archived as `snapshots/<name>.sqlite3.zst.age` (zstd -19
  `--long=31`), not converted.
- Archive job (monthly, after month closure): write Parquet → encrypt → write
  to Drive → read back, decrypt, compare row count + hash → only then delete
  those rows from SQLite. Any mismatch aborts and keeps the rows.
- Read path: a query that spans archived months (search, trends, history)
  asks `storage::cold` for the months; it reads the file from Drive, decrypts
  and decompresses into memory (or a 0600 temp file for large months),
  queries with DuckDB, unions with hot rows, and keeps an LRU cache capped at
  256 MB in memory. Temp files are deleted on eviction and at startup.
- If Drive is unreachable, the UI shows hot data and marks older ranges as
  unavailable; nothing fails silently.

### Backups

Nightly: SQLite online backup → zstd -19 → age → Drive `Finance Backups/`.
Retention 30 daily + 12 monthly. Restore is a CLI command (`finance restore`).

## 7. Bank sync (replaces Firefly + importer)

- `crates/bank` talks to the Enable Banking API: authorisation (consent
  renewal through the app opening the bank's consent page and receiving the
  redirect on a custom URL scheme `finance://eb-callback`), account list,
  balances, transactions with pagination.
- Application private key (RS256) for signing Enable Banking JWTs lives in
  the Keychain, never on disk.
- Sync is idempotent: transactions are keyed by bank entry reference (or a
  stable hash of booking date, amount, counterparty, description when the
  bank gives none), so re-running a window never duplicates.
- Existing logic is ported, not redesigned: own-transfer detection and
  double-booking checks, zero-amount warnings, reconciliation, sync forward
  from reconciled history, IBAN/no-IBAN account bindings.
- Each run records a `sync_runs` row with sanitized warning kinds (no amounts
  or IBANs).

## 8. Migration from laptopthree

`crates/migrate` is a CLI run once from the Mac over the LAN
(`rafael@laptopthree.home`, existing SSH key, host key pinned). It streams
data, writes nothing on `laptopthree`, and does not recompute anything.

1. Gateway state: `sqlite3 .backup` inside the gateway container, streamed
   over SSH → local `finance.sqlite3`.
2. Firefly: read accounts, transactions, splits, categories, bills, budget
   limits through the Firefly API (token read in-container, never printed) or
   `mariadb-dump` streamed over SSH, and load into the new tables, keeping
   Firefly ids in a `legacy_firefly_id` column so existing gateway references
   (category corrections, paid links, review entries) still resolve.
3. Enable Banking: bank configs from `Bank Imports/data/*.json` and the
   application private key → Keychain. Session IDs are reused where still
   valid.
4. Snapshots in `/var/lib/pfa/audits/**` and `before-*.sqlite3` are streamed
   straight into the cold archive (§6) without landing unencrypted on the Mac.
5. Verify: per-account transaction counts and balances, per-month spending
   totals, and open month-closure state must match the gateway API on
   `laptopthree`. Mismatch → stop, report, keep `laptopthree` running.
6. After two weeks of matching syncs (§11 phase 2), the owner stops the
   finance containers on `laptopthree` and removes the
   `finance.giedriusathome.com` tunnel hostname. A final encrypted backup of
   the old volumes goes to Drive.

## 9. Engineering rules

**Latest**: stable Rust, edition 2024, newest stable crates (§4), resolver 3.
CI fails on `cargo outdated` major drift older than 60 days.

**DRY**
- `crates/types` is the only definition of domain types; the Leptos UI and
  the Tauri commands use the same structs (serde), so there is no separate
  TypeScript or JSON schema to keep in sync.
- Connector requests and responses are defined once in `crates/types` and
  exported as JSON Schema; Python models are generated from it.
- Design tokens are one JSON source in `design/tokens/`; a build script emits
  CSS variables for the UI.
- Shared lints, dependency versions and profiles live in the workspace
  `Cargo.toml` (`[workspace.dependencies]`, `[workspace.lints]`).

**Memory safety and secrets**
- `#![forbid(unsafe_code)]` in every crate except the glass-window shim in
  `app/src-tauri`, where `unsafe` is limited to one module, each block has a
  `// SAFETY:` comment, and `clippy::undocumented_unsafe_blocks` is denied.
- Secrets (Enable Banking key, session tokens, age identity, connector bearer
  token) live in the macOS Keychain, are loaded into `secrecy`/`zeroize`
  wrappers, and are wiped on drop. They never appear in `Debug` output,
  logs, crash reports or the UI.
- The age identity has an offline recovery copy the owner prints once at
  setup; losing both means the cold archive is unrecoverable, which the setup
  screen states.
- Logging via `tracing` with a redaction layer: no amounts, IBANs, names,
  counterparties or tokens at any level.
- Network: outbound only to Enable Banking and the connectors on loopback.
  Tauri capabilities allow only the commands the UI uses; CSP blocks remote
  scripts. `reqwest` uses rustls.
- Supply chain: `cargo-deny` (licences, advisories, duplicate versions, banned
  crates) and `cargo-audit` in CI; `Cargo.lock` committed.
- App is signed and hardened-runtime enabled; data folder 0700.

**Tests**: core logic only (owner preference). Golden fixtures exported from
the Python gateway's tests and live data shapes; the Rust port of each module
must produce identical outputs. Plus archive round trip, prune-only-after-
verify, idempotent bank sync, and migration verification. No UI tests beyond
a launch smoke check.

## 10. Releases and updates

The app is a native, installed macOS app. It never builds from source or
pulls from `main` on the owner's Mac; it only installs released, signed
builds.

- **Release pipeline**: pushing a version tag `vX.Y.Z` (created from `main`
  after CI passes) runs a GitHub Actions workflow on a macOS runner that
  builds an Apple-silicon `Finance.app`, packages the `.app.tar.gz` (no DMG:
  creating one needs Finder automation, and updates do not use it), signs the
  update archive with the Tauri updater key (Ed25519), and
  publishes a GitHub Release with `latest.json` (version, notes, URL,
  signature). Merges to `main` alone never ship anything.
- **Code signing**: Developer ID signing and notarization if the owner has an
  Apple Developer account; otherwise ad-hoc signing, with the Tauri updater
  signature as the integrity check. The updater private key lives only in
  GitHub Actions secrets; the public key is compiled into the app.
- **In-app check**: the Tauri updater plugin checks `latest.json` at launch
  and every 6 hours (also from the headless run). The repo is private, so the
  request carries a fine-grained, read-only GitHub token scoped to this repo's
  contents, stored in the Keychain and sent as a header. An update whose
  signature does not verify is rejected.
- **Install**: the menu bar shows "Update to X.Y.Z" with release notes; the
  owner clicks to install, the app downloads, verifies, replaces itself and
  relaunches. No silent installs while a sync or archive job runs.
- **Schema safety**: each release runs pending SQLite migrations at startup
  after taking an automatic local backup; a failed migration restores the
  backup and keeps the old version running. Releases never downgrade.
- **Connectors**: the Python connectors are bundled inside the app (a
  `uv`-built standalone environment in `Contents/Resources/connectors`), so
  they update together with the app.

## 11. Phases

Each phase ends with a working app.

1. **Skeleton and data**: workspace, `types`, `storage` (hot), `secrets`,
   Tauri + Leptos shell with glass, read-only Overview, Transactions (with
   search), Accounts, Categories and Data screens. The Enable Banking key moves
   to the Keychain in phase 2, when the app first uses it. `migrate` copies all data from `laptopthree`; verification
   passes. Release workflow and in-app updater (§10) work from the first
   tagged build. `laptopthree` stays the live system.
2. **Bank sync**: `bank` crate, consent renewal, launchd headless sync,
   notifications. Runs side by side with `laptopthree` until results match
   for two weeks, then `laptopthree` finance is stopped (§8.6).
3. **Logic port**, one module per PR, each gated by golden fixtures:
   month closure (incl. via routes and transfers), spending/savings forecast,
   daily forecast, planned payments and bills, categorisation + review inbox
   (with Ollama connector), spending groups and budgets, alerts,
   subscriptions, transaction search, Google Calendar and email connectors.
   Matching UI screens are rebuilt in Leptos in the same PR.
4. **Cold storage**: archive job, snapshot archive, Drive read path with
   DuckDB, nightly backups, restore command.

## 12. Risks

| Risk | Mitigation |
|---|---|
| Ported logic drifts from Python behaviour | Golden fixtures per module; Python gateway on `laptopthree` stays as reference until phase 3 ends |
| Firefly id references break after migration | `legacy_firefly_id` kept; verification checks every reference resolves |
| Enable Banking consent renewal via custom URL scheme rejected | Fallback: loopback HTTPS redirect on `localhost:8444` as today |
| Drive desktop app not running / offline | Hot data unaffected; cold ranges shown as unavailable; archive job retries |
| Loss of age identity | Keychain + printed recovery copy; setup screen warns |
| Leptos ecosystem lacks a chart component | Charts drawn as SVG components from the existing chart specs |
