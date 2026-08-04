---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: fixed
priority: normal
closed: 2026-08-04
---

# Write-verb errors on a missing slug bubble as `command-failed`, not `not-found`

_Source: /llm-review of json-optional-version change (2026-08-04)_

## Observed

The read paths classify a missing issue precisely:

    $ issuectl --json show does-not-exist
    {"error":{"code":"not-found","message":"issue does-not-exist not found"}}

But the write verbs (`update`/`close`/`set`/…) on a missing slug bubble the
mutate-layer error through `.map_err(|e| anyhow::anyhow!("{e}"))` to `main`,
which renders the generic envelope:

    $ issuectl --json update no-such-issue --status in-progress
    {"error":{"code":"command-failed","message":"issue not found"}}

So a machine caller must string-match `"not found"` on a generic
`command-failed` code instead of branching on a stable `not-found` code — the
same class of machine-ergonomics gap the json-optional-version change was about.
Three review models (Gemini, GPT-5.6, DeepSeek) flagged it.

## Suggested fix

Map `MutateError::NotFound` to the `not-found` error code in the CLI JSON
envelope for all mutation verbs (thread a typed classification through
`main`'s error rendering rather than flattening to anyhow text). Then tighten
`json_error_contract_wraps_bubbled_errors` (or add a sibling) to assert
`error.code == "not-found"` for a write on a missing slug, and reserve the
`command-failed` fixture for a genuinely generic failure.

## Severity

Low — machine-ergonomics, not a correctness bug. Pre-existing; predates the
opt-in-CAS change.
