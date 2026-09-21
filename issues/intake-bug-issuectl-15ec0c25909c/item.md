---
created: 2026-09-21
updated: 2026-09-21
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:native-agent-host-wrapup-note-eof-20260921
---

# issuectl note leaves a blank line at EOF

## Description

issuectl note leaves a blank line at EOF

## Observed

Running `issuectl --json note <slug> --as pi --expected-version <version> '<message>'` appends the note but leaves an extra blank line at end of `issues/<slug>/item.md`.

`git diff --check` then reports:

```
issues/receive-designer-v1/item.md:48: new blank line at EOF.
```

The same warning occurred earlier after adding a note to `review-ui-v1`.

## Expected

The issue body should end with exactly one newline so an ordinary `issuectl note` result passes `git diff --check` without a follow-up `issuectl body set` normalization.

## Environment

- issuectl 0.18.4
- command used with `--json` and `--expected-version`

<!-- intakectl:analysis:start job:f05b77ae-052d-4c36-9d01-1025f6847738 generation:0 -->

## Triage analysis

### Root cause

The blank line at EOF is caused by `insert_block_in_section` in `crates/issuectl-core/src/body_sections.rs` (lines 546–576).

When `issuectl note` appends a block to an existing `## Comments` section that is the **last section** in the file, and the file body ends with `\n` (the normal Unix convention), the function produces a trailing blank line.

**Why:**
1. `body.split('\n')` produces a trailing empty string element when the body ends with `\n`.
2. `insert_block_in_section` computes `tail_lines = &lines[splice..]`. When the section is the last section, `tail_lines` contains only this trailing empty string.
3. The code checks `if !tail_lines.is_empty()` — a slice `[""]` is **not empty** (it has one element), so the condition is true.
4. It adds an extra `\n` before the empty tail, turning the intended single `\n` at EOF into `\n\n` (i.e., a blank line at EOF).

**Trace for a typical case:**
- Body: `"\n# T\n\n## Comments\n\n### …\n\nfirst\n"`
- `split('\n')` → `["", "# T", "", "## Comments", "", "### …", "", "first", ""]`
- `splice = 8` (after `"first"`)
- `head = lines[..8].join("\n")` → `"\n# T\n\n## Comments\n\n### …\n\nfirst"`
- `tail_lines = lines[8..]` = `[""]` — **not empty**
- Output: `head\n\n<block>\n\n` — the second `\n\n` comes from the `!tail_lines.is_empty()` branch pushing `\n` then joining `[""]` (which is empty string).

The `append_new_section` path (used when `## Comments` does not yet exist) avoids this because it calls `body.trim_end_matches('\n')` before appending.

### Fix suggestion

In `insert_block_in_section`, filter out empty trailing lines from `tail_lines` before the emptiness check and join. Specifically, strip trailing blank elements from the tail slice:

```rust
// Before the join, strip trailing blank lines from tail_lines
let tail: Vec<&str> = tail_lines.iter()
    .rev()
    .skip_while(|l| l.trim().is_empty())
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .copied()
    .collect();
if !tail.is_empty() {
    out.push('\n');
    out.push_str(&tail.join("\n"));
}
```

Or simpler: trim trailing newlines from the joined tail string itself.

### Affected path

- `crates/issuectl-core/src/body_sections.rs::insert_block_in_section` (lines 546–576)
- `crates/issuectl-core/src/mutate/body.rs::note_issue_via` (lines 224–340) — caller

### Test gap

`append_into_existing_section_keeps_prior_blocks` (line 1202) exercises `insert_block_in_section` but only checks block ordering and section count — it does **not** assert that the output ends with exactly one `\n`. `append_idempotent_under_fmt` (line 1275) tests the `append_new_section` path only.

<!-- intakectl:analysis:end job:f05b77ae-052d-4c36-9d01-1025f6847738 generation:0 -->
