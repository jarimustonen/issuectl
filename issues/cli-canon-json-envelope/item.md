---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: open
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 20
---

# cli-canon: §10 JSON schema_version envelope + version subcommand

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§10) — `--json` output is a bare blob and there is no `version` subcommand.**

`--json` returns bare domain JSON (no `schema_version` envelope), and there is no `version`
subcommand, so an agent cannot detect schema drift on the CLI it calls most. Highest-leverage
single fix in the family (audit §3 #2).

**Do:** (1) wrap all `--json` output in the canon envelope `{"schema_version":N,"data":…,
"warnings":[]}` (errors: `{"schema_version":N,"error":{code,message,…}}`); (2) add a `version`
subcommand whose `--json` carries `supported_schemas[]` + `skills[{name,cli_version,
schema_version}]` (§17 drift-audit in one call).

**Current state (evidence):** `issuectl list --json` is a bare array; `issuectl version` → unrecognized subcommand (only clap `--version`).
