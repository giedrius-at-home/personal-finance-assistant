# Finance server

The app's commands, the built UI and the background work (bank sync, the
20:00 email, AI suggestions, forecasts) as one program on an always-on Linux
machine, so the phone can open Finance in a browser and syncs run with the
laptop closed. Design: `docs/superpowers/specs/2026-11-24-cloud-hosting-design.md`.

It listens only on the Tailscale address, never on the public internet,
and every request needs the server token (a header from the Mac app, or the
cookie the login page sets on the phone).

## Oracle VM, once

1. **Instance** (Compute → Instances → Create): image Canonical Ubuntu 24.04
   (aarch64), shape `VM.Standard.A1.Flex` with 2 OCPU and 12 GB, boot
   volume 50 GB. Networking: the default new VCN with a public subnet and a
   public IPv4 address (Tailscale needs outbound internet). Paste your SSH
   public key (`~/.ssh/id_ed25519.pub`). "Out of host capacity" means try
   again later or another availability domain.
2. **SSH once over the public address**, `ssh ubuntu@<public ip>`, and:
   ```bash
   sudo apt-get update && sudo apt-get -y upgrade && sudo apt-get -y install unattended-upgrades
   curl -fsSL https://tailscale.com/install.sh | sh
   sudo tailscale up --ssh --hostname finance
   ```
   Approve the machine in the Tailscale admin page that link opens.
3. **Close the public door.** Networking → the VCN → its security list:
   delete the ingress rule for port 22. From now on `ssh ubuntu@finance`
   goes over Tailscale. (Tailscale itself needs no inbound rule.)
4. **Install the service**: copy `app/server/deploy/` to the VM and run
   `sudo ./setup.sh`. It prints the server's address and makes a token in
   `/var/lib/finance/secrets/server-token`.
5. **Push the build** from the Mac (Docker via `colima start`, and `trunk`):
   `app/server/deploy/push.sh ubuntu@finance`. Run it again after each change.

Until the move (step 5 of the design), the server runs against a **copy** of
the ledger and no bank secrets: syncs report `missing_config`.

## Layout on the VM

| Path | What |
|---|---|
| `/opt/finance/finance-server`, `/opt/finance/ui/` | The program and the built UI |
| `/var/lib/finance/Finance/finance.sqlite3` | The ledger; `backups/` next to it |
| `/var/lib/finance/secrets/` | `server-token`, later the bank and mail secrets (0600) |
| `/etc/finance-server.env` | `FINANCE_LISTEN=<tailnet ip>:8443` |

Logs: `journalctl -u finance-server`.

Forecasts use TimesFM when its two files (`config.json`, `model.safetensors`,
1.3 GB) are in `/var/lib/finance/Finance/models/timesfm-3.0/`, owned by
`finance`; restart after copying them. Without them, simple forecasts.

## Running it locally

```bash
FINANCE_LISTEN=127.0.0.1:18443 FINANCE_UI_DIST=app/ui/dist \
FINANCE_SECRETS_DIR=/path/to/scratch/secrets FINANCE_DB=/path/to/copy.sqlite3 \
cargo run -p finance-server
```

`FINANCE_SECRETS_DIR` needs a `server-token` (`openssl rand -hex 32`). Never
point a local run at the real secrets folder: it would sync the real banks
against the copy.

## The Mac app as a window onto the server

Put the server's address in `~/Library/Application Support/Finance/server-url`
(one line, e.g. `http://100.83.10.51:8443`) and the server token in
`…/Finance/secrets/server-token` (0600), then restart the app. Every command
except updates now goes to the server, and the app stops its own bank sync,
email, suggestions and forecasts. Plain `http://` is accepted only for a
Tailscale address; anything else must be `https://`. Delete `server-url`
and restart to return to the local ledger (the two do not merge).

Not yet in server mode: screens do not reload by themselves after a sync
on the server (use the refresh button), and alerts are not shown.

## The phone

Open the server's `https://` address in Chrome, sign in with the token once,
then menu → "Install app" (the UI ships a web app manifest and icons).

## Backups

The server holds the ledger; the Mac keeps copies. `app/server/deploy/pull-backup.sh ubuntu@<tailnet host>`
fetches a consistent copy into `~/Library/Application Support/Finance/server-backups/`
(integrity-checked, last 14 kept). To go back to the Mac: quit the app, put a
copy at `Finance/finance.sqlite3`, move `secrets/on-server/*` back into
`secrets/`, delete `server-url`, and stop the server so only one side syncs.

## Development and CI on the server

The `dev` user (no sudo) holds the checkout (`~/finance`), the Rust
toolchain, Trunk and `cargo-deny`, and runs the repository's GitHub Actions
runner (`~/actions-runner`, labels `self-hosted, Linux, ARM64`), so CI uses
no Actions minutes. `dev` cannot read `/var/lib/finance`. Its one root
command is `/usr/local/sbin/finance-deploy`, which installs the build in
`~/finance` and restarts the service; `app/server/deploy/deploy-here.sh`
builds and then calls it. Deployed code still runs with the ledger, so the
separation guards against accidents, not against what gets merged.

## The phone: Cloudflare Access instead of the token

Behind the tunnel (`cloudflared`, public hostname → `http://<tailnet ip>:8443`,
an Access application allowing only the owner's email) the server trusts
the Access login and shows no token page. `/etc/finance-access.env`:

```
FINANCE_ACCESS_TEAM=<name>.cloudflareaccess.com
FINANCE_ACCESS_AUD=<the application's AUD tag>
FINANCE_ACCESS_EMAIL=<the owner's email>
```

Each request's `Cf-Access-Jwt-Assertion` must verify against the team's
keys, be for that AUD, in date, and name that email; otherwise 403. Without
the file, tunnel requests are refused. The token stays for the Mac app and
for browsers on the tailnet. Refusals are logged with their reason.
