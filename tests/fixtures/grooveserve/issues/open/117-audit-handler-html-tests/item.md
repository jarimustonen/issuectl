---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#101"]
labels: [testing, admin-audit]
---

# 117. HTML-tason testit audit-log handlerille

_Source: `/llm-review` assessment of `fd6d445` (#101)_

## Description

`crates/server/src/http/routes/audit.rs` has zero tests. The ops layer (`audit.rs`) has 9 integration tests covering SQL queries, filtering, pagination, tenant isolation, and authorization gates — but the rendering layer is untested.

Concerns that ops tests cannot catch:

- **XSS**: `metadata_summary()` injects user-controlled strings without `escape()`
- **Date boundary**: `to_dt` caps at `23:59:59` with `<=`, silently excluding sub-second events
- **Pagination clamping**: `MAX_PAGE` clamping and total computation at the handler level
- **Filter rendering**: hardcoded `target_type` dropdown, whitespace handling, URL encoding
- **Empty state**: what the page looks like with zero results

## Possible approaches

**A) Extract pure rendering functions** — separate data transformation from HTML generation. Functions like `metadata_summary`, `render_table`, `render_filters` return structured data; HTML assembly happens separately. Tests assert on the structured output.

**B) Full HTTP integration tests** — spin up the server with a test DB and assert on HTML responses. Requires test infra.

Approach A is lighter and sufficient for catching the XSS/date/pagination concerns. Approach B would also catch template-level issues but requires more setup.

## Scope

At minimum: test `metadata_summary` with malicious metadata values and `render_table` with edge-case rows. Add a date-boundary test for the `to_dt` cap.
