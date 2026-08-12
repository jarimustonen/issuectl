---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: open
priority: low
related: ['@pi-manifest-locking']
---

# pi-corpus: install prints "skills mirrored" hint even when the pi block was skipped

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (OpenAI #11, DeepSeek #1, Anthropic #4).

`install_skill` prints "The same skills are mirrored into ~/.pi/agent/skills…" whenever `pi_root.is_some() && claude selected`, with no check that the pi block actually ran. It already lied when all mirror writes failed (warn-and-skip); the locking change adds one more trigger — a lock-acquire failure now early-returns `Ok(results)` with no pi entries, yet the hint still prints.

Fix: only print the hint when at least one returned `InstallResult` is a pi mirror (path under `pi_root`), or thread a structured mirror status (`Completed`/`Partial`/`Skipped{reason}`) out of `install_skill_summary`. Pre-existing inaccuracy the locking change slightly widens.
