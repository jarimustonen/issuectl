---
title: "pi.dev skill lifecycle: doctor drift-detection + prune for global ~/.pi mirrors"
type: feature
priority: normal
status: open
slug: pidev-pi-skill-lifecycle
---

## Context

`issuectl skill install` / `issuectl init` now dual-home Claude skills into the
home-global pi.dev corpus `~/.pi/agent/skills/<name>/SKILL.md` (see
`pidev-dual-home-skills`, WS4 of the pidev-migration epic). Unlike the
repo-local Claude/Codex targets, these pi copies are **global and unmanaged**:
nothing tracks, verifies, or removes them. This is a deliberate scope cut for
the initial dual-home landing; the follow-up lifecycle work is captured here.
The sibling binary `orchestratectl` filed an identically-scoped issue of the
same name.

## Problems to address (surfaced by the /llm-review panel on the dual-home diff)

- **Version drift.** A non-force install leaves an existing pi copy in place,
  so a pi mirror written by an older issuectl stays stale after a newer binary
  installs the repo-local skill (the accepted trade-off documented in
  `skill.rs`). Any repo's `--force` install silently rewrites the *global* pi
  copy to that binary's version. There is no way to see or reconcile the pi
  corpus version.
- **No prune.** Renamed/removed skills (e.g. the deprecated `/triage-bugs`)
  leave orphaned entries in `~/.pi/agent/skills/` with no tooling to clean
  them.
- **No verify.** `issuectl doctor` does not inspect the pi corpus at all, so
  drift or corruption there is invisible.
- **Uninstall gap.** There is no `skill uninstall`; if one is added, it must
  decide what to do with the shared global pi copy (likely can't be reference-
  counted across repos).

## Sketch

- Add out-of-band provenance to pi copies (a marker or a manifest under
  `~/.pi/agent/skills/`) so prune/drift can distinguish issuectl-owned entries
  from hand-authored ones — the same mechanism orchestratectl needs.
- `issuectl doctor` (or a dedicated `skill pi-status` / `skill pi-prune`):
  detect version drift vs the running binary, list orphans, offer `--fix`.
- Decide the reconciliation policy: overwrite-only-if-newer vs. always-on-force.

## Out of scope

The dual-home write itself (done in `pidev-dual-home-skills`). This issue is
purely the lifecycle/observability layer on top.
