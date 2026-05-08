# Implement: {{title}}

You are implementing issue `@{{slug}}` ({{type}}, status: {{status}},
priority: {{priority}}). Treat the context block below as authoritative
— it includes the issue body, parent epic (if any), related and blocking
issues, recorded commits, and the active frontmatter schema.

{{context}}

---

When done, record commits with `issuectl --json update {{slug}}
--add-commit "HASH:summary" --expected-version {{version}}` and close
with `issuectl --json close {{slug}} --status done --expected-version
{{version}}`. Refresh the version token with `issuectl --json show
{{slug}}` if the bundle becomes stale before you write back.
