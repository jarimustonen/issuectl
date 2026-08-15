---
created: 2026-08-10
updated: 2026-08-15
type: bug
status: open
priority: high
lane: blocked-upstream
lane_seq: 90
labels:
- blocked:upstream-ossctl
---

# ossctl release cut does not actually publish to crates.io (real-cut no-op); use manual cargo publish until fixed

## Description

## Symptom

`ossctl release cut` (the newly-wired release path from
`@wire-oss-release-as-release-path`) **does not actually publish to
crates.io** on a real cut. Observed while cutting **0.8.1** (2026-08-10, the
first real release on this path):

- Phases ran `dry_run: ok → build: ok → publish: failed`.
- Publish failed with: *"`issuectl-core@0.8.1` was not visible on the registry
  index within 300s; a crate that depends on it cannot be published until it
  is."*
- After the failure, `issuectl-core@0.8.1` was polled for **9 minutes**: the
  crates.io API returned **404** and the sparse index never showed it. So it
  was never uploaded — the 300s "index visibility" wait was waiting for
  something that was never published.
- `ossctl release verify <run>` reported **no publish receipt** for either
  target.

## Root-cause hypothesis

An index-visibility wait implies ossctl believed `cargo publish -p
issuectl-core` succeeded (exit 0) — yet nothing landed on crates.io. The
signature (cargo "success" + no upload) matches the publish adapter running
the real-cut publish as a **`--dry-run`** (or otherwise not uploading).
`@wire-oss-release-as-release-path` only ever verified via `ossctl release
plan` (which is *all* dry-run), so a real-publish defect would not have been
caught. This is most likely an **upstream ossctl bug** (its cargo-publish
adapter), manifesting in this repo's release path.

## What actually happened / workaround

0.8.1 was shipped via the **proven manual fallback** (every release ≤0.7.2
used it):
- `cargo publish -p issuectl-core` → uploaded + confirmed available.
- `cargo publish -p issuectl` → uploaded + confirmed available.
- `git tag -a v0.8.1 -m "Release 0.8.1"` + `git push origin main --follow-tags`
  → fired cargo-dist `release.yml` for binaries + Homebrew.
Manual `cargo publish` worked **instantly** for both crates, confirming the
crate itself is healthy and the defect is in ossctl's publish step.

## Impact

The documented "release path is ossctl" (AGENTS.md operating facts) is **not
actually usable for real releases** until this is fixed. Until then, releases
must use the manual `cargo publish` + tag path (see the AGENTS.md caveat added
alongside this issue). Note `publish-crates.yml` was retired by
`@wire-oss-release-as-release-path`, so there is no CI crates.io path either —
the manual local publish is the only working route right now.

## What a fix touches

- Upstream: ossctl's cargo-publish adapter (confirm it does a *real* publish on
  `release cut`, not a dry-run; add an integration test that asserts a version
  actually appears on the registry).
- Here: once ossctl is fixed and re-verified with a REAL low-stakes cut, remove
  the manual-fallback caveat from AGENTS.md and re-point releases at `ossctl
  release cut`.

## Acceptance Criteria

- [ ] Root cause confirmed in ossctl (real cut published a dry-run / no-op).
- [ ] ossctl fix released; a real `ossctl release cut` publishes a version that
      becomes visible on crates.io.
- [ ] AGENTS.md manual-fallback caveat removed; release path re-pointed at ossctl.

## Comments

### 2026-08-10T14:42:30Z · @agent-claude

Root-cause fix tracked upstream in the ossctl repo: @release-cut-publish-noop (bug, high). The stale-lock recovery friction hit during the same cut is upstream @release-abandon-break-stale-lock. This issuectl-side issue stays as the release-path blocker: once ossctl ships the fix and a real cut is verified end-to-end, remove the reminder and confirm the ossctl path here.
