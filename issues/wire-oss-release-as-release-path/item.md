---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: normal
labels: [deferred]
---

# Wire /oss-release as the release path (integrate ossctl release engine with cargo-dist)

_Source: OSS-RELEASE.md + .github/workflows/release.yml + publish-crates.yml_

## Description

**BLOCKED on ossctl** — upstream ossctl changes are in progress to model a cargo-dist-style release (see the two issues filed 2026-08-04 in the ossctl repo: maturity pre-1.0 gate + cargo-dist multi-output modeling). Do this once ossctl ships that support. Labeled `deferred` until then.

## Goal
Make `/oss-release` (→ `ossctl release plan|cut`) the actual release path for this repo, replacing the current manual bump/CHANGELOG/tag steps, using the approved `OSS-RELEASE.md` contract.

## Current state (why it isn't plug-and-play)
Release work is split across three places, all tag-triggered or manual:
- `.github/workflows/release.yml` (**cargo-dist**) → GitHub-Release binaries + shell installer + Homebrew formula pushed to `jarimustonen/homebrew-issuectl`.
- `.github/workflows/publish-crates.yml` → crates.io publish (both crates, `issuectl-core` before `issuectl`).
- Manual: version bump + CHANGELOG `[Unreleased]` move + `release: X.Y.Z` commit + `git push --follow-tags`.

`ossctl release cut` reads the contract and does bump + CHANGELOG + tag + **crates.io publish** (adapter `cargo-publish`).

**Correction (2026-08-04) — the current crates.io auto-publish is actually BROKEN, which strengthens the case for moving it into ossctl.** `publish-crates.yml`'s trigger is `release: types: [published]` (NOT the tag directly). But cargo-dist's `release.yml` publishes the GitHub Release using the default `GITHUB_TOKEN`, and GitHub does **not** fire downstream workflows from `GITHUB_TOKEN`-authored events (recursion guard). So `publish-crates.yml` does **not** auto-run on a release — confirmed: its only runs on record are old manual `workflow_dispatch` ones; it did not fire for v0.6.5 or v0.6.6. crates.io currently requires a **manual** `gh workflow run publish-crates.yml` after every release. Moving the crates.io publish into `ossctl release cut` (and retiring `publish-crates.yml`) fixes this standing gap rather than colliding with it. (Also: `AGENTS.md` "Operating facts" wrongly states the tag triggers `publish-crates.yml` — correct it as part of this work.)

## Integration decision (recommended division of labor)
- **ossctl release owns:** version bump + CHANGELOG finalize + git tag + crates.io publish.
- **Retire `publish-crates.yml`** — its job moves into ossctl (removes the double-publish).
- **Keep cargo-dist `release.yml`** as a pure binary-distribution backend (binaries + installer + Homebrew tap), still triggered by the tag ossctl pushes; verify it does not itself publish to crates.io.

## First step when unblocked
Run `ossctl release plan` as a dry-run to see exactly what the engine would do here (it seals a content-addressed plan), reconcile against the three workflows above, then decide/execute the retirement of `publish-crates.yml`.

## Notes
- The `OSS-RELEASE.md` contract is already `approved` (maturity mvp). Its `## Rationale`/`## Release notes` currently say 'keep cargo-dist; /oss-release-cut must not regenerate release.yml' — that framing was a guardrail and should be revisited/relaxed as part of this work, since the goal is now to USE /oss-release.
