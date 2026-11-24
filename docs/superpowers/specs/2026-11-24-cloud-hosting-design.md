# Cloud hosting on Oracle's free tier: study and proposal

Date: 2026-11-24
Status: proposal. Nothing is built, no account or cloud resource exists, the
ledger has not been copied anywhere. Waiting for the owner to pick a way.

## Intent

Owner: "do you think we could push to https://www.oracle.com/cloud/free/ so
that we could host the data on the cloude and compute and locally we see all".

Short answer: yes, it can be done, and the code is closer to it than it
looks. But I would not move the only copy of the ledger to a free account.
I recommend an encrypted off-site backup first, and the hosted server only
as a second step, tried on a copy, if the owner wants what it gives.

## What exists today (read in the code)

- **One file.** `~/Library/Application Support/Finance/finance.sqlite3`,
  178 MB today, WAL mode. `Service` (`crates/core/src/service.rs`) opens it,
  keeps a read snapshot in memory and runs every write under one lock.
- **Backups are on the same disk.** Before the first write of a session
  `VACUUM INTO backups/` (last ten kept; the folder holds 2.0 GB). If the
  Mac is lost, the ledger and all its backups go together.
- **The commands already exist over HTTP.** `app/devserver` serves about 60
  of the app's commands as `POST /invoke/<command>`, and the UI already
  picks between Tauri and `fetch('/invoke/…')` (`app/ui/src/api.rs`). The
  devserver is loopback only, no TLS, no login, a hand-written HTTP parser,
  and it refuses bank sync, email, updates and the forecast model.
- **Background work lives in the Tauri shell.** Bank sync (every 15 min,
  each bank daily), the 20:00 email, alerts, AI suggestions and forecast
  runs are threads started in `app/src-tauri/src/{bank,notify,ai,model}.rs`
  and need the app's handle. They run only while the app is open.
- **Secrets are six 0600 files** in `Finance/secrets/`: the Enable Banking
  application id and private key, three bank consents, the GitHub release
  token (plus SMTP and TypeSafe keys when set). `FINANCE_SECRETS_DIR`
  already lets another folder be used.
- **Bank consent is paste-based.** Approve in the browser, paste the
  address the bank returned to. No callback server, so it works from any
  machine. Consents last 90 to 180 days.
- **Things tied to this Mac:** macOS notifications, Ollama on
  `127.0.0.1:11434` (email copy, optional suggestions), TimesFM with
  Apple's Accelerate (about 1.4 GB while it runs), the signed updater.
- CI already builds and tests every crate except the app shell on Linux.

## Oracle Always Free (read 2026-11-24)

Read on oracle.com/cloud/free, its FAQ and docs.oracle.com (Always Free
Resources, Free Tier, Autonomous Always Free, Regions).

Free with no end date:

| What | Limit |
|---|---|
| ARM VM (A1 Flex) | **2 OCPU and 12 GB RAM in total** (1,500 OCPU-hours, 9,000 GB-hours a month). Was 4 and 24; see below |
| AMD VM (E2.1.Micro) | 2 VMs, 1/8 OCPU and 1 GB RAM each |
| Disk | 200 GB boot and block volumes together, 5 volume backups |
| Object storage | 20 GB, 50,000 API requests a month |
| Outbound data | 10 TB a month |
| Autonomous Database | 2, 1 OCPU and 20 GB each; stopped after 7 idle days, may be deleted after 90 days stopped |
| Vault | 150 secrets, software keys |
| Network | 2 VCNs, bastion, 1 small load balancer, site-to-site VPN |

Trial only: US$300 for up to 30 days. After it, Always Free resources stay;
paid ones are removed after a 30-day grace period.

The catches that matter here:

- **Idle VMs may be taken back.** A VM counts as idle when, over 7 days,
  CPU (95th percentile), network and (on ARM) memory are all under 20%. A
  server for one person's ledger is exactly that.
- **Idle accounts.** "Accounts left idle for 30 days or more may be deemed
  abandoned" (FAQ) and can be suspended or closed.
- **No SLA and no support** on a free-only account.
- **ARM capacity.** Oracle admits a "temporary lack of Always Free shapes";
  getting the VM "might take several days".
- **Home region** is picked at sign-up; free compute exists only there.
  European regions include Amsterdam, Frankfurt, Stockholm, Paris, Milan,
  Madrid, Zurich.
- **Sign-up needs a phone number and a real credit card** (no virtual or
  prepaid cards). Not charged unless upgraded to Pay As You Go. Upgrading
  cannot be undone; Always Free resources stay free after it.
- **Outbound port 25 is blocked.** The daily email uses authenticated SMTP,
  normally 465 or 587, so it should not be affected.
- The ARM allowance was halved in 2026. Oracle's pages show the new numbers
  without a date; InfoQ (July 2026) reports 15 June 2026, unannounced, with
  VMs over the limit shut down. A free offer that changed once can change
  again.

## Options

### A. The ledger and the compute live on an Oracle VM; the Mac app is a window

A new `finance-server` binary (the devserver's command table plus the
schedulers moved out of the Tauri shell) runs on the ARM VM with the SQLite
file on its disk. The Mac app sends every command to it.

- **Code changes.** (1) A headless host for the four schedulers, which today
  need the Tauri handle. (2) The refused commands (bank, email, model)
  wired in the server. (3) A login on the API and a private network in
  front of it. (4) The app sending commands to the server instead of its
  own `Service`, and polling for "bank synced" and "forecast ready" (Tauri
  events do not cross the network). (5) A Linux ARM build and a way to
  deploy it, in step with app releases, because the app and the server must
  agree on every command's shape and every merge ships. (6) A nightly pull
  of the ledger back to the Mac.
- **Gains.** Bank sync, the 20:00 email and suggestions run with the Mac
  closed. A phone or second computer could open the same UI in a browser.
- **Breaks or gets worse.** No offline use: no network, no app. macOS
  notifications cannot come from a server (the app would poll, or alerts go
  by email). Ollama is not there: email copy and local suggestions fall
  back or need TypeSafe. Forecasts lose Accelerate; 12 GB is enough memory
  but speed on 2 ARM cores is unmeasured. Every screen waits on a network
  round trip plus a slower CPU; unmeasured.
- **Cost at zero:** free, on the ARM VM. The 1 GB AMD VMs cannot run the
  forecast.
- **Owner's chores.** Create the Oracle account (card, phone). Keep it
  from looking abandoned. Patch the VM. Renew bank consents as now. Check
  that the nightly copy still arrives.
- **Size:** two to three weeks.

### B. Everything stays local; an encrypted copy of the ledger goes to object storage

After the backup the app already makes, encrypt the copy on the Mac and
upload it to a private bucket. Oracle only ever holds ciphertext.

- **Code changes.** Small: encrypt and upload after `backup()`, keep the
  last N, plus a restore command that downloads, decrypts and runs the
  integrity check the storage crate already has. Could even be a script
  with no app change.
- **Gains.** The first backup that survives losing the Mac. A second Mac
  could restore from it. 178 MB a copy: 20 GB holds about a hundred
  uncompressed.
- **Breaks:** nothing. Offline, updater, consent, forecasts unchanged.
- **Does not give:** cloud compute, or anything running with the Mac closed.
- **Owner's chores.** Create the account. Keep the passphrase somewhere
  that is not the Mac (without it the copies are useless, by design). Log
  in now and then so the account is not "abandoned".
- **Size:** one to two days, including a tested restore.

### C. Cloud does the unattended jobs; the Mac keeps the ledger

A small worker on the VM syncs the banks and sends the email; the app pulls
what it gathered.

- **The problem.** One SQLite file cannot be written in two places and
  merged. So either the worker keeps only an inbox of raw bank entries the
  app later applies (then it has no budgets and cannot write the email or
  the alerts), or it gets a full copy pushed up each day (then it is option
  A's exposure with a second moving part).
- **Gains.** Little. The app already syncs within a minute of opening and
  fetches the last 30 days, so nothing is missed today. The only real gain
  is the 20:00 email on days the Mac stays closed.
- **Size:** one to two weeks for the least useful result.

### Not an option: Oracle's Autonomous Database

It would mean rewriting the storage crate from SQLite to Oracle SQL, and
the free one stops itself after 7 idle days. No.

### Worth a look: the same server at home

The README says the data came from `laptopthree` on the home network, which
served the old web UI. If that machine still runs, option A's server could
live there: same code, no third party holding the ledger, no reclamation.

## Security and privacy

This is a real ledger with live bank connections, so plainly:

- **What Oracle would hold.** B: ciphertext only. A and C: the ledger and
  the bank secrets, readable by anyone who gets into the VM or the Oracle
  account. Disk encryption by Oracle protects against a stolen disk, not
  against Oracle or an account takeover.
- **What the bank secrets allow.** As far as the code shows, the consents
  give read access to balances and transactions, not payments. A thief
  could read the accounts until the consent ends or is revoked. The Enable
  Banking private key lets someone ask for new consents in the app's name,
  but each still needs the owner's approval at the bank. Both can be
  revoked: consents at each bank, the key in Enable Banking.
- **Where secrets would live (A, C).** The same 0600 files, in a folder
  owned by a dedicated user on the VM (`FINANCE_SECRETS_DIR`). Oracle's
  Vault is free but adds code for little: whoever owns the VM can read from
  it anyway.
- **Who can reach the API (A).** Nobody from the internet. The devserver
  has no login and can write everything; an open port would hand the ledger
  to the first scanner. The server would listen only on a private network
  (WireGuard or Tailscale), with every inbound port closed in Oracle's
  firewall, SSH included, and a token on top. A public HTTPS endpoint with
  a login is possible but puts a hand-written HTTP server on the internet;
  I would not.
- **In transit.** A: inside the private tunnel. B: HTTPS, and the file is
  already encrypted before it leaves.
- **Backups and getting the data out.** It stays one SQLite file in every
  option. A: the Mac pulls a copy every night and keeps its ten; restoring
  is pointing the app back at the local file. B: download, decrypt, check.
- **What a loss costs.**
  - VM reclaimed or account closed (A): everything since the last pull, at
    most a day of hand edits. Bank transactions download again (30 days).
    Then a rebuild, or simply staying local.
  - Oracle account taken over (A): full read of the ledger and the bank
    accounts until consents are revoked. (B): nothing readable; the copies
    could be deleted, the Mac still has the ledger.
  - Mac lost today, with no change: everything, unless Time Machine or
    similar is running.

**I would not recommend:**

- A with a public port, in any form.
- A or C as the only home of the ledger on a free-only account: no SLA,
  idle reclamation, a 30-day "abandoned" rule and an allowance that was
  halved this year.
- C at all: the most moving parts for the smallest gain.

## Recommendation

**B now. A later, on a copy first, only if the owner wants the Mac-closed
or phone use it buys. Check `laptopthree` before choosing Oracle for A.**

B fixes the weakest point there is today (all copies on one disk), touches
almost no code, breaks nothing, and Oracle never sees a readable byte. It
also means that if A is tried later, an independent copy already exists.

It does not give "compute in the cloud". For one person on one Mac the
compute is not what is short: the app is fast locally and the cloud VM is
slower. What A really buys is "works with the Mac closed" and "open it from
another device". If that is what was meant, A is the way, and it is a real
project.

### First step (reversible, no real ledger)

1. The owner creates the Oracle account and one private bucket. I do not.
2. A script encrypts **the parity golden database, not the ledger** and
   uploads it; a second one downloads, decrypts and runs the integrity
   check. Passing means the round trip is proven.
3. Only then, with the owner's yes, point it at the real backups.

Undo: delete the bucket. About a day of work.

If the owner prefers A, its first step needs no cloud either: build the
server for Linux ARM, run it locally in a container limited to 2 cores and
12 GB against the golden copy, and measure a dashboard load and a forecast
run. That answers "is it fast enough" before any account exists. Two to
three days.

## Open questions for the owner (most important first)

1. **What should this give you?** Use from a phone or second computer;
   sync and the email with the Mac closed; a backup off the Mac; or
   something else? The answer picks the option.
2. **May Oracle hold the ledger readable** (needed for A), or only
   encrypted (B)?
3. **Must the app keep working offline?** If yes, A is out as designed.
4. **Is there a backup off this Mac already** (Time Machine, iCloud)?
5. **Is `laptopthree` still running**, and would you rather host there?
6. **Credit card with Oracle, and Pay As You Go?** Upgrading is said to
   stop idle reclamation (not confirmed by Oracle) but cannot be undone
   and makes real charges possible if something is misconfigured.
7. For A: Tailscale (another account, easiest) or plain WireGuard?
8. Region: Amsterdam or Frankfurt?

## Not verified

- Whether Pay As You Go accounts are exempt from idle reclamation, or keep
  the old 4 OCPU / 24 GB. Community reports only; Oracle's page says the
  new limit applies to "All tenancies".
- What exactly happens to a reclaimed VM's disk.
- What counts as an "idle" account for the 30-day rule.
- Whether the home region can be changed later; ARM capacity in Amsterdam
  or Frankfurt today.
- Oracle's pages disagree on some numbers (boot volume 47 or 50 GB; email
  3,000 a month or 100 a day). Neither matters for this proposal.
- Oracle's contract terms on closing accounts (not read).
- That Oracle encrypts block volumes and buckets at rest by default: from
  memory, not re-read today.
- Speed of the dashboard and of TimesFM on 2 ARM cores without Accelerate.
  Not measured; nothing was built or run.
- That the bank consents are read-only: taken from what the code asks for,
  not checked against Enable Banking's terms. Also whether Enable Banking
  minds the same application calling from a new IP address.
- Whether the daily email's SMTP port is 465/587 in the owner's settings
  (no SMTP password is stored today, so the email looks unused).
- Whether the parity golden holds real data; if it does, the first step
  uses a generated database instead.
