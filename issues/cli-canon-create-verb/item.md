---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: done
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 70
commits:
- hash: 225652d
  summary: 'feat(cli): make create the primary issue verb'
- hash: 99d996b
  summary: 'test(cli): cover create alias routing'
closed: 2026-08-16
---

# cli-canon: §7 prefer create over new as primary verb

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§7) — primary create verb is `new`, not the canonical `create`.**

`create` exists only as an alias; the corpus-familiarity trap §7 warns against. Minor, but
part of one uniform verb vocabulary across the family.

**Do:** make `create` the primary/documented create verb (keep `new` as the alias for
back-compat), so help + examples lead with `create`.

**Current state (evidence):** `new … [aliases: create]` — create is only an alias.
