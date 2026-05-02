# Issues

Issues, tasks, features, and epics are tracked here.

## Structure

```
issues/
├── AGENTS.md
├── open/       # Unresolved items (issues and epics)
│   └── NN-short-title/
│       ├── item.md           # Issue/epic description
│       ├── analysis.md       # Optional deeper analysis
│       └── screenshot.avif   # Optional (always AVIF)
└── closed/     # Resolved items (move from open/)
```

## Issue Types

| Type          | When to use                                | Examples                        |
| ------------- | ------------------------------------------ | ------------------------------- |
| `bug`         | Something is broken                        | UI glitch, crash, wrong output  |
| `task`        | Concrete work item with clear scope        | Deploy X, set up Y, migrate Z   |
| `feature`     | New capability or system                   | New CLI command, new integration |
| `improvement` | Enhancement to existing functionality      | Performance, UX, code quality   |
| `chore`       | Maintenance, infrastructure, cleanup       | Backups, key rotation, upgrades |
| `epic`        | Large initiative spanning multiple issues  | API v2 migration, mobile app redesign |

There is no default type. You need to decide or ask for the type.

## item.md Format

```markdown
---
created: YYYY-MM-DD
updated: YYYY-MM-DD
type: bug
reporter: username
assignee: username
status: open
priority: normal
epic: NN              # optional — parent epic number
related: ["#NN"]      # optional — cross-references
labels: [area, tag]   # optional — freeform tags
commits:              # optional — added as work progresses
  - hash: abcdef12
    summary: "fix: description of the fix"
---

# NN. Issue title

_Source: where it happens_

## Description

Description of the problem.

## Reproduction

Steps to reproduce or where the issue is visible.

## Quick Test

Quick way to verify the issue (optional section, omit if not applicable).

## Screenshots

![description](filename.avif)
```

### Frontmatter Fields

| Field        | Required | Description                                        |
| ------------ | -------- | -------------------------------------------------- |
| `created`    | yes      | Date issue was created (YYYY-MM-DD)                |
| `updated`    | yes      | Date of last update (YYYY-MM-DD)                   |
| `type`       | yes      | `bug`, `task`, `feature`, `improvement`, `chore`, or `epic` |
| `reporter`   | yes      | Who reported the issue (epics: use `owner` instead) |
| `assignee`   | yes      | Who is currently responsible (epics: use `owner` instead) |
| `status`     | yes      | Current status (see workflow below)                |
| `priority`   | yes      | `normal` or `high`                                 |
| `commits`    | no       | List of related commits (hash + summary)           |
| `epic`       | no       | Parent epic number (just the number, e.g. `5`)     |
| `related`    | no       | Related issue numbers (e.g. `["#3", "#7"]`)        |
| `labels`     | no       | Freeform tags (e.g. `[infra, monitoring]`)         |
| `closed`     | no       | Date issue was closed (YYYY-MM-DD), set when status becomes a closing status |

Note: `type` defaults to `bug` if omitted (backward compatibility with existing issues).

### Status Workflow

Statuses fall into two categories: **active** (issue stays in `open/`) and **closing** (issue moves to `closed/`).

#### Active statuses (issue stays in `open/`)

| Status        | Meaning                                           |
| ------------- | ------------------------------------------------- |
| `open`        | Created, not yet started                          |
| `in-progress` | Actively being worked on                          |
| `testing`     | Being tested / awaiting verification              |

#### Closing statuses (trigger move to `closed/`)

When an issue reaches any of these statuses, it is moved from `open/` to `closed/` via `git mv`:

| Status              | Meaning                                           |
| ------------------- | ------------------------------------------------- |
| `done`              | Work completed successfully                       |
| `fixed`             | Bug fix committed and verified                    |
| `wontfix`           | Decided not to fix (by design, out of scope, etc.) |
| `duplicate`         | Duplicate of another issue — add `related:` reference |
| `cannot-reproduce`  | Bug could not be reproduced                       |
| `obsolete`          | No longer relevant (superseded by other changes)  |

The status itself tells the story — no separate `resolution` field needed.

#### Typical flows

- **Bug**: open → in-progress → testing → `fixed`
- **Bug (not fixable)**: open → in-progress → `wontfix` / `cannot-reproduce`
- **Task/feature**: open → in-progress → testing → `done`
- **Chore**: open → in-progress → `done` (testing often skipped)
- **Epic**: open → in-progress → `done`
- **Duplicate**: open → `duplicate`

When setting status to `testing`, change `assignee` to whoever needs to verify it.

### Body Conventions

- `_Source: where it happens_` — which service/page/feature
- `_Continues: #NN_` — reference to predecessor issue

These are in the markdown body, not frontmatter.

**Epic linkage**: prefer the `epic:` frontmatter field for new issues. Some older issues may use `_Epic: **#NN** title_` in the body — both are valid, but frontmatter is searchable and preferred.

## Epics

Epics track larger initiatives that span multiple issues and weeks. They live in `open/` and `closed/` just like regular issues, distinguished by `type: epic`.

### When to create an epic

- The work will span multiple weeks
- It involves 3+ related issues
- It has distinct phases or milestones

### Epic item.md format

Epics use the same directory structure as issues (`open/NN-slug/item.md`) but with `type: epic` and adapted frontmatter and body:

```markdown
---
created: YYYY-MM-DD
updated: YYYY-MM-DD
type: epic
owner: username
status: open
priority: normal
---

# ENN. Epic title

## Goal

One-paragraph description of what this epic achieves.

## Issues

- **#NN** Issue title (status)
- **#NN** Issue title (status)

## Phases

### Phase 1: Name
- [x] Completed task (#NN)
- [ ] Pending task

### Phase 2: Name
- [ ] Pending task (#NN)

## Notes

Free-form notes, decisions, context.
```

### Epic frontmatter

Epics use `owner` instead of `reporter`/`assignee` since they are owned long-term, not assigned for a specific fix.

| Field      | Required | Description                        |
| ---------- | -------- | ---------------------------------- |
| `created`  | yes      | Date epic was created              |
| `updated`  | yes      | Date of last update                |
| `type`     | yes      | Always `epic`                      |
| `owner`    | yes      | Who owns this epic                 |
| `status`   | yes      | `open`, `in-progress`, or a closing status (`done`, `obsolete`) |
| `priority` | yes      | `normal` or `high`                 |

### Epic lifecycle

Epics follow the same open/closed flow as issues:
- Created in `open/NN-slug/item.md`
- When all phases complete: set status to `done` → moved to `closed/`
- The `E` prefix in the title (`# E40.`) distinguishes epics visually

## Issue Numbering

Issue numbers are sequential across the entire tracker (`open/` and `closed/`). Each item gets a unique number (e.g. `1`, `14`). No zero-padding required. Epics share the same number space.

**Important**: Numbers must be unique — never reuse or duplicate a number. When creating a new issue or epic, scan both directories to find the highest existing number and increment by 1.

## Creating Issues

Use the `/issue` skill to create new issues and epics interactively.

The skill determines the next number, gathers details (including type), and creates the directory. It suggests creating an epic when the described work sounds like a larger initiative.

## Workflow

- Create new issues with `/issue` skill
- Add `analysis.md` for deeper investigation notes
- When work starts, update status to `in-progress`
- When ready for verification, set status to `testing` and change `assignee` to tester
- When done, set a **closing status** (`done`, `fixed`, `wontfix`, etc.) — this automatically moves the issue from `open/` to `closed/`
- Add the commit to `commits` when a fix/implementation is committed
- Epics: update the `## Issues` and `## Phases` sections as child issues progress

## Commit & worktree-spawn rules

**Issue file changes are always committed.** Status changes, frontmatter
updates, epic Worktree-loki / Decision log edits, new analysis.md files,
git mv to `closed/` — none of these stay uncommitted. Commit them as you
go (a small `docs(issues)` / `chore(issues)` commit is fine), so the
issue tracker on `main` always reflects current state.

**Before spawning a worktree for an issue:**
1. Update the issue's `status:` to `in-progress` (and `updated:` date) in `issues/open/NN-*/item.md`
2. Update the parent epic's Worktree-loki and Phase checkboxes if applicable
3. **Commit those changes on `main`** (or whichever base branch the worktree forks from)
4. Then spawn the worktree

This keeps the worktree's base point already showing the new status — the
worktree branch starts from a tree where the issue is correctly marked in
flight, and `main` reflects what's currently being worked on at all times.
The worktree itself will still update `commits:`, close the issue, and
update the epic when its work is done.

## Images

All images in issues must be in AVIF format. Convert any PNG/JPG/WebP screenshots to AVIF before adding them to the issue directory.
