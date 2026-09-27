# Issues

This directory is the repository's issue tracker. Each issue is a directory
named by its slug, holding `item.md`: YAML frontmatter plus a markdown body.
Closed issues that have aged out live under `issues/archive/YYYY/MM/<slug>/`
and are still found by every query. `issues/.schema.yaml` declares the fields
and their allowed values.

The `issuectl` CLI is the interface to this tree. It holds the repo-wide write
lock, validates every write against the schema, keeps `updated:` and the
version token current, and rewrites cross-references when a slug changes. A
hand edit to frontmatter, or a directory moved with `mv`, gets none of that:
other issues end up pointing at a slug that no longer exists, and
`issuectl doctor` will report the damage later. Body markdown is yours to
edit directly when the CLI has no verb for what you need.

The `/issue` skill installed alongside this file is the agent-facing manual:
`.claude/skills/issue/SKILL.md` for Claude Code, `.pi/agent/skills/issue/SKILL.md`
for pi, `.codex/prompts/issue.md` for Codex. It covers finding, creating,
updating and closing issues and the `--json` contract. `/issue-new` files a
report into the intake queue and `/issue-intake` works that queue. If the
skill and the binary disagree, the binary is right; `issuectl --help` is the
source of truth and `issuectl skill install --force` refreshes the skills.

Repository-specific policy, together with a field and status-transition
reference derived from the live schema, lives in `.issuectl/AGENTS.md` when
the repository has opted in with `issuectl init` or `issuectl agents init`.
