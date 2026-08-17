---
created: 2026-05-10
updated: 2026-05-10
type: bug
reporter: jari
assignee: jari
status: fixed
priority: high
labels: [release-v0.5.1]
closed: 2026-05-10
---

# issues/AGENTS.md template is stale; replace with /issue skill pointer + doctor --fix support

## Description

The template at crates/issuectl-core/templates/issues-agents.md (installed by 'issuectl skill install') still describes the pre-v0.2.0 numbered layout with open/closed/ subdirs and is duplicative of v0.5.0's .issuectl/AGENTS.md. Replace with a minimal pointer to the /issue skill, teach 'doctor --fix' to rewrite stale copies, and add a version-check to the /issue skill template so the agent prompts the user to upgrade issuectl when the runtime version drifts from the version that wrote the skill.
