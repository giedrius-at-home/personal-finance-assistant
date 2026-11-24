#!/usr/bin/env bash
# On the server: build the server and the UI from its own copy of the repo
# (~/finance.git, pushed to over Tailscale: `git push oracle main`, or
# pulled from GitHub once `gh auth login` was done here) and
# restart the service. No Docker and no build on the Mac. Run as `dev`, who
# may run only /usr/local/sbin/finance-deploy as root.
# usage: ~/finance/app/server/deploy/deploy-here.sh [branch, default main]
set -euo pipefail

branch="${1:-main}"
export PATH="$HOME/.cargo/bin:$PATH"
TRUNK=0.22.1

[ -d ~/finance ] || git clone -q ~/finance.git ~/finance
cd ~/finance
git fetch -q origin && git checkout -q -B "$branch" "origin/$branch"
rustup show active-toolchain >/dev/null   # installs rust-toolchain.toml's toolchain and wasm target

if ! trunk --version 2>/dev/null | grep -q "$TRUNK"; then
  curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v$TRUNK/trunk-aarch64-unknown-linux-gnu.tar.gz" \
    | tar -xz -C ~/.cargo/bin trunk || cargo install --locked trunk --version "$TRUNK"
fi

cargo build --release --locked -p finance-server
(cd app/ui && trunk build --release)

sudo /usr/local/sbin/finance-deploy
git log --oneline -1
