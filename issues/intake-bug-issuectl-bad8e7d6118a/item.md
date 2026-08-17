---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# skill install --force silently overwrites repo-authored content in issu…

## Description

skill install --force silently overwrites repo-authored content in issues/AGENTS.md

`issuectl skill install --force` rewrites the `issues/AGENTS.md` scaffold from
the bundled template, discarding any content the repo has added to that file.
It is reported as a normal overwrite, with no warning that local prose was lost.

## Observed

In ~/Sources/homebase (issuectl 0.14.1), refreshing the installed skills:

    $ issuectl skill install --force
      ✓ Overwrote .claude/skills/issue/SKILL.md
      ✓ Overwrote .claude/skills/issue-new/SKILL.md
      ✓ Overwrote .claude/skills/issue-intake/SKILL.md
      ...
    $ git diff --stat
     .claude/skills/issue-intake/SKILL.md |   4 +-
     .claude/skills/issue-new/SKILL.md    |   8 ++-
     .claude/skills/issue/SKILL.md        | 133 ++++++++++++++++++-------
     issues/AGENTS.md                     |  18 -----

The 18 removed lines were a repo-authored "## Scheduling DAG" policy section
(the project's own rules about `issuectl dag` being the authoritative schedule
and root TODO.md being narrative only) plus a pointer to where the generic
`/stint-*` skills are maintained. Nothing in the output indicated that the
scaffold is regenerated wholesale, and nothing distinguished "restored a
missing scaffold" from "deleted content you wrote".

Recovered with `git checkout -- issues/AGENTS.md`; a repo without a clean
worktree at that moment would have lost the text.

## Expected

Any of the following would prevent the loss:
- Leave `issues/AGENTS.md` alone when it already exists (the skills are the
  thing `skill install` is being asked to install), or
- write only a marker-bounded managed region and preserve everything outside it,
  the way marker-anchored generators elsewhere in the family work, or
- at minimum, detect that the existing file diverges from the bundled scaffold
  and refuse (or require a separate, explicitly-named flag) rather than folding
  it into the same `--force` that refreshes the skill bodies.
