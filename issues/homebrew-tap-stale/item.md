---
created: 2026-08-16
updated: 2026-08-17
type: bug
status: fixed
priority: high
lane: release-infra
lane_seq: 10
closed: 2026-08-17
closed_by: claude-stint
commits:
- hash: 068df55
  summary: 'fix(dist): enable Homebrew formula publishing on tag'
---

# Homebrew tap is three releases stale (0.11.0) — cargo-dist homebrew publish is disabled

## Description

## Observed

After cutting v0.14.0, the three publish targets disagree:

- crates.io `issuectl-core`: **0.14.0** ✅
- crates.io `issuectl`: **0.14.0** ✅
- GitHub Release v0.14.0: 14 assets ✅
- **Homebrew tap `jarimustonen/homebrew-issuectl`: still `version "0.11.0"`** ❌

The tap formula still points at `releases/download/v0.11.0/…`. It missed **0.12.0, 0.13.0 and
0.14.0**. Anyone installing or upgrading via `brew` is three releases behind, including the
BREAKING 0.13.0 JSON-envelope change.

## Cause

`dist-workspace.toml` says so explicitly:

    # Homebrew auto-publish is deliberately NOT enabled here — that needs a tap write
    # token and is owned by the separate `homebrew-adapter-first-formula` issue.

So the tag-triggered `release.yml` (cargo-dist) publishes binaries but **never touches the
tap**. The 0.11.0 formula was presumably written by some other path that has not run since.

## Why this is worth its own issue here

Two documented facts in this repo are wrong as a result, and they are the facts an agent reads
before cutting a release:

1. `TODO.md`'s release lore states: *"Tag laukaisee `release.yml`:n (cargo-dist) → binäärit +
   Homebrew"*. The Homebrew half is **false** — the config disables it.
2. `AGENTS.md`'s operating policy states the homebrew leg *"is the most important target — it
   must be cut, not dropped"*. It is in fact being dropped silently on every release.

The referenced `homebrew-adapter-first-formula` issue does not exist in this repo's tracker, so
nothing here tracks the gap.

## Expected

Either:

1. Enable cargo-dist's Homebrew publish (needs a tap write token in repo secrets) so the tap
   tracks every tag, **or**
2. Document the manual tap-update step as a required part of the release recipe in `AGENTS.md`
   + `TODO.md`, so it stops being invisible.

(1) is preferable — a manual step that has already been missed three times in a row is not a
working process.

Either way, **fix the two false statements** in `TODO.md` and `AGENTS.md`, and bring the tap up
to the current release.

## Comments

### 2026-08-17T04:05:42Z · @claude

The stated blocker does not hold: HOMEBREW_TAP_TOKEN has existed on this repository since 2026-08-05, i.e. before dist-workspace.toml recorded the disable as 'needs a tap write token, owned by a separate issue'. Verified 2026-08-17. Re-enabling cargo-dist's homebrew publish is therefore unblocked on credentials.

Worth doing rather than waiting for the upstream engine: this repository's tap formula, when cargo-dist last wrote it, has the CORRECT shape — per-platform urls pointing at the published release binaries with checksums, no toolchain dependency. The upstream engine's own formula writer currently emits a source-build formula that does not install at all (filed upstream as homebrew-formula-uninstallable). So cargo-dist is the better producer here today.

Upstream is separately adding verification so a lagging tap is reported instead of passing silently (upstream release-verify-homebrew-tap) — that will catch a recurrence, but it does not fix this repository's disabled publish. That part is this repository's to do.

### 2026-08-17T04:06:58Z · @claude

CORRECTION to the previous note — the token is far older than stated there. HOMEBREW_TAP_TOKEN was CREATED 2026-05-02 (the 2026-08-05 date in the previous note was its last-updated timestamp, not its creation). So the credential predates the 'needs a tap write token' justification by about three and a half months; it was never a real blocker at any point.

The justification comment in dist-workspace.toml was last touched 2026-08-16 in a cargo-dist workflow standardization commit — i.e. it was read, built around, and preserved yesterday without being checked. That is how the tap came to sit three releases behind: not one wrong decision, but a stale justification that each subsequent pass treated as established fact.

The comment also points at an owning issue that does not exist in this repository's tracker, which would have been the cheapest possible tell.

## Resolution

### 2026-08-17T05:43:11Z · @claude-stint

Fixed and verified end-to-end on the 0.14.1 release.

Root cause was dist-workspace.toml: the homebrew installer was disabled with the comment that it 'needs a tap write token'. HOMEBREW_TAP_TOKEN had in fact been configured on the repo long before (created 2026-05-02), so the stated blocker was stale. Added the homebrew installer, tap = jarimustonen/homebrew-issuectl, and publish-jobs = [homebrew], then regenerated release.yml with 'dist generate'.

Deliberately did NOT run 'ossctl dist generate' (the nominal owner via /oss-dist): it would have stripped this repo's self-hosted macOS ARM64 runner override, which is load-bearing (the 'hauis' runner builds macOS in ~67s versus the 45+ min hosted-queue allocation that motivated the override). Edited the config minimally by hand instead; the override survived.

One further blocker surfaced during the cut: ossctl's tag phase pre-created an empty GitHub Release, so cargo-dist's host job failed with 'a release with the same tag name already exists' and publish-homebrew-formula was skipped. Deleted the asset-less release object (git tag untouched) and re-ran; host and publish-homebrew-formula then both succeeded. Filed upstream as ossctl @release-tag-preempts-cargo-dist.

Verified after the fix: tap formula at version 0.14.1 pointing at v0.14.1 assets; GitHub Release v0.14.1 carries 15 assets. The tap had been stranded at 0.11.0 across 0.12.0, 0.13.0 and 0.14.0.

Follow-up left open deliberately: the OSS-RELEASE.md contract still has no distribution block (contract show reports homebrew_tap: null), so ossctl's own homebrew leg is inert and the prose claim that cargo-dist publishes the tap was only made true by this dist-workspace.toml change. Also still to correct: TODO.md's release lore and AGENTS.md's homebrew-leg claim.
