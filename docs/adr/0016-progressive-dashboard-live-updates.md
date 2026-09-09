# ADR 0016: Add progressive dashboard live updates

- Status: Accepted
- Date: 2026-09-08

## Context

ADR 0008 deliberately began with server-rendered pages and no client JavaScript.
The dashboard now receives daily and post-import Firefly synchronization and has
frequent category-review actions. Requiring a full browser reload after every
review makes the local workflow unnecessarily slow and can leave balances or
reports visibly stale.

## Decision

Keep server-rendered HTML and the existing typed JSON contract authoritative,
but add one small, first-party progressive-enhancement script. Every 20 seconds,
and when the page becomes visible, it fetches a fresh server-rendered dashboard
and replaces only the main dashboard region. It pauses while the owner is editing
a field or another request is active.

Intercept only mutations that already return directly to the dashboard:
category suggestion, approval, correction, rejection, and policy rollback. Send
the unchanged form through `fetch`, including its CSRF token and same-origin
cookie, then replace the dashboard region with the server response. CSV and
settings preview/confirmation flows retain normal navigation because the owner
must inspect a separate exact preview.

Show an accessible fixed confirmation or failure toast after each live action so
the result remains visible at the owner's current scroll position. Category
review cards join each proposal to its exact canonical transaction and show the
full description, signed amount, currency, and date. This sensitive context is
limited to the loopback owner dashboard and is not logged or sent to a third
party.

Serve the script locally, permit scripts only from `'self'` in CSP, use no
third-party assets, retain no financial data in browser storage, and leave all
calculations and authorization on the server. Without JavaScript, every form and
page continues to work through its original navigation path.

## Consequences

Balances, reports, and review state update without whole-page reloads. Review
actions feel immediate while preserving the same server-side validation,
same-origin rules, and audit behavior. The browser performs a small local request
every 20 seconds while the visible dashboard is idle.

ADR 0008 remains authoritative for server rendering, deterministic contracts,
loopback binding, and first-party assets; this decision supersedes only its
no-client-JavaScript and full-refresh consequences.
