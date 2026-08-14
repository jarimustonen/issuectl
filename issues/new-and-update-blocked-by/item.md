---
created: 2026-08-12
updated: 2026-08-14
type: feature
status: wontfix
priority: normal
labels: [cli]
closed: 2026-08-14
closed_by: jari
---

# Set blocked_by at creation (new --blocked-by) + light update --add-blocked-by

## Description

Setting a dependency currently requires either a full versioned `apply` YAML patch or hand-editing `blocked_by:` frontmatter — there is no lightweight path.

Observed this session (bootstrapping a new repo's dependency-ordered backlog, 5 issues):
- `issuectl new --slug X --type feature --blocked-by Y` → `error: unexpected argument '--blocked-by' found`.
- `issuectl update X --add-blocked-by Y` → no such flag (only `--lane` / `--no-lane` exist for dag metadata).
- So deps had to be set by hand-editing frontmatter (fragile) or by assembling a versioned `apply` patch (heavy for a one-field add).

Request:
1. `issuectl new --blocked-by <slug>` (repeatable) — set deps at creation, so a backlog can be filed with its DAG in one pass.
2. `issuectl update <slug> --add-blocked-by <slug>` / `--remove-blocked-by <slug>` (repeatable) — the same single-field convenience the other dag fields (`--lane`) already have.

Both should validate the target resolves to a real issue and reject self/cycle deps (same checks `issuectl dag` relies on). Consider mirroring for `--related` at creation if not already present.

## Comments

### 2026-08-12T15:09:48Z · @jari

RE-SCOPE (2026-08-12): the 'update --add-blocked-by' half is effectively DONE — commit 6e95b07 landed a 'depend add/remove <slug> --blocked-by' subcommand giving the same lightweight dep-set-without-apply-patch convenience. Remaining scope = ONLY 'new --blocked-by' at CREATION (file an issue with its deps in one pass; depend-add requires the issue to already exist). Consider closing if the creation-time sliver isn't worth it.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: the 'update --add-blocked-by' need is already met by the landed 'depend add/remove'; the remaining creation-time 'new --blocked-by' sliver is marginal.
