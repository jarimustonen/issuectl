---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: open
priority: high
lane: release-infra
lane_seq: 10
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
