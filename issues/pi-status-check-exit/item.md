---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: low
closed: 2026-08-14
closed_by: jari
---

Follow-up from the /llm-review of `pidev-pi-skill-lifecycle` (see
`history/review-pi-skill-lifecycle.md`). Anthropic, OpenAI, and DeepSeek noted
`issuectl skill pi-status` always exits 0, so CI can't gate on corpus drift
without parsing JSON. Add a `--check` flag that exits non-zero when
`PiStatusReport::has_findings()` is true (stale/modified/missing/orphan), leaving
the default informational exit-0 behavior unchanged.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: status exit-code semantics only matter if someone scripts against them; no such need. Low value.
