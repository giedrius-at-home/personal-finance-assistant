# ADR 0017: Retrieve local category examples before model classification

## Status

Accepted

## Context

A small local language model can follow the output schema but may classify an
unfamiliar merchant poorly. Passing the same global sample of Firefly history
to every request also wastes context and may omit the examples most relevant to
the merchant under review. Sending bank descriptions to a hosted vector store
or general web search would violate the local-first privacy boundary.

## Decision

Keep Firefly as the category authority and add optional, local-only retrieval:

- derive a corpus of at most 256 unambiguous, identifier-redacted merchant and
  approved-category pairs from current Firefly history;
- use an explicitly configured Ollama embedding model on the loopback service;
- keep vectors in memory for one suggestion batch and do not persist them;
- retrieve at most 12 examples per merchant, with at most three from any one
  category, before calling the local classifier;
- fall back to the existing balanced examples if embedding is unavailable;
- continue to require owner review before any category is written to Firefly;
- supersede and regenerate pending suggestions when the configured classifier
  changes, without touching already approved Firefly categories.

The default remains retrieval-disabled until
`PFA_OLLAMA_EMBEDDING_MODEL` is set. Public merchant research remains a separate,
owner-triggered and citation-backed action; its output does not enter the local
retrieval corpus automatically.

## Consequences

Suggestions receive more relevant owner-approved examples and remain entirely
local. Each suggestion batch incurs one bounded embedding pass, and operators
must install both configured Ollama models. Retrieval improves context but does
not establish correctness, so confidence controls and owner review remain
mandatory.
