---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: low
related: ['@pi-corpus-metadata-error-misclass']
closed: 2026-08-14
closed_by: jari
---

# Manifest-owned entry replaced by a symlink hides from has_findings (Unmanaged)

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-corpus-metadata-error-misclass (OpenAI).

`classify_pi_corpus` reports a symlinked entry dir as `Unmanaged` (deliberate, from the symlink-traversal threat model — never read through). But when the entry is issuectl-OWNED (has a manifest row / `recorded_version`), `Unmanaged` is excluded from `PiStatusReport::has_findings()`. So a corpus whose only anomaly is an owned path replaced by a symlink can return `has_findings()==false` — a serious ownership/filesystem inconsistency is suppressed from the actionable summary. Prune correctly leaves the row alone, so this is NOT provenance loss, only a visibility gap.

Fix: for a manifest-owned entry-dir symlink, use an actionable state (e.g. a new `UnsafePath`, or reuse `Inaccessible` — though the symlink was inspected successfully, so a dedicated state is cleaner) so `has_findings()` surfaces it. Keep unowned symlinked dirs as `Unmanaged`. Add a test: owned symlinked entry ⇒ `has_findings()==true` and still never pruned. Note interaction with the symlink threat-model tests around skill.rs:2553+.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: niche status-visibility gap (owned entry swapped for a symlink hides from has_findings). Not data loss; low value.
