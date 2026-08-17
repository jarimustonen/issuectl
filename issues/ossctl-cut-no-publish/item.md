---
created: 2026-08-10
updated: 2026-08-17
type: bug
status: open
priority: high
lane: release-infra
lane_seq: 10
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

### 2026-08-17T05:48:42Z · @claude-stint

REOPENED as a verification gate for the next release (maintainer decision, 2026-08-17).

Scope note, because the title is now narrower than the issue: the ORIGINAL symptom — 'release cut does not actually publish' — is genuinely fixed and was verified on 0.14.1 (both crates published by the engine, correct core-before-binary order). What keeps this open is that the engine path as a whole is not yet trustworthy end-to-end: two other 0.6.1 defects were hit on the same cut, and both were worked around by hand rather than by a working engine.

Do NOT close this on the basis of a green 'release complete' line. That line was printed on 0.14.1 while the entire binary + Homebrew half had silently failed.

#### Verify on the next cut, then close if all clean

1. Publish phase (the original symptom): the engine publishes issuectl-core then issuectl to crates.io, no 'not visible on index' failure, no manual 'cargo publish' fallback needed.
2. ossctl @release-bump-plan-uncuttable: run 'ossctl release plan --bump patch' and cut the plan_id it returns. If the cut still answers plan_stale, the bug is unfixed — fall back to bumping by hand and planning without --bump. Never follow the suggested current_plan_id; on 0.14.1 it meant 'republish the already-published version'.
3. ossctl @release-tag-preempts-cargo-dist: after the cut, check the GitHub Release actually has assets (expect ~15, not 0) and that publish-homebrew-formula ran rather than being skipped. If host failed with 'a release with the same tag name already exists', delete the asset-less release object (leave the git tag) and re-run the failed jobs.
4. Homebrew tap actually advanced to the new version — this is the leg that was silently dropped for three releases.

Close only when 1-4 all pass with no manual intervention. If any step still needs a hand-fix, record which one here and leave it open: a partially-working release engine that reports success is the exact failure mode this issue exists to catch.

Upstream issues to check for fixes before the next cut: ossctl @release-bump-plan-uncuttable, @release-tag-preempts-cargo-dist.

### 2026-08-17T08:31:59Z · @agent-stint

ossctl upgraded to 0.7.0 and both cut gotchas are fixed upstream (release-bump-plan-uncuttable, release-tag-preempts-cargo-dist closed). OSS-RELEASE.md now declares the distribution block (adapter cargo-dist, gh_releases, homebrew_tap jarimustonen/homebrew-issuectl, 3 platforms) plus a registry:homebrew target, so 0.7.0 plans AND verifies the delegated legs — the mandatory verify barrier checks crates.io receipts, GitHub Release assets, and the tap formula before a cut reports complete. Gate for the next cut: run it on ossctl >=0.7.0, confirm the verify phase passes; the manual gh-release-view checklist remains as a backstop only.



## Resolution

### 2026-08-17T05:43:22Z · @claude-stint

Fixed upstream in ossctl 0.6.1 (commit 2846d66) and verified on this repo's 0.14.1 cut.

The reported symptom was that 'ossctl release cut' did not actually publish: the publish phase failed with 'core not visible on index within 300s' and nothing was uploaded, forcing the manual 'cargo publish -p issuectl-core' -> wait -> '-p issuectl' -> tag fallback used for 0.12.0, 0.13.0 and 0.14.0.

On 0.14.1 the engine path published for real:

  -> publish
    published: rust:issuectl-core@0.14.1
    published: rust:issuectl@0.14.1
  ✓ publish complete
  -> tag
    tag created / pushed: v0.14.1

crates.io now serves both crates at 0.14.1 in the correct core-before-binary order, so the manual publish fallback documented in TODO.md is no longer required.

Two SEPARATE 0.6.1 defects were hit during the same cut and are filed upstream rather than here, since neither is this repo's bug:

- ossctl @release-bump-plan-uncuttable — 'release plan --bump' seals a plan that 'release cut' always rejects as stale, and the rejection suggests a current_plan_id that means 'republish the version already on the registry'. Following that suggestion attempted a republish of 0.14.0; only ossctl's byte-identity guard stopped it. Nothing landed. Workaround: bump by hand, then plan without --bump.
- ossctl @release-tag-preempts-cargo-dist — the tag phase pre-creates the GitHub Release, so cargo-dist's host job collides and the Homebrew publish is skipped while the cut still reports success.

Neither blocks the publish path this issue was about, but the first means '--bump' must be avoided and the second means a cut must be checked for release assets afterwards. TODO.md's release-lore section should be updated to reflect that the engine path now works, with those two caveats.

## Reopen Notes — 2026-08-17

_Add rationale for reopening here._
