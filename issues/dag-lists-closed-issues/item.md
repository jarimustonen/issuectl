---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: fixed
priority: normal
labels: [observability]
closed: 2026-08-12
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

## Resolution

### 2026-08-12T16:55:56Z · @issuectl

Filtered closing-status (terminal) issues out of the dag scheduling view — both the unscheduled bucket and named lanes. done/graph/all_slugs are still built over the full issue set so a done dependency still reads as satisfied. Excluding closed lane members also fixed a latent reordering bug: a closed lane member's blocked_by edge + priority could demote a higher-priority runnable member out of head-of-line. Schema-aware via StatusClass::Closing (honours status_classes overrides). Reviewed by 4-model llm-review; 2 spin-offs filed (dag-inprogress-schema-aware, dag-reservations-run-id-object-shape).
