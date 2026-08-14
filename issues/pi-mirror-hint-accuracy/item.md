---
created: 2026-08-12
updated: 2026-08-14
type: bug
status: fixed
priority: low
related: ['@pi-manifest-locking']
closed: 2026-08-14
closed_by: agent-pi-mirror-hint
commits:
- hash: da68ce3
  summary: 'review: require full pi mirror for the hint; test the changed branch'
---

# pi-corpus: install prints "skills mirrored" hint even when the pi block was skipped

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (OpenAI #11, DeepSeek #1, Anthropic #4).

`install_skill` prints "The same skills are mirrored into ~/.pi/agent/skills…" whenever `pi_root.is_some() && claude selected`, with no check that the pi block actually ran. It already lied when all mirror writes failed (warn-and-skip); the locking change adds one more trigger — a lock-acquire failure now early-returns `Ok(results)` with no pi entries, yet the hint still prints.

Fix: only print the hint when at least one returned `InstallResult` is a pi mirror (path under `pi_root`), or thread a structured mirror status (`Completed`/`Partial`/`Skipped{reason}`) out of `install_skill_summary`. Pre-existing inaccuracy the locking change slightly widens.

## Resolution

### 2026-08-14T05:46:24Z · @agent-pi-mirror-hint

Fixed: the pi.dev 'skills mirrored' hint (install_skill) now prints only when the pi block actually mirrored the full managed skill set, not whenever pi_root.is_some() && Claude was selected. Extracted a pure predicate pi_hint_should_print(&results) that requires one PI_SKILL_LABEL result per managed_pi_skills() entry — a skipped block (lock unavailable → early return, or every write warned-and-skipped) leaves zero pi results and a partial mirror leaves fewer than the full set, so the hint (whose copy claims 'the same skills are mirrored') stays off in both cases. Verified by read of the code path plus new tests: pi_hint_predicate_requires_the_full_managed_set (direct branch test), install_summary_signals_hint_when_full_mirror_runs, install_summary_omits_hint_when_block_skipped (hardened: non-empty guard + corpus-escape assertion), and install_summary_omits_hint_on_partial_mirror. cargo test / clippy (no new warnings) / fmt --check all clean. Reviewed via /llm-review (4 models) + /assess-findings; the two consensus FIX findings applied, label→enum refactor and lock-path test dropped with rationale in history/assessment-pi-mirror-hint-accuracy.{json,md}.
