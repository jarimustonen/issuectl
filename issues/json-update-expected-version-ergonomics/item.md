---
created: 2026-07-29
updated: 2026-07-29
type: feature
reporter: jari
status: open
priority: normal
---

# scripted --json update: --expected-version round-trip is unobvious

## Description

## Observed

Using `issuectl --json update <slug> ...` in an automated (agent/script) flow requires `--expected-version`. Without it:

    {"error":{"code":"command-failed","message":"--expected-version is required with --json (per design D4=B); fetch with `issuectl show <slug> --json`"}}

The message *does* name the fix, but the round-trip is more awkward than it needs to be for scripted callers:

1. Must first run `issuectl show <slug> --json` and parse out the version.
2. The version key location is not obvious — in my output it was nested under `item.version`, not a top-level `version`. A defensive parser (`(d.get("item") or d)["version"]`) was needed to handle both shapes.
3. Then a second call passes `--expected-version "<sha256:v1:...>"`.

So the minimal "update one field with --json" is three steps + shape-guessing, and each subsequent `--add-commit` in a loop needs a fresh version fetch because the prior write bumped it.

## Feature idea (any one helps)

- `--expected-version latest` (or `--force-version`) — opt into atomic read-current-then-write in a single `update` call, for callers that accept last-writer-wins.
- Stable, documented JSON shape for `show --json` version location (top-level `version` mirror, or documented `item.version`) so callers do not need `(d.get("item") or d)`.
- Echo the current version in the error payload — the "expected-version required" error could include `"currentVersion": "sha256:v1:..."` so a caller retries immediately without a second `show` round-trip.

## Impact

Low severity — the guardrail is intentional (D4=B) and the error message is helpful. Scripted-ergonomics improvement, not a bug. Encountered while batch-logging four commits to one issue from an agent session (3dbear-monorepo docs restructuring, 2026-07-29).
