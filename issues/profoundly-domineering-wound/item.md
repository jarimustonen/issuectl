---
created: 2026-05-06
updated: 2026-05-08
type: feature
status: done
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [agent-friendly, cli]
commits:
- hash: a1fe3cb73d37859418806322dfffaf26621c5ca7
  summary: 'feat(context): agent context bundle and prompt templates'
- hash: d29967243418dd890825c822bdbf3ea3d6f1644b
  summary: 'fix(context): address llm-review findings'
closed: 2026-05-08
---

# Agent context bundle (issuectl context <slug>) + repo-local prompt templates

_Source: src/cli/context.rs (new), src/cli/prompt.rs (new), .issuectl/prompts/ (new)_

## Description

issuectl context <slug> renders a deterministic markdown/JSON bundle: issue + epic + blockers + acceptance criteria + linked commits + schema rules. issuectl prompt <template> <slug> renders repo-local templates (.issuectl/prompts/implement.md). Default stdout; --write places artefacts under .issuectl/cache/agent/<slug>/ (gitignored). Pairs with the planned 'aloita toteuttaminen' button (@excessively-beneficial-owner) — same prompt-shaping logic powers both.
