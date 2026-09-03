# ADR 0009: Restricted public research and aggregate-only model tools

- Status: Accepted
- Date: 2026-09-03

## Context

Phase 6 needs selected public context for ambiguous merchants and market
entities, plus a safe way for Codex to inspect the owner's financial picture.
Passing raw bank text or unrestricted database access to a hosted model would
break the local-first privacy boundary. General-purpose URL fetching would also
create SSRF, prompt-injection, and uncontrolled disclosure paths.

## Decision

Add a disabled-by-default retrieval gateway that accepts only two strict domain
requests: an owner-approved public business name, or a validated public market
identifier and enumerated question. The gateway constructs URLs internally,
uses HTTPS exact-host allowlists for English Wikipedia and SEC EDGAR, rejects
non-global DNS answers and unsafe redirects, ignores environment proxies, and
bounds time, redirects, response size, content type, and parsed output.
Merchant evidence requires one exact normalized public-title match; ambiguous
search results abstain instead of asking a model to choose an identity.

Retrieved excerpts are labelled `untrusted_public_web` and carry publisher,
URL, retrieval/expiry times, and a content hash. A separate owner-only SQLite
cache contains only those sanitized public results, can serve explicitly stale
cited evidence during an outage, and can be purged independently. It is
disposable and excluded from durable backups.

Expose a separate, opt-in STDIO MCP server. Its local report tool returns only
documented currency-separated aggregates; raw transactions, source/account/
goal identifiers, merchant text, and database handles are absent from the
schema. The derived-state database is opened with SQLite `mode=ro`; public
research tools accept only the same bounded request contracts.
Every tool is read-only, with no payment, mutation, trading, personalized-advice,
arbitrary URL, SQL, or generic filesystem capability. STDIO avoids a new
listening network service and runs inside the existing gateway container so it
can read the same assistant-derived volume.

## Consequences

The model can explain bounded aggregates and cited public material without
receiving direct ledger access. Connecting this MCP server to Codex still sends
tool inputs and outputs to the configured hosted model; it is therefore an
explicit privacy decision, not a fully local workflow. Qwen/Ollama remains the
local model path.

Exact-host DNS preflight reduces SSRF risk but cannot cryptographically bind the
subsequent HTTP connection to the inspected address. The fixed provider hosts,
no arbitrary URLs, low response limits, and fail-closed parsing constrain the
remaining DNS time-of-check/time-of-use risk. Adding a provider, query field,
model-visible aggregate, write action, remote transport, or self-hosted ingress
requires a new security review and decision.
