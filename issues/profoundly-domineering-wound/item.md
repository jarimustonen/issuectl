---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [agent-friendly, cli]
---

# Agent context bundle (issuectl context <slug>) + repo-local prompt templates

_Source: src/cli/context.rs (new), src/cli/prompt.rs (new), .issuectl/prompts/ (new)_

## Description

issuectl context <slug> renders a deterministic markdown/JSON bundle: issue + epic + blockers + acceptance criteria + linked commits + schema rules. issuectl prompt <template> <slug> renders repo-local templates (.issuectl/prompts/implement.md). Default stdout; --write places artefacts under .issuectl/cache/agent/<slug>/ (gitignored). Pairs with the planned 'aloita toteuttaminen' button (@excessively-beneficial-owner) — same prompt-shaping logic powers both.
