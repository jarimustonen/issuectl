---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: normal
---

Follow-up from the /llm-review of `pidev-pi-skill-lifecycle` (see
`history/review-pi-skill-lifecycle.md`). All four reviewers flagged that the
global pi provenance manifest (`~/.pi/agent/skills/.issuectl-manifest.json`) is
read-modify-written with no cross-process lock. Two `issuectl skill install`
runs from different repos can interleave and lose a manifest row. The lifecycle
landing added atomic temp+rename writes (no torn/empty file), but not a lock.

**Scope:** acquire an advisory `fs2` lock (matching the repo-wide flock
convention) spanning the pi mirror writes AND the manifest read-modify-write in
`install_skill_summary` / `record_pi_provenance` / `pi_prune`. Note the
pre-existing mirror writes already race unlocked, so this is a pre-existing gap
the manifest merely makes more visible.
