---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: in-progress
priority: normal
labels: [observability]
---

# issuectl dag lists closed/terminal issues in its unscheduled output

_Source: crates/issuectl-core/src/dag.rs_

## Description

## Observed
`issuectl dag` lists **closed / terminal** issues (status `done`, `fixed`, `obsolete`, `wontfix`, …) in its output — they appear in the "unscheduled" section alongside genuinely active work. Example (abridged real output):
```
unscheduled
    awfully-faint-sound          done       ...
    fiercely-colossal-rabbits    obsolete   ...
    events-jsonl-log             wontfix    ...
    pidev-dual-home-skills       done       ...
  ▶ ossctl-cut-no-publish        open       ...
  ▶ rate-limit-test-flaky        open       ...
    epic-tree-view               open       ...
```

## Impact
The DAG is a **scheduling** view — only non-terminal issues (`open` / `in-progress`, minus `deferred`) can ever be scheduled/spawned. Dumping every closed issue into 'unscheduled' is noise, and it actively misleads: a reader (human or agent) skimming `dag` output can mistake shipped-and-closed work for open backlog. This actually happened — another agent read the `dag` view and reported four **closed** dag-* issues (`dag-scheduling-view` done, `dag-inprogress-spawnable` fixed, `dag-stable-intralane-order` done, `dag-unlaned-parallel-sentinel` done) as 'open bugs'.

## Expected
`issuectl dag` shows only non-terminal issues by default (`open` + `in-progress`; `deferred` parked as it is today). Terminal-status issues are excluded from the scheduling view. If listing them is ever wanted, put it behind an explicit `--all` / `--include-closed` flag — off by default.

## Fix direction
Filter the input set to non-terminal statuses in the dag computation (`crate::dag` head-of-line / spawnability build, or the `cmd_dag` input query) before rendering. Add a test that a `done`/`wontfix` issue does not appear in default `dag` output.

## Note
Confirmed on the version at commit ~2031294 (0.8.1 tree).
