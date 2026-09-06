---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: wontfix
priority: normal
closed: 2026-08-12
closed_by: jari
---

# issuectl --json create emits a FLAT object, not a {data:…} envelope (inconsistent with ls/update + taskfleet)

## Description


## Observed vs expected

**Command:** `issuectl --json create --title "…" --type task`

**Observed** — a flat object, no `data` wrapper:
```json
{
  "dir": "/…/issues/zz-wrap-up",
  "path": "/…/issues/zz-wrap-up/item.md",
  "slug": "zz-wrap-up",
  "title": "…",
  "warnings": []
}
```

**Expected** — the same `{schema_version, data:{…}}` envelope that the other
`--json` subcommands use. `issuectl --json ls` and `issuectl --json update`
return `{data: […]}` / `{data:{…}}`, and every `taskfleet --json`/`--output
jsonl` command wraps its payload in `data`. So a caller that does
`json.load(...)['data']['slug']` — the natural pattern learned from every other
command — gets a `KeyError: 'data'` from `create` only.

## Impact
Broke scripted issue-filing twice in one session (a `/stint` autonomous run that
files issues programmatically). The workaround is to special-case `create` and
read `.slug` at the top level, but that's a footgun: the inconsistency is silent
until a script hits it.

## Fix options
- (preferred) Wrap `create`'s `--json` output in `{schema_version, data:{…}}` to
  match `ls`/`update`/`taskfleet`. This is the consistency fix.
- OR, if the flat shape is intentional/load-bearing, document it explicitly in
  `--json`'s help and AGENTS.md so callers know `create` is the exception.

Found 2026-08-12 in the 3dbear-monorepo `/stint` flow.

## Resolution

### 2026-08-12T15:09:48Z · @jari

Wontfix: issuectl's --json create contract is INTENTIONALLY a flat object (per AGENTS.md), not taskfleet's {data:…} envelope. Reporter (parallel session) conflated the two family contracts. No change.
