# Issues

This directory tracks issues, tasks, features, and epics. Files are
managed by the `issuectl` CLI — do not edit `item.md` frontmatter or
move directories by hand.

For agents and humans interacting with these issues, use the **`/issue`
skill** installed alongside this file (`.claude/skills/issue/SKILL.md`
for Claude Code, `.codex/prompts/issue.md` for Codex). The skill
documents every supported workflow (search, create, update, close,
note, apply) and the JSON contract.

For the agent policy and schema-derived field reference, see
`.issuectl/AGENTS.md` (created by `issuectl agents init`).
