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
| `crates/secrets` | Keychain access, zeroizing secret values |
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

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p finance-ui --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
cargo deny check
```

## Releases and updates

1. Bump `version` in `app/src-tauri/tauri.conf.json` and `[workspace.package]`.
2. Merge to `main`, then tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow builds and signs `Finance.app.tar.gz` and publishes it
   with `latest.json`. Merges alone never ship anything.

The installed app checks at launch and every 6 hours. Because the repository
is private it needs a fine-grained GitHub token with read-only **Contents**
access to this repository; paste it once under **Manage → Settings → Updates** (stored in
the Keychain). Updates whose signature does not match the public key in
`tauri.conf.json` are rejected.

The signing key lives in the repository secrets `TAURI_SIGNING_PRIVATE_KEY`
and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Losing it means releases can no
longer update installed apps; a new key needs one manual reinstall.
