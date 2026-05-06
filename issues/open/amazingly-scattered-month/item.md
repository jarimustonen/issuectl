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

## Inconsistent states the reconciler must handle

(Expanded from focused review pass 2 — these are the states `locate_issue`/the reconciler must define behavior for. Without these rules `locate_issue(slug)` becomes nondeterministic.)

| State on disk | Likely cause | Reconciler action |
| --- | --- | --- |
| `closed/<slug>` with active `status:` (open/in-progress/testing) | crash after rename before write; manual edit | rewrite `status: done`; if `closed:` absent, set to today; emit `LoadWarning` |
| `open/<slug>` with closing `status:` (done/fixed/wontfix/...) | external writer wrote file before rename; manual edit | rewrite `status: open`; remove `closed:`; emit `LoadWarning` |
| Both `open/<slug>` and `closed/<slug>` exist | failed external rename target; merge conflict | DO NOT pick a side; emit `LoadWarning code: "ambiguous_slug"`; `locate_issue` returns `Err(AmbiguousSlug)` until human resolves |
| `<folder>/<slug>/` exists without `item.md` | partial external write; `mkdir` without write | emit `LoadWarning`; exclude from listings |
| `item.md` is invalid YAML | merge conflict; partial editor save | emit `IssueInvalid` event; never auto-rewrite |
| `item.md` contains git merge markers (`<<<<<<<`, `=======`, `>>>>>>>`) | unresolved `git merge` | emit `LoadWarning` with prominent error; do not auto-fix |
| `closed/<slug>` with closing status but no `closed:` date | manual close via vim | with `--fix`: synthesize `closed:` from directory mtime if available, else today; warn |
| `open/<slug>` with stale `closed:` date | reopened via folder move only | with `--fix`: remove `closed:`; warn |
| Symlinked issue dir (e.g. `issues/open/foo` → external path) | filesystem-level escape attempt | refuse to follow (existing `repo::locate_issue` check); emit `LoadWarning code: "symlink_escape"` |
| Orphan `.issuectl-tmp-*` files in `issues/**` | SIGKILL during atomic write | with `--fix`: delete; without: report count |

The reconciler runs in two modes:

- `issuectl doctor` (read-only): walk and report all findings, exit 0.
- `issuectl doctor --fix`: apply the actions above for self-healing
  cases; never auto-fix `IssueInvalid` or `ambiguous_slug` (require
  human attention).

Web server runs the read-only path on every `serve` startup before
opening the watcher, so transient warnings appear in `#warnings`
immediately and don't propagate as misleading `IssueUpserted` events.

Depends on the M1 mutation protocol (flock, canonical hash) being stable. Implement after M1 ships.
