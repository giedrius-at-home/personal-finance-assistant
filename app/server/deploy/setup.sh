#!/usr/bin/env bash
# Run once on the Oracle VM as root, after Tailscale is up (`tailscale up`).
# Creates the `finance` user and folders, a server token, the systemd unit,
# and lets the tailnet (only) reach the server's port. Safe to run again:
# it keeps an existing token and ledger.
set -euo pipefail

PORT="${PORT:-8443}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ip="$(tailscale ip -4 | head -1)"
[ -n "$ip" ] || { echo "Tailscale has no address: run 'tailscale up' first." >&2; exit 1; }

id finance >/dev/null 2>&1 || useradd --system --home-dir /var/lib/finance --shell /usr/sbin/nologin finance
install -d -o finance -g finance -m 0700 /var/lib/finance /var/lib/finance/Finance /var/lib/finance/secrets
install -d -o root -g root -m 0755 /opt/finance /opt/finance/ui

token=/var/lib/finance/secrets/server-token
if [ ! -s "$token" ]; then
  (umask 077; openssl rand -hex 32 > "$token")
  chown finance:finance "$token"
  echo "New server token in $token (sudo cat it once to sign in from the phone and the Mac)."
fi

echo "FINANCE_LISTEN=$ip:$PORT" > /etc/finance-server.env
install -m 0644 "$HERE/finance-server.service" /etc/systemd/system/finance-server.service

# Oracle's Ubuntu images reject everything but SSH in iptables. Accept the
# tailnet interface only; the public interface stays closed.
if ! iptables -C INPUT -i tailscale0 -j ACCEPT 2>/dev/null; then
  iptables -I INPUT 1 -i tailscale0 -j ACCEPT
  command -v netfilter-persistent >/dev/null && netfilter-persistent save
fi

systemctl daemon-reload
systemctl enable finance-server
echo "Set up. Push the build with push.sh from the Mac, then: systemctl start finance-server"
echo "Address: http://$ip:$PORT"
