---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: wontfix
priority: normal
closed: 2026-08-04
---

# Decide whether `apply --json` should join the opt-in --expected-version model

_Source: /llm-review of json-optional-version change (2026-08-04)_

## Context

When `--expected-version` was made **optional** (opt-in compare-and-swap) on
`--json` writes for the single-field verbs (`update`/`close`/`set`/`note`/`check`/
`label`/`depend`/`body set`, superseding design D4=B), the transactional `apply`
verb was **deliberately left out of scope**: `apply --json` still requires a
non-empty `expected_version:` in the patch file.

All four review models (Gemini, GPT-5.6, Opus, DeepSeek) flagged this as a DX
inconsistency: an agent that learns "under `--json`, CAS is opt-in" will hit
`apply` and get the exact `exit 1` trap the change set out to remove — for the
one verb agents most often use for multi-field mutations.

## The decision to make

Pick one and record it in `docs/design/web-edit-sync.md` (the D4 entry):

1. **Make `apply` consistent** — `expected_version:` optional in the patch,
   honored when non-empty (drop the `--json` parse-time requirement in
   `parse_apply_patch`, invert the `parse_apply_patch_*_under_json` tests).
2. **Keep `apply` strict** — but justify it with a reason distinct from the
   (now-rejected) D4=B reasoning. The current working justification: a
   multi-field patch assembled from an earlier `show` is the read-modify-write
   shape most exposed to lost updates.

The change already signposts the exception in `apply --help` and the skill
template, so callers aren't surprised — but the underlying inconsistency stands
until this is decided.

## Comments

### 2026-08-04T14:35:03Z · @jari

DECIDED 2026-08-04: option 2 (keep apply --json strict). apply still requires non-empty expected_version — deliberate exception to D4's opt-in relaxation, justified because a multi-field apply patch (assembled from an earlier show) is the read-modify-write shape most exposed to lost updates. No code change; behaviour already correct + signposted in apply --help and the skill template. Recorded as D4a in docs/design/web-edit-sync.md. Closing wontfix = decided not to make apply consistent.
