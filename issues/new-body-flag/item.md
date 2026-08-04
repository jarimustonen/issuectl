---
created: 2026-08-01
updated: 2026-08-04
type: feature
status: fixed
priority: normal
closed: 2026-08-04
---

# issuectl new: accept --body/--body-file to set the initial issue body

## Status (trimmed 2026-08-04)

**`--body` (inline text) already shipped in v0.6.5** as an alias for
`--description` on `new` (the CLI-alias nippu, `@verb-alias-discoverability`), so
`issuectl new "Title" --type feature --body "…markdown…"` works today. The
**remaining scope** is only:
- `--body-file <path>` — read the body from a file, and
- accept `-` for **stdin**.

The `## Observed` block below predates v0.6.5 and is stale for `--body`. Build
just the file/stdin variants.

## Description

`issuectl new` creates `item.md` with an empty body — there is no flag to set
the body at creation time. An agent (or human) that wants a fleshed-out issue
must always do a **second step**: create, then edit `item.md` (or `issuectl
body`). In an agent workflow that files several issues per session, this is
repeated friction — every `new` is followed by a hand-edit.

## Proposal

Add mutually-exclusive `--body <text>` and `--body-file <path>` (and accept `-`
for stdin) to `issuectl new`, writing the given markdown into the body section
below the `# <title>` heading. Mirrors how many CLIs (`gh issue create --body`,
`gh pr create --body-file`) work, and matches the AI-first "self-describing in
one argv" principle.

## Observed

```
$ issuectl new --help | grep -iE '\--body|\--file'   # → nothing
```
So today: `issuectl new --type feature --title X --slug y` then a separate edit
of `issues/y/item.md`.

## Notes

Found while filing several issues from an agent session in another repo
(homebase digest system). Low priority but a nice ergonomic win for agent use.

