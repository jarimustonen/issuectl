---
created: 2026-05-10
updated: 2026-05-10
type: improvement
reporter: jari
status: open
priority: high
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
---

# doctor: warn when issuectl-relevant files (.issuectl/AGENTS.md, .schema.yaml) are gitignored

## Description

After 'agents init' the file is silently untracked if .gitignore has '.issuectl/' or similar. Doctor doesn't notice. Asymmetric footgun: works locally, agents/CI on other machines see missing-file fallback. Fix: when doctor sees that .issuectl/AGENTS.md, issues/.schema.yaml, or any tracked-by-design file is matched by 'git check-ignore', surface a warning with remediation hint. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
