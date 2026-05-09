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
- **Tests live next to the code** in `#[cfg(test)] mod` blocks by
  default. New features add tests; bug fixes add regression tests.
  - **Exception — `tests/` integration tests:** use only for
    black-box behaviour that no inline test can observe: process
    exit code, byte-level stdout/stderr, argument parsing performed
    by the built binary, and `main()`'s `anyhow::Error` rendering.
    Anything reachable through a `pub(crate)` entry point belongs in
    an inline `#[cfg(test)]` module.
- **`update --type` scaffolds missing required body sections (append, don't reject).**
  When `--type` lands a value whose schema requires body sections that
  aren't already present, `update` appends `## <Section>` stubs to the
  body — mirroring `cmd_new`'s scaffolding so a type change can't
  silently drift into a doctor-failing state. The alternative
  considered was rejecting the change with `MutateError::SchemaViolation`;
  appending was chosen because the user's intent is unambiguous (they
  asked for the new type), the no-op-on-already-present case is safe
  (`schema::missing_body_sections` is idempotent and fence-aware), and
  it removes a dead-end where the user has to hand-edit the body
  before the mutation will succeed. Looser-target type changes are a
  body-noop. Reuses the same `schema::missing_body_sections` +
  `schema::stub_for_sections` pair as `mutate::new_issue`.
- **New mutation verbs go in `mutate.rs`, CLI handlers stay thin.**
  Every write path (CLI subcommand or web endpoint) routes through a
  function in `src/mutate.rs` so a) every writer obtains the same
  repo-wide `flock`, b) every writer emits the same canonical version
  token, and c) schema validation runs in exactly one place. Add new
  domain logic as a public function in `mutate.rs` (or a sibling
  domain module) and keep the `cmd_*` handler in `main.rs` to
  argument parsing + JSON / human formatting (≤30 lines is the
  target). Do **not** reach into `write::*` directly from `main.rs`
  for new write paths — that bypasses the lock and the schema check.
- **Domain logic lives in domain modules, not in `main.rs`.**
  Layering complement to the operational rule above. `main.rs` owns
  CLI parsing (clap structs), the top-level `cmd_*` handlers, and
  `find_root` — nothing else. State-changing logic (lock acquisition,
  schema validation, slug claiming, atomic writes) lives in
  `src/mutate/`. Pure on-disk render/serialize primitives live in
  `src/write.rs`. Shared domain helpers (issue enums like
  `ISSUE_TYPES`/`PRIORITIES`, ref normalization, status
  classification) live in their own domain module (`src/refs.rs`,
  etc.). **No module under `src/` other than `main.rs` may
  reference items defined in the crate root** — if a `mutate::*` or
  `write::*` site needs to call `crate::foo()`, `foo` belongs in a
  domain module. The `_pub` re-export wrapper anti-pattern is the
  warning sign that a private root helper is leaking.
- See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, repo layout,
  PR process, and commit-message conventions.
- See [issues/AGENTS.md](issues/AGENTS.md) for how this project's own
  issue tracker is organized.

## When in doubt

Run `issuectl --help` and `issuectl <subcommand> --help`. The CLI
help is the source of truth for currently-accepted flags.
