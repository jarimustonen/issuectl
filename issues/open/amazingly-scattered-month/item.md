---
created: 2026-05-06
updated: 2026-05-06
type: task
status: open
priority: normal
---

# Startup reconciliation: extend issuectl doctor to repair status/folder mismatches and orphan tempfiles

## Description

Spin-off from web-edit-sync design (docs/design/web-edit-sync.md §11, §3.4).

When 'issuectl serve' starts, an external writer or a prior crash may have left the issues/ tree in an inconsistent state. The design's mutation protocol (rename-then-write for status changes) means a crash between the rename and the content write leaves a renamed directory with stale frontmatter content.

Extend 'issuectl doctor' (and 'doctor --fix') to detect and repair:

- Status/folder mismatches: closed/foo with active 'status:' (or open/foo with closing status). Authoritative rule (DISCUSS #19): directory wins. With --fix, rewrite frontmatter to a sane default ('done' for active-in-closed, 'open' for closing-in-open) and emit LoadWarning. Without --fix, just diagnose.
- Orphan '.issuectl-tmp-*' files inside issues/**: remove them (atomic-write tempfiles that survived SIGKILL).
- Git merge conflict markers ('<<<<<<<', '=======', '>>>>>>>') in item.md: log a LoadWarning, never auto-fix — user decides.
- Missing item.md inside an issue dir, symlinked issue dirs (existing checks), duplicate slugs across open/closed.

Reuses existing repo::LoadWarning shape so the web UI surfaces these in the existing #warnings strip without new UI.

Depends on the M1 mutation protocol (flock, canonical hash) being stable. Implement after M1 ships.
