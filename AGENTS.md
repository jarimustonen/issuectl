# AGENTS.md

Guidance for AI agents (and humans) working **inside this repo**. The
canonical entry points are below; click through if you need detail.

## What this project is

`issuectl` — AI-first CLI for managing markdown-based issues with
frontmatter. See [README.md](README.md) for a user-facing overview.

## Design principles

All CLI changes must follow [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md):
strict input validation, structured (`--json`) output, no interactive
prompts, informative errors, composable commands.

## Critical rule: keep the skill in sync with the CLI

The `/issue` skill template is shipped with the binary via
`include_str!` in `src/skill.rs` and installed by `issuectl skill
install`. It tells consumer-side agents how to use this CLI. **It is
the only contract those agents see.**

**Whenever you change the CLI in a way an agent would notice, update
the skill templates in the same commit.** Triggers include:

- New subcommand or flag added → add a usage example
- Flag renamed, removed, or its accepted values changed → update examples
- Output shape changed for `--json` → update the documented shape
- Install destination or filename changed → update the install table
- Default values changed → update the prose
- Error messages or exit-code semantics changed in a way agents handle

The two template files:

- `templates/issue-skill.md` — Claude Code variant (YAML frontmatter)
- `templates/issue-prompt.md` — Codex CLI variant (plain markdown)

Both are dogfooded into this repo as `.claude/skills/issue/SKILL.md`
and `.codex/prompts/issue.md` via `issuectl skill install --agent all
--force`. After editing either template, run that command (or
equivalent) so the local copies don't drift from `templates/`.

If the two variants would otherwise drift, regenerate the Codex one
from the Claude one with:

```sh
tail -n +5 templates/issue-skill.md > templates/issue-prompt.md
```

(That strips the Claude-specific YAML frontmatter, leaving the body.)

## Other conventions

- **Always `--json`** when scripting `issuectl` from another tool or
  agent. The human-readable mode is for terminal users.
- **Tests live next to the code** in `#[cfg(test)] mod` blocks. New
  features add tests; bug fixes add regression tests.
- See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, repo layout,
  PR process, and commit-message conventions.
- See [issues/AGENTS.md](issues/AGENTS.md) for how this project's own
  issue tracker is organized.

## When in doubt

Run `issuectl --help` and `issuectl <subcommand> --help`. The CLI
help is the source of truth for currently-accepted flags.
