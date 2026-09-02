---
created: 2026-08-28
updated: 2026-09-02
type: bug
reporter: jari
status: open
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:homebase-wrapup-2026-08-28-issuectl-body-file-description
lane: create-body
collision: [crates/issuectl-core/src/write.rs, crates/issuectl/src/cmd/mod.rs]
---

# issuectl create --body-file duplicates Description heading

## Description

issuectl create --body-file duplicates Description heading

## Observed

In homebase, this command supplied a complete Markdown body beginning with `## Description`:

```sh
cat report.md | issuectl create --json --type bug \
  --title 'Politics digest PLAN timeout has no transient retry' \
  --source 'live haapa digest 2026-08-28 19:00 EEST' \
  --body-file -
```

The created issue body contained two consecutive headings:

```markdown
## Description

## Description

The scheduled politics consumer failed ...
```

The first heading was empty. It had to be removed manually before committing.

## Expected

When `--body-file` supplies structured Markdown, `issuectl create` should write it below the H1 as documented without injecting an additional `## Description` heading. Supplying `--source` may add the `_Source: ..._` line, but should not duplicate a body heading.

Alternatively, if `## Description` is intentionally always generated, validation/help should state that `--body-file` must contain only section content rather than a complete Markdown body. Current help says the initial body is read from the file and written below the H1, and its example permits H2 sections.
