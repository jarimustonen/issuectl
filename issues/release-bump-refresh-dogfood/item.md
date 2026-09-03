---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: in-progress
priority: normal
provenance: agent:issuectl-wrapup
source_ref: agent:issuectl-wrapup/2026-09-03/release-bump-dogfood
lane: release-automation
collision: [OSS-RELEASE.md]
---

# Release bump leaves dogfooded skills stale

## Description

The engine-owned release bump updates the workspace version and finalizes the changelog, but leaves the repository's six tracked dogfooded agent-instruction copies rendered with the previous issuectl version.

The affected copies are the Claude and Codex variants of `/issue`, `/issue-new`, and `/issue-intake` under `.claude/skills/` and `.codex/prompts/`.

## Observed

During the v0.17.1 release, Shipshape changed the workspace version from 0.17.0 to 0.17.1. The published binary correctly embedded 0.17.1, but the tracked dogfooded copies still said 0.17.0. This left the release commit inconsistent with `skill::tests::dogfooded_copies_match_templates` until a separate post-release housekeeping commit regenerated all six files.

The copies were safely refreshed with issuectl 0.17.1 in an isolated HOME. The resulting diff contained only rendered version substitutions.

## Expected

The release bump should regenerate the six dogfooded copies in an isolated environment as part of the same release mutation, before the release commit is sealed and validated. The release commit should therefore be self-consistent and pass the dogfood invariant without a follow-up commit or writes to the operator's global skill directories.

## Acceptance Criteria

- The issuectl release configuration or bump hook regenerates all six tracked dogfooded copies during an engine-owned version bump.
- Generation runs with an isolated HOME and does not mutate global Claude, Codex, or pi.dev skill installations.
- `issues/AGENTS.md` and template source files remain unchanged unless their content genuinely changed.
- The focused dogfood-copy test passes on the release commit itself.
- The release run does not require a separate post-release version-refresh commit.
