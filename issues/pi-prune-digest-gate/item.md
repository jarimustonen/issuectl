---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: normal
closed: 2026-08-14
closed_by: jari
---

Follow-up from the /llm-review of `pidev-pi-skill-lifecycle` (see
`history/review-pi-skill-lifecycle.md`). OpenAI + Anthropic recommended the
strongest deletion-safety design: record a `content_sha256` in each
`PiManifestEntry` (schema v2) and, before `pi-prune` deletes an orphan, compare
the current regular-file bytes to the recorded digest. A mismatch → a distinct
`OrphanModified` state requiring manual action; legacy rows without a digest are
never auto-deleted.

This closes the residual window that the shipped fixes narrow but don't fully
eliminate: issuectl writes a copy, the user later edits it, the skill is then
retired → prune would currently delete the user-modified file. Requires a
manifest schema bump + migration.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: extra content-digest safety gate on top of the already-fixed metadata-misclass data-loss path. Marginal belt-and-suspenders; not worth it.
