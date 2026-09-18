---
created: 2026-09-16
updated: 2026-09-18
type: bug
reporter: jari
status: open
priority: normal
provenance: other
provenance_detail: Observed during 3DBear issuectl doctor cleanup
source_ref: taskfleet:01m2mezzw40464813xn8d8zw56/observed:homebrew-intel-prefix-macos
originating_run: 01m2mezzw40464813xn8d8zw56
originating_run_kind: spinoff
lane: release-infra
---

# Homebrew formula silently skips Intel-prefix macOS installs

## Description

## Description

The published Homebrew formula has no macOS x86_64 branch. On an Apple Silicon Mac whose Homebrew installation runs from the Intel `/usr/local` prefix, Homebrew evaluates `Hardware::CPU.arm?` as false. The formula then selects no `url`, so `brew outdated issuectl` reports nothing and leaves the installed version stale without explaining why.

This occurred on Pekka's Apple Silicon Mac: issuectl remained at 0.6.4 while 0.18.1 was current. Uninstalling the formula and using the release installer installed the arm64 binary successfully.

The problem remains in the published 0.18.4 formula as of 2026-09-16. Its macOS URL is guarded only by:

```ruby
if OS.mac? && Hardware::CPU.arm?
```

Source checked: <https://raw.githubusercontent.com/jarimustonen/homebrew-issuectl/main/Formula/issuectl.rb>

## Expected behavior

The formula should either support macOS x86_64/Rosetta or fail with an explicit architecture requirement. Homebrew should not silently retain an obsolete issuectl version.

## Quick Test

Run Homebrew through an Intel-prefix/Rosetta installation on Apple Silicon and verify that `brew install` or `brew upgrade jarimustonen/issuectl/issuectl` either installs a supported artifact or reports a clear architecture error.
