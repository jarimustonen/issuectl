---
created: 2026-08-11
updated: 2026-08-11
type: feature
status: in-progress
priority: high
---

# Dual-home skills into pi.dev's skill dir (~/.pi/agent/skills)

## Description

## Problem

`issuectl skill install` currently installs this CLI's companion skill(s) into
`~/.claude/skills/` only, so they are **not discoverable under the pi.dev harness**.
pi discovers skills from `~/.pi/agent/skills/`, `.pi/skills/`, `.agents/skills/`
and invokes them as `/skill:name`.

## Ask

Teach the installer to **dual-home**: place each skill's `SKILL.md` into
`~/.pi/agent/skills/<name>/` in addition to `~/.claude/skills/<name>/`, without
breaking the Claude Code path. For vendored/bundled skills, mirror **only**
`SKILL.md` into the pi target (same filtering homebase's `dotfiles link` applies).

## Context

Jari is migrating the agent stack from Claude Code to pi.dev. homebase's
`dotfiles link` already dual-homes its `~/.claude/skills` corpus into
`~/.pi/agent/skills/` (homebase issue `pidev-skills-portability`, epic
`pidev-migration`, workstream **WS4** = "propagate the convention to the
binary-owned skill installers"). But binaries that ship their own skills via
`issuectl skill install` bypass `dotfiles link`, so their skills stay Claude-only
under pi. Verified on pi v0.82.0 (openai-codex / gpt-5.5): pi loads
`~/.pi/agent/skills/<name>/SKILL.md` and invokes `/skill:name`; bare `/name`
cross-references also resolve via pi's injected available-skills list, so **no
cross-reference rewrite is needed — only the install target**.

## Done

- `issuectl skill install` (and `--force` / `--agent` variants) create each skill
  under `~/.pi/agent/skills/` too — idempotent, vendored-filtering-aware.
- Claude Code install path unchanged.
- Documented (README / AGENTS).
