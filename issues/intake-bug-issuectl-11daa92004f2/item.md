---
created: 2026-10-01
updated: 2026-10-01
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:nah-2026-10-01-unlaned-unscheduled
---

# lane: unlaned lands in unscheduled despite dag --help

## Description

lane: unlaned lands in unscheduled despite dag --help

issuectl 0.18.7 in native-agent-host (2026-10-01). `issuectl dag --help` says: "`lane: unlaned` means confirmed parallel-safe work: every member is independently headed and spawnable. It differs from an absent lane, which means unclassified work."

Observed: `issuectl update design-package-cross-device --lane unlaned` (status open, no collision; also tried --lane-seq 1) → `issuectl dag --json --reservations '[]'` lists the slug in `.data.unscheduled` and there is no `unlaned` entry in `.data.lanes[]`. The wrap-up helper `list-unlaned-issues.sh` therefore still reports it as outside the DAG.

Expected: an `unlaned` lane row in `.data.lanes[]` with the issue independently headed and spawnable, per the help text; or, if `unlaned` was retired, `update --lane unlaned` should be rejected and the help corrected.

<!-- intakectl:analysis:start job:513fc62d-5043-4d43-ad00-c5d60d568c06 generation:0 -->
## Triage analysis

- **Verdict:** The reported `dag` output is by design, not a DAG computation bug. `unlaned` is an independently spawnable sentinel represented in `unscheduled`, not a serialized `lanes[]` entry; its row retains `lane: "unlaned"` so consumers can distinguish it from an unclassified issue.
- **Severity:** Low / documentation-or-consumer-contract mismatch. No scheduling failure is demonstrated: the issue should appear in `unscheduled` and count toward `spawnable_heads` when runnable.
- **Affected area:** `issuectl-core/src/dag.rs` DAG partition and serialization; downstream consumers such as wrap-up's `list-unlaned-issues.sh` must inspect `unscheduled`, not expect a lane named `unlaned`.
- **Reproduction status:** Confirmed against repository implementation and regression test `unlaned_sentinel_members_are_parallel_spawnable` in `crates/issuectl-core/src/dag.rs`. That test explicitly asserts no `unlaned` lane is created, two sentinel issues appear in `unscheduled`, and both are spawnable with the sentinel echoed on each row.
- **Fix sketch:** No change to DAG grouping is warranted; creating a lane would contradict the parallel-safe semantics and risk consumers treating it as a serial lane. If the wrap-up helper actually misses these rows, fix that consumer to recognize `unscheduled[]` items whose `lane` is `"unlaned"` (and verify it against current JSON). Otherwise clarify the issue's expectation rather than changing issuectl behavior.
<!-- intakectl:analysis:end job:513fc62d-5043-4d43-ad00-c5d60d568c06 generation:0 -->
