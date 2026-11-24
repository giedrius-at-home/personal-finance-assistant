#!/usr/bin/env bash
# From the Mac: build the server for Linux ARM (in Docker, e.g. colima) and
# the UI, copy both to the VM over the tailnet and restart the server.
# usage: app/server/deploy/push.sh <tailnet host, e.g. ubuntu@finance>
set -euo pipefail

host="${1:?usage: push.sh <user@tailnet-host>}"
root="$(cd "$(dirname "$0")/../../.." && pwd)"
# Under the repo (in the home folder), which colima shares with its VM.
out="$root/target/server-push"
rm -rf "$out" && mkdir -p "$out"

docker run --rm -v "$root":/src:ro -v "$out":/out -v finance-target:/target \
  -v finance-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target -w /src \
  rust:1-bookworm bash -c 'cargo build --release --locked -p finance-server && cp /target/release/finance-server /out/'
(cd "$root/app/ui" && trunk build --release --dist "$out/ui")

scp "$out/finance-server" "$host":/tmp/finance-server
scp -r "$out/ui" "$host":/tmp/finance-ui
ssh "$host" 'sudo install -m 0755 /tmp/finance-server /opt/finance/finance-server &&
  sudo rm -rf /opt/finance/ui && sudo mv /tmp/finance-ui /opt/finance/ui && sudo chown -R root:root /opt/finance/ui &&
  rm -f /tmp/finance-server && sudo systemctl restart finance-server && systemctl --no-pager status finance-server | head -5'
