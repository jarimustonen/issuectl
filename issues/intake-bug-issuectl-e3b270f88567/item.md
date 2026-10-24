---
created: 2026-10-21
updated: 2026-10-21
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:oh-wrapup-20261021-ictl-laneorder
---

# dag: priority overrides lane_seq within a lane, so a parked high-priori…

## Description

dag: priority overrides lane_seq within a lane, so a parked high-priority head cannot be moved back

Observed 2026-10-21 in open-habitat. Lane `web` had `reconnect-live-cause` (priority high, waiting only for a new live occurrence) at its head, blocking five normal-priority issues. `issuectl update reconnect-live-cause --lane-seq 99 --json` succeeded, but `issuectl dag --json --reservations '[]'` still listed it at position 0 / head_of_line, apparently because ordering sorts by priority before lane_seq. Workaround: move it to another lane (`--lane unlaned`).

Expected: within a lane, explicit `lane_seq` defines order (priority at most a tie-breaker), or the docs/`dag` output state the ordering rule and `update --lane-seq` warns when it has no effect on position. Also minor: `update` has no `--preparation` flag (needs `--field preparation=ready`) while dag reports `preparation_reason: preparation_missing`; a first-class flag or a hint in the reason would help.

<!-- intakectl:analysis:start job:f3c44283-7699-41fd-994a-4c57826dd9cd generation:0 -->
## Triage analysis

- **Verdict:** The reported ordering is confirmed in the current implementation; it is an explicit scheduling policy, not a failed persistence of `lane_seq`. The practical problem is that a human cannot use sequence alone to move a high-priority item behind normal-priority work. No disposition or fix is made here.
- **Severity / impact:** Normal. A waiting item can keep useful work behind it in a serial lane. There is no evidence of data loss. A manual wait for another live occurrence is not a dependency the DAG can infer.
- **Affected area:** `crates/issuectl-core/src/dag.rs`, intra-lane ordering and preparation-gated head selection; CLI scheduling discoverability.
- **Evidence / reproduction status:** Source-confirmed, not replayed against the original open-habitat repository. `OrderKey` derives ordering with `priority` before `no_lane_seq` and `lane_seq`; `order_lane` uses this key for its topological ready set. The existing test `lane_seq_setter_sorts_ahead_of_unset_but_below_priority` encodes the exact relevant case: a high-priority member with sequence 99 precedes both normal-priority members. This test was inspected, not executed during this analysis. Intra-lane dependencies still take precedence over these keys.
- **Existing contract:** `docs/design/lane-design.md` (Intra-lane ordering) already says priority outranks sequence, and installed `issuectl 0.21.0 update --help --json` describes `--lane-seq` as consulted after dependencies and priority. Thus documentation of the rule already exists, although the observed workflow still exposes an expectation mismatch. Also, setting sequence sorts ahead of an unset sequence within the same priority band: 99 is not an unconditional “send to back” instruction even among equal-priority peers.
- **Head blocking distinction:** An explicit open dependency allows the work-conserving head search to advance to another runnable member. A preparation gate instead checks readiness after selecting the head; `preparation_gate_keeps_unready_head_in_place` explicitly pins that an unready head stalls ready followers. If the parked item is unready rather than dependency-blocked, sequence remaining subordinate to priority can therefore strand the lane. Without that gate or an explicit blocker, the DAG can still advertise the parked item as spawnable despite the human's external wait.
- **Workaround caveat:** `lane: unlaned` means independently executable, not parked. It releases the original lane but makes the moved item its own head and potentially spawnable, so it is not a safe general parking mechanism.
- **Fix sketch:** Settle whether explicit sequence should override priority before changing the comparator. If that is the desired contract, update both `OrderKey` and `tiebreak_sorted`, preserve dependency precedence, and add coverage for mixed priority, set/unset sequences, and preparation-gated heads; revise help, design documentation, and shipped skill templates together. If retaining the existing policy, a contextual explanation of the effective ordering/head blocker would address the discoverability gap without pretending the stored sequence update failed. Adding a false dependency solely to reorder work would misrepresent the graph.
- **Secondary preparation report:** Confirmed that readiness is a custom field and the reason code is `preparation_missing`, not a dedicated `--preparation` flag. The installed JSON help already documents `--field preparation=pending|reviewing|ready` under `--field`; an actionable hint alongside the reason would improve discoverability. Setting `ready` is approval of actual prepared work, not merely a way to clear the warning.
<!-- intakectl:analysis:end job:f3c44283-7699-41fd-994a-4c57826dd9cd generation:0 -->
