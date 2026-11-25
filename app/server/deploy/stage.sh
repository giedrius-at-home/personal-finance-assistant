#!/usr/bin/env bash
# Puts a build where /usr/local/sbin/finance-deploy installs it from
# (/home/dev/deploy). Used by deploy-here.sh and the CI deploy job.
# usage: stage.sh <finance-server binary> <ui dist folder>
set -euo pipefail

bin="${1:?usage: stage.sh <finance-server> <ui dist>}"
ui="${2:?usage: stage.sh <finance-server> <ui dist>}"
[ -x "$bin" ] && [ -f "$ui/index.html" ] || { echo "not a build: $bin $ui" >&2; exit 1; }

out=/home/dev/deploy
rm -rf "$out.new" && mkdir -p "$out.new"
install -m 0755 "$bin" "$out.new/finance-server"
cp -r "$ui" "$out.new/ui"
rm -rf "$out" && mv "$out.new" "$out"
