---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: normal
related: ['@pi-manifest-locking']
closed: 2026-08-14
closed_by: jari
---

# pi-corpus: mirror SKILL.md writes are non-atomic (torn file on crash)

_Source: crates/issuectl-core/src/skill.rs_

## Description

Spin-off from /llm-review of pi-manifest-locking (Anthropic #8, OpenAI #9).

`install_pi_mirror` → `install_rendered_file` writes with non-atomic `std::fs::write`, so a crash mid-write leaves a truncated `SKILL.md` in the global corpus. This is inconsistent with the atomic temp+rename the manifest now uses (`save_pi_manifest`) and with `write_item_atomic` in mutate/. The new lock serializes issuectl writers but does not protect pi.dev readers, `pi_status`, or other tools from observing a torn copy.

Fix: write mirror copies via same-dir temp + fsync + atomic rename (e.g. `tempfile::Builder::tempfile_in().persist()`), with an explicit symlink policy (see pi-corpus-symlink-traversal). Pre-existing; out of scope for the locking change.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: crash-mid-write torn-file hardening; improbable timing. Review-cascade polish, not worth it now.
