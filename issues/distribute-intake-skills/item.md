---
created: 2026-08-06
updated: 2026-08-06
type: feature
status: in-progress
priority: normal
commits:
- hash: 4e81422
  summary: distribute /issue-new + /issue-intake via skill install
---

# Distribute /issue-new + /issue-intake via skill install (fleet)

## Description

Today `issuectl skill install` installs only the binary-shipped /issue skill; the standalone /issue-new (filer) and /issue-intake (processor, replaces /triage-bugs) live in this repo's .claude/skills but are NOT distributed to Jari's machines (only the old /triage-bugs is in ~/.claude/skills). Add an option (or companion behaviour) so `skill install` also installs /issue-new + /issue-intake, so the haapa fleet-apply hook distributes them the same way as /issue. They need their own install + tests (they are NOT covered by the /issue dogfood test — see docs/design/intake-flow.md §4). This unblocks retiring /triage-bugs in homebase (thin deprecation alias first). Context: homebase issue bug-feature-intake-architecture (the intakectl intake-architecture).
