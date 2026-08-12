---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: done
priority: normal
closed: 2026-08-12
closed_by: agent-pi-manifest-lock
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

## Resolution

### 2026-08-12T16:35:09Z · @agent-pi-manifest-lock

Locked the pi-corpus provenance manifest RMW: acquire_pi_lock reuses mutate::WriteLock (flock) at <pi_root>/.issuectl/write.lock, held across install_skill_summary's mirror+provenance writes and pi_prune's load→classify→save; record_pi_provenance/save_pi_manifest stay lock-free under the held lock. Two concurrency tests (8-writer lost-update proof + real install||prune serializability). /llm-review 4-model pass: 3 in-scope FIX findings applied (temp-name nonce for test faithfulness, blocking-lock comment accuracy, retired-skill absence assertion); 7 pre-existing concerns staged as spin-offs. Green: cargo test 995 pass, clippy no new warnings, fmt clean.
