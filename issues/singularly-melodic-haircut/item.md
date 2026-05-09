---
created: 2026-05-09
updated: 2026-05-09
type: improvement
reporter: jari
status: open
priority: normal
epic: hugely-exciting-spiders
related: ['@especially-unruly-crate']
labels: [web-edit-sync, canonical-hash]
---

# Add version-token scheme marker to canonical_hash

## Description

Currently `canonical_hash` produces tokens of the form `sha256:<64hex>`. There is no algorithm/scheme version embedded. When the canonical projection changes (as it did when title was added — see @especially-unruly-crate), every outstanding `expected_version` token becomes invalid and the failure mode is an opaque 409 indistinguishable from a real content conflict.

## Proposal

Domain-separate the hash input and/or prefix the token so a stale token from an older binary fails with a recognisable error rather than masquerading as a content-conflict 409.

Two options (need to pick one in design):

- **Input domain separator**: `h.update(b"issuectl-canonical-v2\n")` before the projection bytes. Keeps token format identical (`sha256:<hex>`); old/new binaries hash differently but the wire format is unchanged.
- **Token prefix**: emit `sha256-v2:<hex>` (or similar). Makes the scheme explicit on the wire — old clients can reject with a clear error. Requires every consumer (web `cardVersion[slug]`, CLI `--expected-version`, agent integrations) to accept the new prefix.

## Why v0.6 (not now)

- Doesn't block v0.5.0 ship — this commit's 409 storm hits clients exactly once on rollout regardless.
- The marker's full payoff comes only when the *next* hash-breaking change lands. Doing it standalone would be its own token-invalidation event, which is the very pain it's trying to reduce.
- **Right time to introduce**: bundle with the next planned canonical-projection change (e.g. `extra` validation, unknown-key policy tightening, or the `serde_jcs` migration if/when that happens).

## Out of scope

- Switching to RFC 8785 JCS serialisation. Pre-existing divergence; tracked separately if it becomes a real interop problem.
- Backwards-compatibility window accepting both old and new tokens — current usage is single-binary local, no need for parallel formats.

## Spin-off context

Discovered during /llm-review of @especially-unruly-crate (commit 0d3b1e2 — adding `title` to canonical_hash). Three reviewers (GPT-5.5, Claude Opus 4.7, partly Gemini 3.1) flagged the missing scheme marker as a forward-looking improvement.
