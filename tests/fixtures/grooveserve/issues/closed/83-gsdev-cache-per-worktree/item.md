---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#75"]
labels: [devex, gsdev]
---

# 83. gsdev cache: per-worktree binary or content-fingerprint sidecar

_Renumbered #82 → #83 — main concurrently filed `#82-agent-trace-schema-cleanup` (commit `db207dc`); same numbering collision pattern as #80 → #81 from this epic._


_Source: `/llm-review` of #75 (`history/review-gsdev-rebuild-policy.md`,
SPIN-OFF #5)._

## Resolution (2026-05-01)

Implemented vaihtoehto B (content-fingerprint sidecar) bundled into
#75:n PR — vaihtoehto A:n levynkäyttö (~1–2 GB / worktree)
osoittautui kestämättömäksi.

- `tools/dev/gsdev/build_cache.py::compute_source_fingerprint()` —
  SHA256 over `(rel_path, content_sha256)` of each tracked source,
  ~3 ms / 41 files.
- `record_build_fingerprint()` kirjoittaa
  `~/.cache/gsdev/cli/release/gs-dev.fingerprint` build-jälkeen
  (`mail.py::_build_gs_dev` kutsuu `os.utime`:n jälkeen).
- `compute_cache_status()` lukee storedin, vertaa nykyiseen.
  Mismatch → `status="stale"`. Kerroksellinen freshness:
  1. mtime-tarkistus (fast path, ~1 ms)
  2. fingerprint-vertailu (~3 ms, kattaa branch-switch-tapaukset)
- `gsdev doctor` raportoi nyt myös `current_fingerprint`,
  `stored_fingerprint`, `stale_reason ∈ {mtime, fingerprint_mismatch,
  no_stored_fingerprint}`.
- 5 uutta yksikkötestiä (yhteensä 23 vihreänä):
  - `test_status_stale_when_no_fingerprint_sidecar` (legacy cache)
  - `test_status_stale_when_source_content_changes_but_mtime_does_not`
    (the actual #83 scenario)
  - `test_record_build_fingerprint_writes_sidecar`
  - `test_fingerprint_changes_when_source_content_changes`
  - `test_fingerprint_stable_for_unchanged_content`

**Mittaus:** kokonaisbudjetti 2,1 ms / `compute_cache_status()`-kutsu
(mtime + fingerprint), edelleen reilusti alle 100 ms warm-cache-rajan.

**Trade-off:** kun kehittäjä vaihtaa worktreesta toiseen, ensimmäinen
`gsdev mail …`-kutsu uudessa worktreessä rebuildaa (fingerprint
mismatch) — sccache pitää sen sekunneissa, ei kymmenissä sekunneissa.
Hyväksyttävä hinta hiljaisen virheen poistamisesta.

## Description

The `gs-dev` binary cache lives at the global path
`~/.cache/gsdev/cli/release/gs-dev`, shared across every worktree on
the developer's machine. The mtime-based freshness check
(`tools/dev/gsdev/build_cache.py`) compares the binary's mtime against
the **current worktree's** source mtimes. This recreates the original
#75 incident in a multi-worktree workflow:

1. Worktree A on branch X → builds binary at T=1000.
2. Switch to worktree B on branch Y where source mtimes are T=500
   (older code).
3. `gsdev mail send` from worktree B → freshness check sees
   `binary_mtime=1000 > newest_source=500` → "fresh".
4. The binary from branch X runs against branch Y's database / config
   — silent staleness, identical symptom to #75.

The whole point of the gsdev workflow is one worktree per branch
(`workmux`), so multi-worktree inconsistency is the documented
pattern, not an edge case.

## Scope — two viable approaches

**Vaihtoehto A — per-worktree cache path:**
- `paths.GS_DEV_CLI_TARGET = Path.home() / ".cache" / "gsdev" / "cli" / safe_resource_id(slug)`
- Each worktree gets its own binary, same `cargo build` invocation,
  no fingerprint logic needed.
- Cost: N copies of the binary (~50 MB each release) on disk per
  developer machine. With ~10 active worktrees, that's ~500 MB.
- `gsdev clean targets` (or a new `gsdev clean cli-cache`) prunes
  cache dirs whose worktree is gone.

**Vaihtoehto B — content-fingerprint sidecar:**
- Store a fingerprint file `~/.cache/gsdev/cli/release/gs-dev.fingerprint`
  alongside the binary, containing a hash of the build's git HEAD or
  a content hash of the tracked source files at build time.
- Freshness check: compare current source fingerprint against stored
  fingerprint, not just mtime.
- Cost: one extra hash computation per build (~10–50 ms for ~40 files),
  one extra read per cache check.
- Cache is shareable across worktrees on the same commit (saves disk
  vs. A) but the implementation is more delicate.

## Acceptance

- A binary built from worktree A's source on branch X must NOT be
  considered fresh when invoked from worktree B on branch Y (different
  source content).
- `gsdev doctor` reports the cache state in a way that makes branch-
  inconsistency visible.
- The fix doesn't regress the warm-cache 100 ms budget.

## Out of scope

- Eliminating sccache or changing CARGO_TARGET_DIR semantics.
- Cross-machine sharing of the cache (separate concern).

## Related

- #75 (parent — mtime-based rebuild check, landed)
- `/llm-review` raised this in cross-review; Anthropic and DeepSeek
  reinforced. Fix requires its own design decision (A vs B), so
  spun off rather than bundled into #75.
