---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: low
---

Follow-up from the /llm-review of `pidev-pi-skill-lifecycle` (see
`history/review-pi-skill-lifecycle.md`). Anthropic, OpenAI, and DeepSeek noted
`issuectl skill pi-status` always exits 0, so CI can't gate on corpus drift
without parsing JSON. Add a `--check` flag that exits non-zero when
`PiStatusReport::has_findings()` is true (stale/modified/missing/orphan), leaving
the default informational exit-0 behavior unchanged.
