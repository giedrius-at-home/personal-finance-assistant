#!/usr/bin/env bash
# From the Mac: fetch a consistent copy of the server's ledger into
# ~/Library/Application Support/Finance/server-backups/ and keep the last 14.
# usage: app/server/deploy/pull-backup.sh <user@tailnet-host>
set -euo pipefail

host="${1:?usage: pull-backup.sh <user@tailnet-host>}"
dir="$HOME/Library/Application Support/Finance/server-backups"
mkdir -p "$dir" && chmod 700 "$dir"
target="$dir/finance-$(date -u +%Y%m%dT%H%M%SZ).sqlite3"

# VACUUM INTO gives a consistent copy of the live WAL database; it is
# streamed and removed, so nothing readable stays outside the data folder.
(umask 077; ssh -o BatchMode=yes "$host" 'sudo -u finance sh -c "
  umask 077; t=/var/lib/finance/Finance/.pull.sqlite3; rm -f \$t
  sqlite3 /var/lib/finance/Finance/finance.sqlite3 \"VACUUM INTO '"'"'\$t'"'"'\" && cat \$t; rm -f \$t"' > "$target.partial")

[ "$(sqlite3 "$target.partial" 'PRAGMA integrity_check')" = ok ] || { rm -f "$target.partial"; echo "backup failed the integrity check" >&2; exit 1; }
mv "$target.partial" "$target"
ls -1t "$dir"/finance-*.sqlite3 | tail -n +15 | while read -r old; do rm -f "$old"; done
echo "$target"
