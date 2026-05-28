---
created: 2026-05-28
updated: 2026-05-28
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
closed: 2026-05-28
---

# Prefer descriptive title-derived slugs on issue creation

_Source: .claude/skills/issue + issuectl new_

## Description

When creating an issue, prefer an obvious 2-3 word slug derived from the title (e.g. "Login redirect loops on safari" -> login-redirect-loops) instead of always relying on the random intensifier-adjective-noun slug. Scope decided with user: this is primarily a /issue skill behavior change (the agent proposes a descriptive --slug when one is obvious); the CLI default auto-generation stays random as the fallback when no clear slug exists; and slug collisions should produce a clear terminal error suggesting to retry with a random/auto slug.
