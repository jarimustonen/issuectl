---
created: 2026-05-06
updated: 2026-08-10
type: feature
status: obsolete
priority: normal
reporter: jari
assignee: jari
epic: hugely-exciting-spiders
labels: [config, kanban, web-ui, deferred]
closed: 2026-08-10
---

# Multiple named kanban boards with per-board configuration

_Source: issues/ (new board config files), src/web/, src/cli/serve.rs_

## Description

Today there is a single implicit kanban board. Allow defining multiple named boards, each with its own configuration: which issues to include (filter by type/label/assignee/epic), which columns to show and how they map to statuses, column order, default sort, etc. Boards selectable in the web UI (tabs or dropdown). Use cases: 'My work', 'Bug triage', 'Release X', 'Epic Y'. Open questions: where boards are stored (issues/boards/*.yaml? .issuectl/boards/?), whether boards are first-class items or just views, CLI surface (issuectl board ls / new / show), how this interacts with the per-user view-state feature (@almost-homely-decision).

## Resolution

### 2026-08-10T10:03:40Z · @issuectl

Web/browser UI is being removed from issuectl (product decision 2026-08-10). This is a web-board enhancement, so it is obsolete. See @remove-web-ui.
