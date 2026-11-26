# Finance

Local-first personal finance app for macOS, written in Rust (Tauri 2 + Leptos).
Everything runs on this Mac: data lives in one SQLite file, the window uses
macOS Liquid Glass, and updates arrive as signed GitHub releases the app
checks for itself. The screens follow the web finance UI
(finance.giedriusathome.com, `personal-site`): same sections, design tokens,
Manrope and Tailwind classes, and the Python service's logic ported to Rust.

Design: [docs/specs/2026-10-26-finance-assistant-rust-design.md](docs/specs/2026-10-26-finance-assistant-rust-design.md)

## Layout

| Path | What |
|---|---|
| `crates/types` | Domain types shared by the backend and the UI |
| `crates/storage` | SQLite schema, ledger import and the ledger queries |
| `crates/core` | Finance logic: the canonical model, every report and every write (the Python service's port) |
| `crates/secrets` | Secrets in owner-only files (`Finance/secrets/`, 0600), zeroizing in memory |
| `crates/bank` | Enable Banking client (RS256 application token), consents in the secret store, one sync at a time |
| `crates/timesfm` | TimesFM 3.0 inference (ndarray; Accelerate on macOS) for the forecasts |
| `crates/ai` | Category suggestions on Mistral or Jev (TypeSafe), plus merchant research |
| `crates/mistral` | Mistral AI client (EU): chat, embeddings, Voxtral speech |
| `crates/notify` | Daily spending email (SMTP), Web Push, and Gisel's Mistral writer and voice |
| `crates/migrate` | One-off copy of the data from `laptopthree` |
| `app/src-tauri` | The macOS app: window, glass, commands, updater |
| `app/ui` | Leptos screens (compiled to WASM by Trunk; Tailwind 4 through Trunk) |
| `app/devserver` | Development only: serves the UI and commands over loopback |

## Setup

```bash
brew install rustup trunk cargo-binstall
rustup-init -y   # the toolchain itself comes from rust-toolchain.toml
cargo binstall -y tauri-cli@2.12.4
```

## Copy the data from laptopthree

Reads over SSH on the home network, writes nothing on the server, and checks
every account's postings and sums against MariaDB before replacing the local
database (the previous file is kept as `finance.sqlite3.before-migrate`).

```bash
cargo run --release -p finance-migrate
```

## Run

```bash
cd app/src-tauri && cargo tauri dev           # app with hot reload
cd app/src-tauri && cargo tauri build --bundles app   # needs the signing key, see below
```

To look at the UI in a browser against the real data:

```bash
(cd app/ui && trunk build) && cargo run --release -p finance-devserver -- app/ui/dist 1430
```

The development server writes too. Point it at a copy, never the real file,
when trying edits: `DEV_DB=/path/to/copy.sqlite3 cargo run --release -p finance-devserver …`.

Before the first write of each session the app saves a copy of the database
to `~/Library/Application Support/Finance/backups/` (the last ten are kept).

## Bank sync

Banks sync straight from Enable Banking, replacing the old importer and
Firefly. Set up once in Manage → Banks:

1. **Application key**: the Enable Banking application id and its `.pem`
   private key. Both go to the secret store (owner-only files).
2. **Consents**: "Use saved consents" takes over the consents the old
   importer saved on this Mac, with their run history. To connect a bank or
   renew an expiring consent, use "Connect or renew": approve in the browser,
   paste the address the bank returns to (registered redirect, default
   `https://localhost:8444/eb-callback`; the page need not load), then map
   each shared account onto a ledger account. Every ledger account the old
   consent filled must stay mapped. Each bank shows when its consent ends.

While the app is open it checks every 15 minutes and syncs each bank once a
day (hourly retries after a failure, six hours after a rate limit). Each sync
downloads the last 30 days, writes booked transactions it has not seen (by the
bank's entry reference) and flags closed months that received late ones.

## Forecasts

Daily spending, group month-end totals and the three-month outlook use
TimesFM 3.0 when its weights are on this Mac (Hugging Face cache, or
Settings → Forecast model downloads them to
`~/Library/Application Support/Finance/models/timesfm-3.0/`). Each method is
backtested against the old service's simple forecast on recent history and
kept only where it wins. Screens never wait: they show the simple forecast
until a background run (a few seconds, ~1.4 GB while it runs) has cached
TimesFM's answers for the current data.

## Suggestions, alerts and email

All off until switched on in Manage → Settings:

- **AI suggestions**: Mistral (EU-hosted) or Jev (TypeSafe) proposes categories
  for new transactions after each sync; proposals at 0.90 or more are applied
  automatically when that is on. Both keys live in the secret store
  (`mistral-api-key`, `typesafe-api-key`); nothing runs a model locally.
- **Notifications**: goal alerts as macOS notifications.
- **Daily spending email** at 20:00 over SMTP (password in the secret store),
  with a "Send test" button; Gisel's opening lines are written by Mistral.
- **Gisel's daily voice brief**: at the chosen hour (08:00 by default) Mistral
  writes a short script from the day's facts (what is left, groups near their
  limit, bills this week, habits against the four weeks before, one thing to
  change, a passing thought in her laid-back Spanish style), Voxtral speaks it, and the phone gets a push that
  opens the player on Overview. Every number Gisel says is checked against
  the facts; otherwise fixed words are used. With "Also send it on Telegram",
  the brief also arrives as a Telegram voice message from the speaker's own
  bot: make one bot each for Gisel and Rafael with @BotFather, paste the
  tokens in the brief card (stored as `telegram-bot-gisel` and
  `telegram-bot-rafael` in the secret store), send each bot a message, and
  press Test; the chat is found and saved from that message.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
cargo deny check
```

## Releases and updates

Every push to `main` that passes CI is released: once CI has passed (and
deployed the Oracle server), the Release workflow builds and signs
`Finance.app.tar.gz`, tags the commit and publishes it with `latest.json`.
It can also be started by hand: **Actions → Release → Run workflow** (or
`gh workflow run release.yml`).

The build runs on this Mac as a self-hosted GitHub runner (GitHub's own
macOS runners are paid), so the runner must be started; while the Mac sleeps a release waits and
builds when it wakes.

Set the runner up once: GitHub → repository Settings → Actions → Runners →
New self-hosted runner → macOS, ARM64, and run the commands it shows in a
folder such as `~/actions-runner`. Then, in that folder:

```bash
./svc.sh install && ./svc.sh start   # starts with the Mac from now on
```

The runner needs `rustup` (above), Xcode's command line tools and the
GitHub CLI (`brew install gh`); the workflow installs Trunk and the Tauri CLI
itself.

- The version is the latest release's patch plus one, stamped into the build
  only; `main` keeps the last hand-set version.
- For a minor or major release, raise `version` in
  `app/src-tauri/tauri.conf.json` and `[workspace.package]` in the PR; the
  workflow uses it when it is newer than the latest release.

The installed app checks at launch and every 15 minutes, installs a new
release by itself and restarts into it once its window is not in front. Because the repository is
private it needs a fine-grained GitHub token with read-only **Contents**
access to this repository; paste it once under **Manage → Settings → Updates** (stored in
the secret store). Updates whose signature does not match the public key in
`tauri.conf.json` are rejected.

The signing key lives in the repository secrets `TAURI_SIGNING_PRIVATE_KEY`
and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Losing it means releases can no
longer update installed apps; a new key needs one manual reinstall.
