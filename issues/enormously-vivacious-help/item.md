---
created: 2026-05-28
updated: 2026-05-28
type: improvement
status: open
priority: normal
---

# Add validation.md and breakdown.md to planning-document template

## Description

The documentation/planning-document convention (scaffolded by init-project and documented in repos' AGENTS.md "Issues & Planning" section) currently lists these per-issue planning doc types:

- plan.md — architecture, implementation plans
- analysis.md — research and analysis
- design.md — design documents
- todo.md — task checklists

Two more doc types proved repeatedly useful during epic planning in the sibling crmctl project and should be added to the template/convention:

- validation.md — checks design assumptions against current reality (source repos, existing data); documents what differs from a first-pass analysis so the design can be corrected before implementation.
- breakdown.md — epic → child-issue breakdown with per-child slug, type, scope, acceptance criteria, and inter-issue dependencies + critical path.

Scope: update the init-project scaffolding template and the documented doc-type list so new repos get these two entries. No core code change required (the doc-type list is a convention in AGENTS.md, not hardcoded in issuectl-core; the api.rs:865 reference is only a test fixture).

Origin: crmctl-foundation epic planning, 2026-05.
