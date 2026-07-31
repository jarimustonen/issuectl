---
created: 2026-07-30
updated: 2026-07-31
type: feature
status: fixed
priority: normal
closed: 2026-07-31
---

# issuectl note --from-file should not reject legitimate ## / ### headings

_Source: note --from-file_

## Description

note --from-file rejects any body line starting with '## '/'### ' (outside a code fence). Over-eager for legitimate structured notes; details + repro in Comments.

## Comments

### 2026-07-30T03:40:33Z · @jari

`issuectl note --from-file <path>` rejects any body line that starts with `## ` or `### `
(outside a code fence) with: `validation: message line "<...>" begins with '## ' or '### '
outside a code fence; this would break out of the comment block; wrap it in a code fence`.

This is over-eager for legitimate note content. Long structured notes (findings, recipes,
analysis) naturally use `##`/`###` subheadings. Hit repeatedly during a Mediamaisteri
backup session (3dbear-monorepo, 2026-07-29/30): every multi-section note had to be
pre-processed with `sed -E 's/^### (.*)/**\1**/; s/^## (.*)/**\1**/'` to demote headings
to bold before `--from-file` would accept it.

**Observed:** heading lines rejected; caller must manually demote/fence.
**Expected:** one of —
- auto-demote leading `##`/`###` to a safe form (e.g. bold or `####`+) when appending, or
- auto-wrap/indent the body so headings can't escape the comment block, or
- a `--allow-headings` / fenced-body flag that opts into keeping them verbatim.

**Repro:**
```
printf '## Section\n\nbody\n' > /tmp/n.md
issuectl note --as x --from-file /tmp/n.md <some-slug>
# → validation error, note not appended
```

Low severity (workaround is a one-line sed), but recurring — it makes `note --from-file`
awkward for exactly the structured notes it's most useful for.
