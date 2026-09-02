# Raw review evidence: create body-file

This appendix preserves the complete provider evidence used by the synthesis in `review-create-body-file.md`.

The authoritative review is the preserved run's recovered independent review plus two complete four-model cross-review rounds. Its first DeepSeek independent transport ended mid-response; the exact thread was resumed and produced the complete self-contained response shown immediately afterward. Under the run's recovery policy, that recovered complete output is valid and the transient truncation remains disclosed.

A fresh supplementary review was then run against the adopted tree. It produced complete independent responses and bounded context follow-ups from all four providers. Its first cross-review returned three responses while DeepSeek failed with account-exhaustion status 402; one finite retry failed identically. That incomplete supplementary round is retained for disclosure but is not substituted for the complete authoritative cross-review rounds.

## Authoritative preserved review — initial independent responses

[thread_id:group_2a54f5bc04154f9b8e01f37b4e430554]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_ee0b71452aaa4d3d893db7cdeb51f306]

Here is a critical review of the changes.

### 1. Architectural Flaws & Foundational Design Problems

**String-Stitching Lifecycle (The Root Cause)**
*`crates/issuectl-core/src/mutate/new_issue.rs`, `do_new_locked`*
The entire approach of rendering a raw Markdown string, parsing it back to search for headings (`item_text::split` + `all_h2_sections`), and then string-concatenating missing schema stubs to the end is a fragile anti-pattern. This "render -> parse -> patch" lifecycle is exactly what caused the `---\n\n` bug in the first place. 

While using `item_text::split` correctly patches the bug within the current design, the architecture is fundamentally poor. `issuectl` should construct an intermediate domain object (an AST or Section Map) for the new issue, apply schema validations/stubs to that object, and serialize it to a Markdown string exactly *once*.

**Primitive Obsession / Invalid States in API**
*`crates/issuectl-core/src/mutate/new_issue.rs:65-69`*
You introduced `structured_body: bool` alongside `description: Option<String>` in `NewArgs` and `NewIssueArgs`. This conflates the *content* with the *interpretation* of that content, creating unrepresentable states (e.g., `structured_body: true` but `description: None`). 

Instead of parallel properties, this should be explicitly modeled as a type-safe enum to enforce the contract across the core API:
```rust
pub enum IssueBody<'a> {
    /// Free text to be wrapped in a generated `## Description` heading
    FreeText(&'a str),
    /// Complete structured Markdown placed directly below the preamble
    Structured(&'a str),
}
```
This removes the boolean entirely, forces callers like `intake`, `import`, and `recurrence` to explicitly declare `IssueBody::FreeText(text)`, and makes the internal API foolproof.

### 2. Bugs and Correctness Issues

**Missing Blank Line Before Appended Schema Stubs**
*`crates/issuectl-core/src/write.rs:433-435`*
```rust
    if let Some(d) = args.description {
        body.push_str(d.trim_end());
        body.push('\n');
    }
```
If `args.structured_body` is true, the user's structured markdown is appended, its trailing whitespace is stripped, and exactly *one* newline is pushed. If the repository schema dictates required sections that the user omitted, `do_new_locked` will immediately string-concatenate those stubs (which typically start with `## <Section>`). 

This results in the new heading sitting directly against the last paragraph of the user's body (`Text\n## Section`), violating Markdown best practices which require a blank line before a heading. Even if the internal `issuectl fmt` pass later normalizes this before writing to disk, the writer itself should emit structurally valid Markdown. 
**Solution:** Change `body.push('\n');` to `body.push_str("\n\n");`.

### 3. What Works Well

- **Horizontal Rule Bug Fix:** Reusing `crate::item_text::split(&render)` is an excellent and robust fix. Because it properly tracks the YAML frontmatter boundary, it completely eliminates the risk of truncating the body at a user-provided `---\n\n` horizontal rule.
- **Empty Body Handling:** Using `--body-file /dev/null` cleanly falls back to appending schema stubs because `read_body_file_arg` reads an empty string, bypassing the description injection but still signaling a structured body.
- **Test Coverage & Sync:** The integration tests in `cli_new.rs` correctly verify the absence of duplicate headers, and the synchronization between the Claude and Codex templates exactly matches the repo's strict `AGENTS.md` guidelines.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d3af4b40a7e14b92b448624d7e755476]

## Problems found

### Medium — structured bodies are still not preserved as Markdown content

`crates/issuectl-core/src/write.rs:431-435` continues to apply `trim_end()` to both free-text descriptions and structured bodies:

```rust
if let Some(d) = args.description {
    body.push_str(d.trim_end());
    body.push('\n');
}
```

`str::trim_end()` removes spaces and tabs, not just terminal line endings. In Markdown, trailing spaces can be semantically significant. For example, a body file ending with a hard break:

```markdown
Final line··
```

loses the hard break when written. This conflicts with the new contract that `--body-file` supplies a complete structured Markdown body. It also means the content is not merely placed below the preamble; it is modified.

This behavior predated the wrapper fix, but formalizing `--body-file` as structured Markdown makes it part of the contract being changed here.

Use distinct normalization for structured bodies:

```rust
if let Some(d) = args.description {
    if args.structured_body {
        body.push_str(d.trim_end_matches(['\r', '\n']));
    } else {
        body.push_str(d.trim_end());
    }
    body.push('\n');
}
```

Better still, move this into an `InitialBody` abstraction so rendering behavior is mode-specific rather than controlled by a nearby boolean.

Add a regression with a body ending in two spaces followed by a newline.

### Low — agent-facing documentation contradicts schema-driven stub generation

These files correctly state that required H2 stubs may be appended by repository schema:

- `.claude/skills/issue/SKILL.md:544-547`
- `.codex/prompts/issue.md:540-543`
- `crates/issuectl-core/templates/issue-skill.md:544-547`
- `crates/issuectl-core/templates/issue-prompt.md:540-543`

Later, the same documents categorically say:

> For epics, add `## Goal`, `## Issues`, `## Phases`, and `## Notes` sections — the CLI does not write these.

That is false when those sections are configured as required schema sections; `do_new_locked` appends their stubs. The wording should distinguish default rendering from schema augmentation:

```markdown
For epics, ensure `## Goal`, `## Issues`, `## Phases`, and `## Notes`
exist. The default renderer does not add them, although a repository schema
may generate required-section stubs.
```

The same qualification should be considered for the preceding bug-section guidance.

## Questionable decisions

### Boolean mode flag preserves the underlying invalid-state design

`crates/issuectl-core/src/mutate/new_issue.rs:65-72` and  
`crates/issuectl-core/src/write.rs:344-350` now represent body semantics as:

```rust
description: Option<String>,
structured_body: bool,
```

This permits meaningless or contradictory states:

- `description: None, structured_body: true`
- structured Markdown stored in a field called `description`
- free text accidentally marked structured by any future constructor
- a new creation path silently receiving `false` through `Default`

The diff had to update intake, core API, recurrence, import, tests, and CLI construction solely to maintain this cross-field invariant. That is evidence that the representation is too weak.

Use a sum type:

```rust
pub enum InitialBody {
    Empty,
    FreeText(String),
    StructuredMarkdown(String),
}
```

or a borrowed write-layer equivalent:

```rust
pub enum InitialBodyRef<'a> {
    Empty,
    FreeText(&'a str),
    StructuredMarkdown(&'a str),
}
```

Then rendering becomes exhaustive:

```rust
match args.body {
    InitialBodyRef::Empty => body.push_str("## Description\n\n"),
    InitialBodyRef::FreeText(text) => {
        body.push_str("## Description\n\n");
        body.push_str(text.trim_end());
        body.push('\n');
    }
    InitialBodyRef::StructuredMarkdown(markdown) => {
        body.push_str(markdown.trim_end_matches(['\r', '\n']));
        body.push('\n');
    }
}
```

This would also make the intentional semantics of intake, recurrence, import, and `new_api` explicit at their call sites rather than encoding them as `false`.

### `description` is now a misleading transport field

At `crates/issuectl/src/cmd/runtime.rs:586-601`, the result of `read_body_file_arg` is assigned to `description`, then interpreted according to a separate flag. The name no longer describes the value. This makes future validation and warning code easy to apply incorrectly—for example, code may assume the value is prose because it is named `description`.

At minimum rename the internal field to `initial_body` or `body_content`. The CLI surface can retain `--description`.

### Required-section detection reparses the complete serialized item

`crates/issuectl-core/src/mutate/new_issue.rs:351-359` now correctly uses the canonical splitter, but the architecture still renders frontmatter and body into one string, reparses it to recover the body, scans it, and then mutates the rendered output.

That works, but it is unnecessary serialization/reparsing inside a write path. A cleaner renderer would produce separate frontmatter and body components, apply required-section completion to the body, then serialize once. This would eliminate the class of delimiter bugs that motivated the splitter change.

The current canonical splitter is still a substantial correctness improvement over `split("---\n\n")`; this is a structural recommendation, not a blocker.

## Missing considerations

### No coverage for empty `--body-file`

The new behavior for an empty file is materially different:

- without required schema sections, the item contains only the generated H1/source preamble;
- with required sections, only schema-generated stubs follow it;
- no `## Description` is generated merely because the file is empty.

That may be the intended meaning of a complete structured body, but it is not pinned by a test or explicitly documented.

Add black-box coverage such as:

```rust
#[test]
fn empty_body_file_does_not_generate_description_wrapper() {
    // Create empty body.md.
    // Assert output ends after generated H1, unless schema requires sections.
}
```

Also add a schema variant that verifies required `Description` is appended exactly once for an empty body.

### Insufficient coverage of required-section completion through the CLI

`structured_body_with_horizontal_rule_does_not_duplicate_required_sections` in  
`crates/issuectl-core/src/mutate/new_issue.rs:696-719` is useful, but only exercises the core mutation directly.

There is no black-box test proving all of these behaviors together:

1. `--body-file` sets structured mode;
2. a body section after `---` is detected;
3. only genuinely missing schema sections are appended;
4. no wrapper is introduced;
5. optional `--source` remains before the structured content.

The runtime wiring is the feature boundary. Add one integration case with a schema such as:

```yaml
body_sections:
  bug: [Description, Expected, Quick Test]
```

and an input containing `Description` and `Expected` separated by a horizontal rule. Assert that only `Quick Test` is appended.

### Preservation paths are changed mechanically but not behaviorally pinned

The following constructors correctly set `structured_body: false`:

- `crates/issuectl-core/src/mutate/intake.rs:321-325`
- `crates/issuectl-core/src/mutate/new_api.rs:193-197`
- `crates/issuectl-core/src/recurrence.rs:515-519`
- `crates/issuectl-core/src/transfer.rs:217-221`

That preserves behavior by inspection. However, there are no focused regressions proving that body text beginning with `## Description` in each path still remains nested under the generated wrapper as intentionally specified.

Not every path needs a large integration test, but at least constructor-level tests should pin the distinction. Otherwise a future cleanup may “simplify” these `false` values to `true`, especially because the field name gives no domain reason for the difference.

### Plain structured bodies are only partially tested

`new_body_file_writes_markdown_below_heading` verifies prose input has no wrapper, which is good. Missing cases include:

- empty content;
- leading blank lines;
- CRLF input;
- terminal Markdown hard-break spaces;
- content consisting only of a horizontal rule;
- a structured body with no H2 headings under a schema requiring H2 sections.

These cases determine whether “directly below” means byte-preserving, normalized, or merely ordered. The implementation currently mixes LF-generated preambles with CRLF body content, and the contract does not state whether this is acceptable.

### The horizontal-rule regression does not directly test `item_text::split`

The mutation test proves the observed result, but `crates/issuectl-core/src/item_text.rs` has no focused regression showing that a serialized item body containing a later `---` remains entirely in `Split.body`.

Add:

```rust
#[test]
fn body_horizontal_rule_does_not_truncate_body() {
    let s = split(
        "---\nstatus: open\n---\n\n\
         # T\n\n## Description\n\nA\n\n---\n\n## Expected\n\nB\n",
    );
    assert!(s.body.contains("## Expected\n\nB"));
}
```

This pins the canonical primitive now relied upon by required-section detection.

## Risks

### Future constructors can silently select the wrong semantic mode

Because `NewArgs::default()` sets `structured_body: false`, any new path using struct update syntax or default construction will automatically treat content as free text. That is safe for historical compatibility but dangerous for future file-based or structured inputs: the code will compile and recreate the duplicate-wrapper bug.

An enum forces the caller to choose a body mode and removes this maintenance trap.

### “Complete structured Markdown” may be interpreted as full-document Markdown

The implementation prepends its own H1 and frontmatter. A user passing a document containing its own H1 will get two H1 headings; a user passing YAML frontmatter will get that frontmatter embedded in the body.

The docs say the content is placed below the generated H1, which mostly disambiguates this, but “complete structured Markdown body” should explicitly say not to include issue frontmatter or the generated title heading. Agent-generated files are especially likely to include a title unless instructed otherwise.

Suggested wording:

```markdown
The file should contain the body content only: do not include YAML
frontmatter or the issue's H1 title. H2 sections are accepted directly.
```

### Required-section scanning remains exact-shape dependent

`body_sections::all_h2_sections` recognizes only unindented ATX headings beginning exactly with `"## "`. Therefore schema completion will append a stub when the body uses variants such as:

```markdown
## Description #
##  Description
Description
-----------
```

This is existing scanner policy rather than a regression in this diff, but structured body files increase exposure because callers now supply complete Markdown structures directly. The documentation should either specify that required sections must use canonical `## Name` headings or the scanner should use a Markdown parser.

### Tests use substring counts rather than structural heading counts

Assertions such as:

```rust
content.matches("## Description").count()
```

in `new_issue.rs`, `write.rs`, and `cli_new.rs` can count occurrences in code fences or prose. The current fixtures are controlled, so these tests pass for the right reason, but this pattern is fragile.

Where testing section cardinality, use `all_h2_sections` plus a heading-count helper, or parse line-by-line outside fences. Keep direct `ends_with` assertions for exact rendering tests.

## What works well

- `crates/issuectl/src/cmd/runtime.rs:586-624` preserves the input source before consuming `body_file`, so file/stdin and inline descriptions are correctly distinguished.
- All shown historical creation paths explicitly retain free-text mode.
- Replacing `split("---\n\n").nth(1)` with `item_text::split(&render).body` fixes the horizontal-rule truncation bug rather than merely patching the test fixture.
- The source-preamble regression verifies the exact ordering that is easy to break.
- The black-box stdin test is appropriately placed in `tests/`; it validates process-level piping and clap/runtime wiring that a core unit test cannot observe.
- Template and dogfooded copies shown in the diff remain synchronized.
- No lock, schema-validation, or mutation-boundary bypass was introduced. The CLI still routes creation through the core mutation path.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_6c3629d4057240d29550996b5ea5ba97]

# Review: `create --body-file` structured-body fix

## Problems found

### P1 — The change is broader than the bug, and the CHANGELOG misrepresents it (`CHANGELOG.md`, `crates/issuectl-core/src/write.rs:428-432`)

The reported defect was a *duplicate* `## Description` when the file already began with one. The fix removes the wrapper for **every** body file, including plain-prose files with no headings at all. For a user whose `notes.md` is two paragraphs of free text (the exact example in the skill: `--body-file notes.md` for "Bulk export"), the output silently changes from:

```
# Bulk export

## Description

<prose>
```

to:

```
# Bulk export

<prose>
```

and if a schema requires `Description`, `do_new_locked` then appends an **empty** `## Description` stub *after* the prose — the prose ends up as headingless preamble and the required section is an orphaned empty stub trailing it. That is arguably worse than the pre-fix state for the plain-prose case: before, the prose landed *inside* the Description section.

Filing this under `### Fixed` as "no longer injecting an empty duplicate heading" understates it. This is a behavior change for all existing `--body-file` callers, not a narrow duplicate-heading fix. It belongs under `### Changed` with the plain-prose consequence stated.

### P2 — No validation or warning when the "structured" contract is violated (`crates/issuectl/src/cmd/runtime.rs:583-601`)

The new contract says the file is "a complete structured Markdown body," but nothing checks it. A body file containing zero H2 headings is accepted silently and produces the degraded output in P1. At minimum, `do_new_locked` should warn (via the existing `warnings` channel — the same channel already used for reserved-heading warnings) when `structured_body` is set and the content contains no H2 sections, since the required-section stub logic will then misfire in the way described above.

### P3 — Two-field representation permits an invalid state (`crates/issuectl-core/src/mutate/new_issue.rs:65-70`, `crates/issuectl-core/src/write.rs:344-348`)

`description: Option<String>` + `structured_body: bool` allows `(None, true)`: a body of just `# Title\n\n` with no Description heading and no content — constructible by any caller of the public `NewArgs`. The renderer at `write.rs:431-434` doesn't guard against it. Seven call sites now carry `structured_body: false` boilerplate that a future caller can get wrong with no compiler help. Since `issuectl-core`'s `pub` items are explicitly not a semver contract, replace both fields with an enum:

```rust
pub enum InitialBody {
    None,
    FreeText(String),   // wrapped under generated `## Description`
    Structured(String), // placed verbatim below preamble
}
```

Render becomes an exhaustive `match`; the invalid state is unrepresentable; the seven constructor sites become self-documenting instead of `structured_body: false` noise.

### P4 — Same-shaped defect likely persists in `intake file` (unverified)

`intake.rs:321` maps `req.body → description` with `structured_body: false`. The field name `body` (not `description`) suggests intake filing may accept full-body content — possibly its own `--body-file`. If so, the exact duplicate-`## Description` bug this diff fixes still exists on that path, and the fix is incomplete. The task statement says intake free-text semantics are "intentionally preserved," but preservation is only correct if intake's input is genuinely free text. Cannot confirm from the attached files. See Context request.

### P5 — Empty-file edge unverified (`runtime.rs:600`)

Inline `--description` is guarded by `parse_non_empty` (`cmd/mod.rs:626`). `--body-file` routes through `read_body_file_arg`, whose empty/whitespace-only handling is not in the attached evidence. If it accepts empty input, `structured_body = true` + `description = Some("")` yields `# Title\n\n\n` plus schema stubs — a shape the old code at least gave a Description heading. See Context request.

### P6 — Missing test: structured body + *missing* required section

The templates now promise: "A repository schema may still append stubs for any required H2 sections the content omits." No test exercises that promise for the structured path — `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` (`new_issue.rs:697`) only covers the all-sections-present case. Add a test with schema `[Description, Expected]` and a body file containing only `## Description`, asserting exactly one `## Expected` stub is appended (and, per P1, note where it lands relative to the content).

## Questionable decisions

- **Test placement** (`crates/issuectl/tests/cli_new.rs:123-215`): `body_file_is_structured_markdown_without_description_wrapper` and `inline_description_retains_generated_description_heading` largely re-test rendering already pinned inline in `write.rs` and `new_issue.rs`. AGENTS.md restricts `tests/` to behavior no inline test can observe. The genuinely process-level bits are the stdin path (`--body-file -`) and the `body_file.is_some() → structured_body` wiring in `dispatch_primary`; the inline-description test in particular adds nothing an inline test doesn't cover. Defensible for the wiring, but it's convention drift.
- **Core doc comment naming a CLI flag** (`new_issue.rs:66-69`): "The CLI sets this for `create --body-file`" couples core documentation to binary-crate surface. Harmless today, stale the day the flag is renamed. Describe the semantics, not the caller.
- **Case-sensitive section matching**: `all_h2_sections` matches headings verbatim (`body_sections.rs`, `all_h2_sections`), so a body file with `## description` still gets a `## Description` stub appended. Pre-existing semantics, but the new "your file's sections satisfy the schema" contract makes the exact-case requirement more consequential and it's documented nowhere in the updated skill text.

## Missing considerations

- **Stub placement documentation**: the skill says stubs "may still append" but not that they append *after* the file content, which matters for the P1 failure mode.
- **Export/import asymmetry**: `transfer.rs:217` keeps `structured_body: false`. If exported records carry anything resembling a full body in `description`, re-import now diverges from what `create --body-file` would produce from the same content. Preserved-by-design per the task, but the divergence between "the CLI's body-file path" and "the import path" is a growing seam with no test pinning either side's shape.
- **Deadlock hardening in `run_with_stdin`** (`cli_new.rs:58-80`): stdout/stderr are not drained while writing stdin. Fine for the current tiny payloads; a future test piping >64 KiB deadlocks. A comment noting the limit would prevent that footgun.

## Risks

- Downstream agents following the *old* skill text (installed copies in other repos lag releases) will keep sending free-text body files and get the P1 degraded shape with no warning. The only mitigation is P2's missing warning.
- Any future creation path that forgets `structured_body` compiles fine and silently gets free-text semantics (or vice versa). P3's enum removes this class of error.

## What works well

- The `item_text::split` switch in `new_issue.rs:351-358` is a genuine correctness fix: the old `split("---\n\n").nth(1)` truncated at a body horizontal rule and would append duplicate stubs; the new path uses the canonical fence-aware splitter, and the regression test pins exactly that failure. 
- All four skill/template copies and the CLI help were updated in lockstep, matching the AGENTS.md sync rule; the wording is consistent across copies.
- The three-way test coverage of preamble ordering (title / source / content) via `ends_with` assertions is appropriately structural rather than substring-based.

## Context request

- Kind: artifact
  Need: Source of `read_body_file_arg` in `crates/issuectl/src/cmd/` (or wherever it lives).
  Why: Determines whether an empty/whitespace-only body file reaches the renderer with `structured_body: true` (P5); if unvalidated, that's an additional defect to file.
- Kind: artifact
  Need: The `intake file` CLI definition (its clap struct in `crates/issuectl/src/cmd/`) and `FileRequest` in `crates/issuectl-core/src/mutate/intake.rs`, plus the `issue-new` skill template.
  Why: If intake filing accepts full-body/file input, the duplicate-`## Description` defect persists on that path and the fix is incomplete (P4); if it's free-text only, `structured_body: false` there is correct as-is.
- Kind: artifact
  Need: The export record format / where `ImportRecord.description` is populated in `crates/issuectl-core/src/transfer.rs`.
  Why: Determines whether export→import round-trips now produce a different body shape than `create --body-file` for the same content, which would warrant either a doc note or a follow-up issue.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_16db540a340743ca90ae1a10f88688e1]

## Problems found

### 1. `--body-file` is now a silent breaking CLI change for bodies without H2 sections

`crates/issuectl/src/cmd/runtime.rs:583-590` derives semantics from the I/O source, not the content:

```rust
let structured_body = body_file.is_some();
```

`crates/issuectl-core/src/write.rs:428-434` then removes the generated `## Description` for every file input.

For existing users who used `--body-file` for free-text notes, the on-disk result changes from:

```markdown
# Title

## Description

First paragraph.

Second paragraph.
```

to:

```markdown
# Title

First paragraph.

Second paragraph.
```

The prose now sits outside any section. If a schema requires `## Description`, the stub appended by `do_new_locked` lands *after* that prose, so the description section is empty and the actual content is orphaned above it.

This is a behavior change to the CLI contract, but it is recorded under `### Fixed` in `CHANGELOG.md`. That undersells the change; an existing script or downstream agent can produce materially different issues after upgrading. At minimum this should be `### Changed`, and likely needs a warning when a `--body-file` input contains no H2 sections, since that is the strongest signal someone is feeding old-style free text.

Suggested guard in `runtime.rs` after reading the file:

```rust

## Authoritative preserved review — recovered Anthropic context and complete DeepSeek independent response

[thread_id:group_7c0d877165c1491c81792d4e5b289602]

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_6c3629d4057240d29550996b5ea5ba97]

# Revised review after supplied context

## Conclusions that change

### P5 (empty body file) — **withdrawn as a defect**
`read_body_file_arg` (`crates/issuectl/src/cmd/write.rs`) rejects empty/whitespace-only input (`bail!("--body-file {} is empty", …)`), and its trailing-only trim matches the renderer's `trim_end`. The CLI cannot produce `structured_body = true` with an empty body. What remains is only the narrower P3 point: the state `(description: None, structured_body: true)` is still constructible through the public core API by any non-CLI caller, and nothing in `render_new_item_from_fm` guards it. That stays a constructor-invariant weakness, not a live CLI bug.

### P4 (intake) — **downgraded for intake, but confirmed and *upgraded* for import**

**Intake:** `FileRequest.body` is defined as free text and the `/issue-new` skill's contract is verbatim capture, so `structured_body: false` at `intake.rs:321` is consistent with the stated design. The residual duplicate-heading exposure — a verbatim report file that itself begins with `## Description` — is real but low-probability, and arguably correct under "capture verbatim, wrapped as the report." I withdraw the "likely defect" claim. What *does* stand is a new inconsistency the fix created: the flag name `--body-file` now means **structured body, no wrapper** on `create` and **free text, wrapped** on `intake file`. Two commands, one flag name, opposite body semantics. The `/issue-new` skill (`templates/issue-new-skill.md`, step 1) says "Prefer `--body-file <path>`" with no hint that its semantics differ from `create --body-file`, whose new semantics the same release's `/issue` skill documents prominently. An agent that has read both skills has to notice the divergence itself. At minimum the intake skill and `intake file --body-file` help text need a sentence stating the body is placed under a generated `## Description`.

**Import: the fixed defect is confirmed live on the import path.** This is now grounded, not speculative:

- `transfer.rs` module doc: JSON intake "reads both issuectl's own JSON export and hand-written arrays."
- `ImportRecord.description` carries `#[serde(alias = "body")]`, and issuectl's own JSON export serializes the full `Issue` including `body` — which for every issuectl-created issue is a structured document (`## Description\n\n…`, plus any `## Comments`/`## Reproduction` sections). The test `export_json_round_trips_through_import` shows exactly this: `description` receives `"## Description\n\nSomething broke."`.
- `into_new_args` sets `structured_body: false` (`transfer.rs:217`), so `render_new_item_from_fm` prepends the generated `## Description\n\n` wrapper.

Result: `issuectl export --format json | issuectl import` produces, for **every** re-imported issue, `# Title\n\n## Description\n\n## Description\n\n…` — the identical empty-duplicate-heading defect this diff fixed for `create --body-file`. The module doc's "content-level, not byte-faithful" disclaimer covers *dropped fields* (status, dates, commits); it does not cover structurally corrupting the body that *is* carried. This should have been part of this fix or filed as a follow-up issue; the CHANGELOG's "Fixed" entry reads as if the duplicate-heading defect is gone, when it survives on the self-export round-trip — the one import input the module explicitly promises to handle.

Practical remediation: serde aliases can't distinguish `body` from `description` at deserialize time, so either (a) split the fields (`body` → structured, `description` → free text, `body` wins when both present), or (b) mark records from issuectl-shaped exports (they carry `slug`/`status` keys the record ignores today) as structured. Option (a) is cleaner and matches the create-side split exactly.

**Unresolved gap:** I could not verify from the attached files whether the parser strips the `# <title>` H1 from `Issue.body` on load. If it does not, the export→import corruption is worse — a doubled H1 (`# New Title … # Old Title`) in addition to the doubled `## Description`. The Description duplication is confirmed either way.

## Conclusions that stand unchanged

- **P1** — the fix is broader than the bug: plain-prose `--body-file` input loses its Description wrapper, and a schema-required `## Description` is appended as an empty stub *after* the prose. Still a `### Changed`-worthy behavior change misfiled under `### Fixed`.
- **P2** — no warning when a "structured" body contains zero H2 headings; the warnings channel exists and should be used.
- **P3** — replace `description: Option<String>` + `structured_body: bool` with an `InitialBody` enum; seven `structured_body: false` boilerplate sites (now including the import-path one that turned out to be wrong) demonstrate exactly the maintenance failure mode predicted.
- **P6** — no test covering structured body + a *missing* required schema section (the stub-append promise in the templates is untested for the structured path).
- Questionable decisions (test placement in `tests/cli_new.rs`, core doc comment naming a CLI flag, case-sensitive section matching) and the `run_with_stdin` pipe-buffer note all stand.
- What works well stands: the `item_text::split` fix, template/help synchronization, and structural ordering assertions.

## Revised priority order

1. Import path retains the duplicate-`## Description` defect on issuectl's own export round-trip (`transfer.rs:217`) — confirmed, file/fix it.
2. P1 plain-prose regression + stub misplacement, with matching CHANGELOG correction.
3. P3 enum refactor (would have prevented finding 1).
4. P2 no-H2 warning; P6 missing-section test.
5. `--body-file` semantic divergence between `create` and `intake file` — document in the intake skill and CLI help.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_16db540a340743ca90ae1a10f88688e1]

## Critical review: `create --body-file` structured-body mode

### Problems found

#### 1. Silent breaking change for existing free-text `--body-file` users; CHANGELOG misclassifies it

`crates/issuectl/src/cmd/runtime.rs` derives the new mode from the I/O source:

```rust
let structured_body = body_file.is_some();
```

`crates/issuectl-core/src/write.rs` then omits the generated `## Description` for every file input:

```rust
if !args.structured_body {
    body.push_str("## Description\n\n");
}
```

Before this change, `create --body-file notes.md` with free text produced:

```markdown
# Title

## Description

First paragraph.

Second paragraph.
```

After, the same input produces:

```markdown
# Title

First paragraph.

Second paragraph.
```

The prose is now outside any section. For a repository whose schema requires `Description`, `do_new_locked` appends an empty `## Description` stub *after* that prose, orphaning the actual content above the section, because the file content has no `## Description` to satisfy the required-section check.

This is a deliberate contract change, and the skill templates document the new intended behavior, but the CHANGELOG files it under `### Fixed`:

```markdown
- `create --body-file` now places complete structured Markdown beneath the
  generated title/source preamble without injecting an empty duplicate
  `## Description` heading.
```

The fix targets the duplicate-heading case where the file *does* begin with `## Description`. It also changes the behavior for every file that does not. Any existing script, migration pipeline, or agent that used `--body-file` for plain free text silently gets materially different issue bodies after upgrading. The entry belongs under a breaking/`### Changed` heading, and the transition deserves at least a warning when a `--body-file` input contains no H2 sections (the strongest signal the caller is supplying old-style free text).

#### 2. Import of issuectl’s own JSON export creates duplicate H1 and nested `## Description`

`crates/issuectl-core/src/transfer.rs` sets:

```rust
structured_body: false,
```

in `ImportRecord::into_new_args`. For foreign inputs (GitHub, hand-written JSON) that is the intended free-text behavior. But issuectl’s own JSON export stores the complete body, including the H1 and all H2 sections, in the `body` key. Re-importing that record through `do_new` now renders:

```markdown
# New imported title

## Description

# Original title

## Description

Something broke.
```

This is a pre-existing import bug, but the new `structured_body` mode is exactly the mechanism that could have fixed the own-format round-trip. The diff hardcodes `false` uniformly and leaves the nested-section bug in place. At minimum, an own-format JSON import should be tested against the new field and either route through `structured_body: true` or be documented as intentionally lossy beyond the existing module note.

#### 3. `duplicate_precheck` builds a candidate body that no longer matches either render mode

`crates/issuectl/src/cmd/write.rs`:

```rust
body: args.description.clone().unwrap_or_default(),
```

`NewArgs` gained `structured_body`, but this pre-creation duplicate check still uses the raw description as the candidate body. The stored body for `--body-file` is now `# Title` + optional source + raw structured markdown; for inline `--description` it is `# Title` + `## Description` + raw text. The candidate used for similarity scoring matches neither. This mismatch predates the diff, but adding a second render mode widens the divergence, and the new mode did nothing to correct the scoring input. Body-overlap scores for structured files will systematically under-represent stored content.

#### 4. Same `--body-file` flag now has divergent semantics across subcommands

`create --body-file` now means “complete structured Markdown, placed directly under the title/source.” `intake file --body-file` still means “verbatim report body, wrapped in generated `## Description`” — see `crates/issuectl-core/src/mutate/intake.rs`, where `description: req.body` is paired with `structured_body: false`, and `crates/issuectl-core/templates/issue-new-skill.md`, which still documents intake `--body-file` as free verbatim text. The two skill families now use the same flag name for opposite rendering policies. This is a real usability trap for agents and users who reasonably assume `--body-file` means the same thing across `issuectl create` and `issuectl intake file`.

### Missing considerations

- **No test for a structured body that omits a schema-required section.** The documented behavior in the templates is: “A repository schema may still append stubs for any required H2 sections the content omits.” The new tests cover the case where required sections are present. The stub-append path for a structured file missing `Description`/`Expected` is untested.
- **No test for inline `--description` containing a horizontal rule plus schema-required sections.** The splitter fix applies equally to free-text descriptions, but only the structured-body path is pinned. If the old `render.split("---\n\n")` bug could occur for inline descriptions, that regression remains unverified.
- **No validation that a structured body file does not contain its own H1.** A file beginning with `# Something` will produce an item with two H1s. The CLI accepts this silently. Whether that is allowed should be explicit, and likely rejected or warned.
- **No fence validation for `create --body-file`.** `--body-file` is not routed through `validate_message`; a structured body ending with an unclosed code fence can swallow a later schema-required section stub during `do_new_locked`. The note path rejects unclosed fences, but the create body path does not.

### Risks

- Existing installed perishable agent skills are not automatically updated. The templates are synchronized in this commit, but a running agent may have an older `/issue` skill installed locally. It will keep telling the agent to submit free-text `--body-file` content, producing the new header-less bodies without warning. There is no CLI check that the installed skill version matches the binary version.
- `create --json` does not expose a `body_mode` or `structured` field. Scripts that consume `--json` to read `path` and then inspect the generated `item.md` will observe changed bytes with no structured signal. This is part of the same breaking-change risk as finding 1.
- The `structured_body` flag on the public `NewArgs` struct in `issuectl-core` is a rendering-policy knob embedded in the domain constructor. Every new `NewArgs` construction site must remember to set it; if a future author adds a creation path and omits it, they silently get the free-text wrapper. The `Default` with `false` is safe today, but the affordance is easy to misuse.

### What works well

- Replacing `render.split("---\n\n").nth(1)` with `crate::item_text::split(&render).body` is a meaningful correctness fix. The new splitter is fence-aware and uses the canonical frontmatter rules, so horizontal rules in the rendered body no longer truncate required-section detection. The added core test `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` pins this behavior well.
- All existing constructors were updated consistently; intake, new API, recurrence, and import all retain the historical free-text behavior as the task requires.
- The integration tests cover the principal CLI modes: file, stdin with `--source`, inline description parity, and the revised structural ordering in `new_body_file_writes_markdown_below_heading`.
- Documentation and both skill-format copies were updated in the same commit, following the AGENTS.md synchronization rule.

**Net:** the splitter change is solid, but the CLI contract change is under-documented and under-guarded. Finding 1 is the critical item; the rest are pre-existing gaps the new mode either fails to fix or makes more likely to trip.

## Authoritative preserved review — cross-review round 1 (all four complete)

[thread_id:group_59785163de1b4a8995b94634ccd7c474]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_ee0b71452aaa4d3d893db7cdeb51f306]

I am adjusting my previous assessment based on a deeper analysis of the provided files and the observations from the other agents.

### 1. Agreement and Disagreement with Other Reviewers (and Myself)

**Retraction of my previous claim (Blank Line Missing):**
I retract my previous claim that appended schema stubs lack a blank line. `write.rs:433` pushes a single `\n`, but `do_new_locked` prepends its schema stubs with `\n## `. The combination safely results in `\n\n`. My prior conclusion was incorrect.

**Agree: Missed `import` Header Duplication (Anthropic / DeepSeek)**
Both agents correctly observed that `transfer.rs` is blindly hardcoding `structured_body: false` for `ImportRecord`. When `issuectl export` generates JSON, it dumps the *entire* parsed Markdown body into the `body` field—which already includes the `## Description` header. When `issuectl import` reads this back, it maps it to `description`, treats it as free-text (`structured_body: false`), and `write.rs` prepends *another* `## Description`. The PR introduced the exact mechanism to fix this (`structured_body = true`) but completely missed applying it to JSON imports.

**Agree: Primitive Obsession (OpenAI / Anthropic / DeepSeek)**
All agents noted that passing `structured_body: bool` alongside `description: Option<String>` is a poor modeling choice. It forces every other subsystem (Intake, Recurrence, Transfer/Import) to explicitly initialize an invalid/dummy state, rather than using a type-safe `enum InitialBody { FreeText(String), Structured(String) }`.

**Disagree: "Breaking Change" for Plain-Prose Files (Anthropic / DeepSeek)**
Several agents complained that passing a plain text file without Markdown headers now leaves it "floating" without a `## Description` wrapper, calling this a regression. I reject this. The PR explicitly changes the contract of `--body-file` to insert content *exactly* as written. If a user supplies a plain text file via `--body-file`, they are explicitly bypassing the wrapper. This is the intended feature, not a bug.

---

### 2. New Findings: Accidental Correctness & Pipeline Confusion

**CRITICAL: The fix for `---\n\n` is completely nonsensical and works by accident.**
*`crates/issuectl-core/src/mutate/new_issue.rs:354-358`*
```rust
// Use the canonical frontmatter splitter. A structured Markdown body
// can legitimately contain ---\n\n horizontal rules; a plain
// string split would truncate at one and falsely append duplicate
// required-section stubs for headings that follow it.
let body_only = crate::item_text::split(&render).body;
```
The developer fundamentally misunderstands their own rendering pipeline. 
The variable `render` is generated by `crate::write::render_new_item(&args)`. That function **does not emit frontmatter**. It emits a string starting with `# <title>\n\n`. Frontmatter is only prepended *much later* in `do_new_locked` when constructing the final disk payload. 

Because `render` starts with `# Title`, `item_text::split(&render)` immediately hits this logic:
```rust
let trimmed = after_bom.trim_start();
if !trimmed.starts_with("---") {
    return Split { frontmatter: None, body: text };
}
```
`item_text::split` instantly bails and returns the *entire* `render` string untouched. It never scans the body for `---\n\n`. 

The test `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` passes **only because you removed the old naive `.split("---\n\n").nth(1)`**, which was erroneously truncating the text. Invoking the frontmatter splitter on a string without frontmatter is dead code masquerading as a structural fix. The correct fix is simply `let body_only = &render;`.

---

### 3. Priority Adjustment: Top 3 Issues

1. **Accidental Correctness via Misunderstood Pipeline (Blocker)**
   The use of `item_text::split(&render).body` is technically a no-op that masks the author's lack of understanding of the data flow. The comment and the function call must be removed. `body_only` should simply be `&render`, as `render` at this stage is exclusively the Markdown body.
2. **Missed Fix for Import JSON Duplication (Blocker)**
   `issuectl export` -> `issuectl import` is currently broken for descriptions because `transfer.rs:217` hardcodes `structured_body: false`. Imported JSON records that originate from `issuectl` already contain their structural headings. The import layer must flag these as structured bodies to prevent `write.rs` from injecting duplicate headers.
3. **Leaky API Surface (Design Debt / Refactor)**
   `NewArgs` relies on a disjointed `description: Option<String>` and `structured_body: bool`. This must be refactored into a strongly typed `enum IssueBody` to prevent unrepresentable states in the CLI, Intake, Recurrence, and Import pipelines. (This can be treated as immediate follow-up design debt if necessary to unblock the bug fix).

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d3af4b40a7e14b92b448624d7e755476]

# Review

## Blockers

### 1. The shipped and dogfooded agent documentation is no longer synchronized

The current diff changes only:

- `crates/issuectl-core/templates/issue-skill.md`

It does **not** change:

- `crates/issuectl-core/templates/issue-prompt.md`
- `.claude/skills/issue/SKILL.md`
- `.codex/prompts/issue.md`

This violates the explicit synchronization rule in `AGENTS.md`. The Claude/Codex bodies are required to remain identical after stripping frontmatter, and dogfooded copies must match their templates.

This is not documentation polish. The Codex agent contract would continue describing the old behavior, and the repository’s synchronization test should fail.

Required fix:

```sh
tail -n +5 crates/issuectl-core/templates/issue-skill.md \
  > crates/issuectl-core/templates/issue-prompt.md

issuectl skill install --agent all --force
```

Then verify all four files are present in the diff. If running the installed binary would regenerate stale content, use the repository’s documented build/install workflow instead, but the final bytes must match.

### 2. A known `NewArgs` struct literal appears to be missing the new field

The earlier evidence in this review showed a `NewArgs` literal at:

- `crates/issuectl/src/cmd/cli_tests.rs:133`

That literal required:

```rust
structured_body: false,
```

The current diff no longer includes that update. Unless the underlying test was independently changed to use `..Default::default()`, `cargo test --workspace` will fail to compile with a missing-field error.

This is the direct cost of adding a required field to a widely constructed struct. Restore the field or change the helper to construct from `Default` intentionally.

Run at minimum:

```sh
cargo test --workspace
```

Given the documented gate, all repository checks remain required before landing.

## Production-code assessment

I do not see a demonstrated production correctness blocker in the creation path itself.

The core behavior is wired correctly:

- `crates/issuectl/src/cmd/runtime.rs:589` captures `body_file.is_some()` before consuming the option.
- File and stdin body sources become structured bodies.
- Inline `--description` remains free text.
- `crates/issuectl-core/src/write.rs:428-435` suppresses only the generated wrapper in structured mode.
- Intake, core API, recurrence, and import explicitly remain in free-text mode.
- Required-section detection now uses `item_text::split`, avoiding truncation at a horizontal rule in the issue body.

The production fix matches the declared create contract.

## Incorrect or exaggerated findings from other reviews

### Plain-prose body files are not an accidental compatibility bug

The revised contract explicitly says `--body-file` is a complete structured Markdown body written directly below the generated preamble. The integration test at `crates/issuectl/tests/cli_new.rs:1247` explicitly pins a plain-prose file as wrapper-free.

Therefore this behavior:

```markdown
# Title

Plain prose
```

instead of:

```markdown
# Title

## Description

Plain prose
```

is intentional under the declared contract. Calling it a defect ignores both the specification and regression test.

A no-H2 warning would also contradict valid Markdown use cases. A structured body need not contain H2 headings unless the repository schema requires them.

### Empty body-file handling is already settled

`crates/issuectl/src/cmd/write.rs` shows that `read_body_file_arg` rejects empty and whitespace-only input:

```rust
let body = body.trim_end();
if body.is_empty() {
    bail!("--body-file {} is empty", path.display());
}
```

My earlier concern about undefined empty-file rendering is withdrawn. The input never reaches the renderer through the CLI.

A focused test for this validation would still be useful, but it is not an unresolved production semantic.

### Trailing whitespace removal is pre-existing, deliberate behavior

`read_body_file_arg` intentionally uses `trim_end()`, and its comment explicitly documents alignment with `body set --from-file`. `render_new_item_from_fm` also already used `trim_end()` before this patch.

This can remove Markdown-significant terminal spaces, but it is not introduced by this fix. It should not block a create-only duplicate-heading correction unless the project separately decides body-file ingestion must become byte-faithful.

### Intake divergence is intentional, not evidence of a bug

The task explicitly requires intake to preserve historical free-text semantics. This line is therefore correct:

- `crates/issuectl-core/src/mutate/intake.rs:324`

```rust
structured_body: false,
```

The same applies to `new_api`, recurrence, and import.

The fact that two commands may both have a spelling like `--body-file` does not override the explicitly stated domain distinction. Any broader CLI consistency redesign belongs in separate work.

### Own JSON export → import duplication is real but pre-existing and out of scope

`ImportRecord.description` accepts exported `body`:

```rust
#[serde(default, alias = "body")]
pub description: Option<String>,
```

and import deliberately sets:

```rust
structured_body: false,
```

Thus an exported body beginning with `## Description` can be imported beneath another generated `## Description`.

That behavior existed before this patch because creation previously generated the wrapper unconditionally. This diff intentionally preserves import semantics, so changing it here would violate scope rather than fix the requested path.

It is legitimate follow-up debt, especially given the misleading test name `export_json_round_trips_through_import`, which tests parsing rather than actual creation. It is not a blocker for this create-only patch.

### Duplicate precheck mismatch is not demonstrated

`duplicate_precheck` uses:

```rust
body: args.description.clone().unwrap_or_default(),
```

For structured bodies, that is exactly the user-provided body. For inline descriptions, it continues using the same free-text content it used before. The generated wrapper is structural boilerplate and should not materially contribute to duplicate scoring.

No evidence shows that duplicate scoring depends on the generated `## Description` token or that this patch introduces a mismatch. Do not change duplicate scoring without a failing case.

### The alleged missing blank line before schema stubs is unsupported

The supplied diff does not include the actual missing-section append block, so the allegation cannot be verified directly from this excerpt. More importantly, no changed line removes separator handling, and the renderer ends supplied content with a newline.

There is no grounded basis here for reporting malformed stub spacing as an existing defect. The right response is a regression test for the missing-section path, not an assertion that the output is already wrong.

## Non-blocking problems and follow-up debt

### 3. Required-section behavior is not adequately pinned

The new test at `crates/issuectl-core/src/mutate/new_issue.rs:696-719` verifies that sections after a horizontal rule are detected. It does **not** exercise the case where a section is actually missing and must be appended.

Add a core test covering all three conditions:

1. Existing sections are not duplicated.
2. A missing section is appended.
3. The appended section has canonical blank-line separation.

Example assertions:

```rust
assert_eq!(content.matches("## Description").count(), 1);
assert_eq!(content.matches("## Expected").count(), 1);
assert_eq!(content.matches("## Quick Test").count(), 1);
assert!(content.contains(
    "## Expected\n\nExpected behavior.\n\n## Quick Test\n\n"
));
```

Use a schema requiring `Description`, `Expected`, and `Quick Test`, while the supplied body contains only the first two.

This is the most important missing behavioral test because the docs explicitly promise schema stubs.

### 4. The boolean-plus-optional-value representation is fragile

These fields:

```rust
description: Option<String>,
structured_body: bool,
```

permit invalid or meaningless states:

```rust
description: None,
structured_body: true,
```

They also force every constructor and test literal to know the mode invariant. The apparent missing `cli_tests.rs` update demonstrates the maintenance problem immediately.

A better model is:

```rust
pub enum InitialBody {
    None,
    FreeText(String),
    StructuredMarkdown(String),
}
```

or a borrowed equivalent in `NewIssueArgs`.

This should be follow-up work, not a blocker for the current bug fix, unless maintainers want to avoid introducing the boolean at all. A full enum refactor would broaden the patch substantially.

### 5. Agent documentation contains a schema-related contradiction

At the modified section of `crates/issuectl-core/templates/issue-skill.md`, the text correctly says:

> A repository schema may still append stubs for any required H2 sections the content omits.

Later it says:

> For epics, add `## Goal`, `## Issues`, `## Phases`, and `## Notes` sections — the CLI does not write these.

That categorical statement is false if the repository schema requires those sections. Rewrite it as:

```markdown
For epics, ensure `## Goal`, `## Issues`, `## Phases`, and `## Notes`
exist. The default renderer does not add them, although repository schema
requirements may append missing stubs.
```

This is minor compared with the outright synchronization failure.

### 6. “Complete structured Markdown body” should exclude frontmatter and H1 explicitly

The CLI still generates issue frontmatter and the title H1. A body file containing its own H1 or YAML frontmatter is not rejected; it becomes body content beneath the generated H1.

The current wording mostly implies this, but agents routinely generate full Markdown documents. Add a direct sentence to the help/skill:

```markdown
Supply body content only; do not include issue YAML frontmatter or the
generated H1 title.
```

This is documentation hardening, not a production defect.

### 7. Import documentation and tests overstate round-trip behavior

`crates/issuectl-core/src/transfer.rs` says import is content-level rather than byte-faithful, which is good. However:

```rust
fn export_json_round_trips_through_import()
```

only verifies export → parse. It never calls `into_new_args` and renders the resulting issue. The test name obscures the existing duplicate-heading behavior.

Rename it to something accurate, such as:

```rust
export_json_parses_as_import_records
```

Then track actual export/import body semantics separately.

## Missing tests

These are worthwhile but not all are blockers:

1. **Required section actually missing** — highest value.
2. **Empty/whitespace-only `--body-file` rejection** — pins existing CLI validation.
3. **Structured body beginning with a horizontal rule** — verifies it remains body content.
4. **Structured body containing an H1** — either pin acceptance or introduce validation; do not leave behavior accidental.
5. **Required-looking headings inside fences** — verify they do not satisfy schema requirements, consistent with `all_h2_sections`.
6. **Preserved free-text mode for import or intake** — one focused test would document the intentional divergence.

The existing process tests are justified because they exercise clap, file/stdin I/O, runtime mode selection, and the built binary. The first structured-file test and the older modified body-file test are partially redundant, but that is not important enough to change.

## Risks

### Constructor drift

Any future `NewArgs` constructor can choose the wrong mode or fail to compile. Defaults bias new call sites toward free text, which could recreate the same wrapper bug for another structured input source.

### Agent-contract drift

The current diff already demonstrates this risk: one shipped template changed while its Codex and dogfooded counterparts did not. If synchronization tests are not run, different agents receive contradictory instructions.

### Schema behavior can regress without detection

The horizontal-rule test covers “do not append duplicates,” but not “append the missing stubs correctly.” A future change could stop appending missing sections entirely and the new test would remain green.

### Terminology encourages misuse

Calling structured content `description` throughout the mutation and writer layers makes it easy for future code to assume prose semantics and wrap, trim, or validate it incorrectly.

## Changed views

- **Withdrawn:** empty body-file ambiguity. The CLI rejects it.
- **Downgraded to pre-existing debt:** Markdown-significant trailing whitespace removal.
- **Rejected:** plain-prose wrapper removal as a defect. The declared contract and tests explicitly require it.
- **Rejected:** intake/import free-text mode as a regression. Preservation is intentional.
- **Accepted only as follow-up debt:** own JSON export/import can duplicate Description.
- **Rejected as unproven:** missing blank line before generated schema stubs.
- **New blocker:** the current diff has lost required template/dogfood synchronization.
- **New blocker:** the current diff appears to have lost the required `cli_tests.rs` constructor update.

## Top 3 priorities

1. **Restore all template and dogfooded-copy updates.** This is a repository-contract violation and likely test failure.
2. **Restore/update every `NewArgs` struct literal, especially `crates/issuectl/src/cmd/cli_tests.rs:133`, then run the workspace tests.**
3. **Add a required-section regression where one section is genuinely missing and assert canonical spacing around the appended stub.**

The enum refactor, import round-trip semantics, and stricter body-document guidance are follow-up debt. They should not be used to expand or block the create-only production fix once the two concrete repository failures above are corrected.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_6c3629d4057240d29550996b5ea5ba97]

# Adjudication of the three reviews

## 1. Agreements (supported by evidence)

- **Bool + Option invalid-state model (all three).** Confirmed and it's my standing P3. `(description: None, structured_body: true)` is constructible via public `NewArgs`, and six call sites carry `structured_body: false` boilerplate. An `InitialBody { None, FreeText(String), Structured(String) }` enum eliminates the class. This is the one point of full consensus and it's correct.
- **DeepSeek: divergent `--body-file` semantics between `create` and `intake file`.** Confirmed: `intake.rs:321` wraps, `create` doesn't, same flag name. The task defines intake preservation as intentional, so this is a **documentation** defect, not a code defect — the `issue-new` skill says "Prefer `--body-file <path>`" with no hint the semantics differ from the `create --body-file` behavior the same release's `/issue` skill advertises.
- **OpenAI/DeepSeek: missing required-section-omitted test.** Confirmed (my P6). The templates promise "A repository schema may still append stubs for any required H2 sections the content omits"; `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` covers only the all-present case. The stub-append leg of the structured path is untested.
- **OpenAI: no H1 guidance.** Valid, minor. A "complete structured body" invites full documents; a file opening with its own `# Heading` produces two H1s under the generated title. Neither the flag help nor the skill addresses it. One sentence in each fixes it.
- **DeepSeek: plain-prose compatibility.** Partially — see §2 for the recalibration.

## 2. Disagreements (wrong, exaggerated, or out of scope)

- **Gemini: "missing blank line before appended stubs" — rejected.** No evidence supports it. The codebase's section-append convention (`body_sections::append_new_section`) trims trailing newlines and emits `\n\n## <name>\n\n`, and the fmt-idempotency tests (`append_idempotent_under_fmt`, `merge_h2_output_round_trips_through_fmt`) pin that render output is fmt-canonical. Gemini asserted a spacing bug without pointing at a line that produces it. If the stub path bypassed the shared helper, `fmt` would flag every schema-stubbed issue — nothing suggests that.
- **Gemini: "render→split→scan→append is fragile" — out of scope and backwards.** That pipeline pre-dates the diff; the diff's `item_text::split` change made it *less* fragile (fixed the `---\n\n` truncation). Criticizing the architecture of code the diff improved, without a concrete defect, is noise.
- **OpenAI: trailing-space loss — exaggerated to the point of wrong.** `read_body_file_arg` calls `String::trim_end()` on the whole document, not per line. Interior-line trailing spaces (Markdown hard breaks) survive intact; only the final line's trailing whitespace is stripped, and a hard break on the last line of a document is semantically inert. It also pre-dates the diff and deliberately mirrors `body set --from-file` (documented in the function's own comment).
- **OpenAI: "contradictory epic-stub wording" — overstated.** "A repository schema *may* append stubs" (conditional on schema config) and "the CLI does not write these [epic sections]" (default, no schema) are not contradictory. There *is* a real wrinkle nearby, but it's a different one — see §3.
- **DeepSeek: duplicate-precheck mismatch — not a regression from this diff.** `duplicate_precheck` (`crates/issuectl/src/cmd/write.rs`) builds the candidate with `body: args.description`, unwrapped. That was equally true pre-diff for both inline and file bodies (the file content always landed in `description`); the only delta versus stored bodies is the literal `## Description` heading token, which existed before too. Nothing in this diff changed what the precheck sees.
- **DeepSeek: own-export import duplication — real defect, but pre-existing and explicitly out of scope; my own prior framing was wrong.** I previously said this "should have been part of this fix." Correcting that: pre-diff, *every* creation path wrapped, so `export --format json | import` produced the doubled `## Description` **before** this change as well (`ImportRecord.description` gets the full body via `#[serde(alias = "body")]`; the round-trip test shows `"## Description\n\nSomething broke."` landing there). The diff neither introduced nor worsened it, and the task explicitly preserves import semantics. It remains a live defect worth a follow-up issue — the module doc's "content-level, not byte-faithful" disclaimer covers dropped *fields*, not body corruption — but it is not a deficiency of this diff. **I also retract my earlier claim that the CHANGELOG entry misleads about import**: the entry is explicitly scoped to `create --body-file`.
- **Recalibrating my own P1 in light of the stated task.** The plain-prose behavior change is a *deliberate contract redefinition*, not a bug. What survives of P1: (a) the CHANGELOG files a semantic redefinition under `### Fixed` — existing `--body-file` users of plain prose get silently different output, which is `### Changed` material; (b) no warning when a "structured" body contains zero H2 headings (my P2), which is exactly the degraded case; (c) the stub-lands-after-prose consequence is documented nowhere. The contract itself is not the defect; the migration handling is.

## 3. New findings

- **Epic guidance now steers `--body-file` users into a warning.** Template step 3 tells agents to add `## Notes` to epics; the `--body-file` help (`cmd/mod.rs:626`) says `## Notes` is a reserved legacy heading that "is accepted but warns." Pre-diff, the epic guidance was about post-create hand-editing; now that `--body-file` is promoted as the one-shot "complete structured body" path, an agent composing an epic body file per the template's own section list triggers the reserved-heading warning by following the same document's instructions. The epic list should say `## Comments` or the help should carve out epics — one of the two files is wrong.
- **`duplicate_precheck` on structured bodies scores the raw description including heading lines.** Not a regression (per §2), but the new contract makes heading-heavy candidate bodies the norm for `--check-duplicates --body-file`: every candidate now shares `## Description` / `## Expected` tokens with every existing issue, inflating `body_overlap` uniformly. Whether `find_duplicates` tokenizes headings is not in evidence; worth a look when the enum refactor touches this call site anyway.
- **Confirmed clean, worth stating:** `read_body_file_arg` rejecting empty input means the CLI can never emit the `(None, structured=true)` state; and the stdin test path exercises the TTY-guarded `read_capped_stdin` route correctly. The `run_with_stdin` helper still doesn't drain stdout while writing stdin — safe at current payload sizes, deadlocks past the pipe buffer; add a comment.

## 4. Top 3 priorities

1. **Replace `description: Option<String>` + `structured_body: bool` with an `InitialBody` enum** (`new_issue.rs`, `write.rs`, all six call sites). Unrepresentable invalid state, self-documenting constructors, and it's the consensus finding across all four reviews.
2. **Reclassify the CHANGELOG entry under `### Changed` and add a core-side warning when a structured body contains no H2 sections** (the existing `warnings` channel). Together these handle the deliberate-but-silent migration for plain-prose `--body-file` callers — the only part of the contract change that is genuinely user-hostile as shipped.
3. **Add the missing structured-body + omitted-required-section test** (schema `[Description, Expected]`, file with only `## Description`, assert one appended `## Expected` stub with correct separation — which also empirically settles Gemini's spacing claim), **and** add one sentence to the `issue-new` skill / `intake file` help stating intake bodies are wrapped under a generated `## Description`, closing the cross-command flag-semantics divergence. File the pre-existing export→import body duplication as a separate follow-up issue.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_16db540a340743ca90ae1a10f88688e1]

## Reviewer cross-check

### Gemini

- “render → split → scan → append is fragile” — partially valid as a maintainability complaint. The change still rounds-trips the fully serialized item through a string split, then re-parses headings. It is not a correctness bug, and the replacement splitter is strictly better than the old `split("---\n\n")`.
- “missing blank line before appended stubs” — **incorrect**. `body_sections::append_new_section` and `append_block` already insert exactly one blank separator (`if !trimmed.is_empty() { out.push_str("\n\n"); }`). The new splitter does not regress spacing.
- “bool + Option is invalid-state prone” — valid. `description: None, structured_body: true` is representable. No current call site creates it, but the public `NewArgs` shape invites it.

### OpenAI

- “trailing-space loss” — **pre-existing and not meaningful here**. `render_new_item_from_fm` already called `d.trim_end()` before this diff. That strips only trailing document whitespace, not per-line hard-break spaces inside the body. Not a regression.
- “contradictory epic-stub wording” — appears to be pre-existing template prose, not introduced by this diff. Not a body-file regression.
- “bool model and naming” — same as Gemini; valid design criticism.
- “edge/integration tests” — valid. The most important missing case is a schema-required section combined with a body file that contains no H2 sections.
- “H1 guidance” — valid. A body file containing its own `# Title` now produces a second H1 directly under the generated one. Nothing documents or rejects that.
- “exact-shape heading scanning” — valid underlying limitation. `body_sections::is_any_h2` requires a literal line start of `## `, so CommonMark’s up-to-3-space ATX indentation is not recognized. That is pre-existing, but “structured Markdown” makes it more reachable.
- “canonical splitting and runtime wiring solid” — agree.

### Anthropic

- Withdrawing empty-file and intake-as-bug concerns — fine.
- Divergent `--body-file` semantics across `create` and `intake file` — valid. The same flag now means structured body on `create` and verbatim free text on `intake file`. The templates document each individually but nowhere warn that the same flag name has opposite rendering policies.
- Own-export import duplication — confirmed, but this is **pre-existing and outside this diff’s stated scope**. The task explicitly preserves import semantics. The `export_json_round_trips_through_import` test name has been misleading for a while; this diff does not make it worse.
- Plain-prose behavior as a broad change — agree, and it is the core problem.
- Requests for no-H2 warning, enum, and missing-section test — all reasonable. The no-H2 warning is the main mitigation for the top bug below.

## New findings

### 1. Unstructured `--body-file` with schema-required sections orphans the actual content

`crates/issuectl-core/src/mutate/new_issue.rs` now extracts the body with `item_text::split`, then calls `all_h2_sections` and appends stubs for missing required sections. The render path in `crates/issuectl-core/src/write.rs` no longer wraps file content in `## Description`.

Consider a repo with:

```yaml
body_sections:
  bug: [Description]
```

and:

```sh
issuectl create --type bug --title "Login loop" --body-file notes.md
```

where `notes.md` is:

```text
The login page redirects back to itself.
```

The resulting item is:

```markdown
# Login loop

The login page redirects back to itself.

## Description

```

The actual report text is outside the required `## Description` section, and the appended stub is empty. That is not a reasonable interpretation of “schema may append stubs for missing sections.” The content is detached from the required section and the schema requirement is satisfied vacuously.

The templates say the schema “may still append stubs for any required H2 sections the content omits,” but they do not state that the omitted content will not be inside those sections. For a free-text `--body-file`, this is a silent data-layout regression.

Mitigation: when `structured_body` is true and a required section is missing, especially when the body contains no H2 sections at all, emit a creation warning telling the author to add the required heading, or reject. A silent empty `## Description` plus orphaned prose is the worst outcome.

### 2. CHANGELOG classifies a breaking behavioral change as `Fixed`

`CHANGELOG.md` puts the new behavior under `### Fixed`. For existing `--body-file` users supplying free text, the generated item body changes shape permanently. Repos with schema-required `Description` get the bug above. This is a breaking change for a CLI contract, not a bug fix. It belongs under `### Changed` with an explicit migration note.

### 3. `duplicate_precheck` still scores the raw description, not the rendered body

`crates/issuectl/src/cmd/write.rs` builds the duplicate candidate with:

```rust
body: args.description.clone().unwrap_or_default(),
```

`--body-file` now stores `# Title` + optional source preamble + raw file content, while the duplicate check scores only the raw file content. Inline `--description` stores the generated `## Description` wrapper too. The mismatch is not introduced by this diff, but adding a second rendering mode widens it and the new `structured_body` flag could have been used to construct a faithful candidate. Body-overlap duplicate scores for structured files will systematically undershoot.

### 4. `--body-file` is not validated for an unclosed fence

`read_body_file_arg` rejects empty/whitespace input but does not run `validate_message` or any fence check. `create --body-file` can therefore accept a document ending in an unclosed code fence. When `do_new_locked` appends required-section stubs, they land inside the still-open fence semantically and are swallowed. The note path already rejects unclosed fences; the create body path should at least consider the same guard for structured files.

### 5. Integration test name no longer describes its behavior

`new_body_file_writes_markdown_below_heading` in `crates/issuectl/tests/cli_new.rs` now asserts that no `## Description` wrapper exists. The name still describes the old contract. Rename to something like `new_body_file_places_markdown_directly_below_heading`.

## Top 3 priorities

1. **Warn or reject unstructured `--body-file` content when the repo schema requires H2 sections.** This is the only path where actual report content is silently misplaced outside the required section. Add a core regression test for a body file with no H2 sections plus a schema requiring `Description`.

2. **Reclassify the CHANGELOG entry as `Changed`, not `Fixed`, and document the migration consequence.** The current entry hides a contract change from scripted callers.

3. **Replace `bool structured_body` + `Option<String> description` with a three-state enum** (`None`, `FreeText(String)`, `Structured(String)`) at least in `NewArgs`/`NewIssueArgs`. That removes the invalid `structured_body: true, description: None` state, forces every future creation path to choose semantics explicitly, and makes the divergent `create` vs `intake file` `--body-file` behavior explicit in the type.

## Authoritative preserved review — cross-review round 2 (all four complete)

[thread_id:group_64f1d0d7e0ee4d32b4a223e5bccd1408]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_ee0b71452aaa4d3d893db7cdeb51f306]

### 1. Corrections and Valid Findings

**Self-Correction: `item_text::split` is not a no-op.**
I explicitly retract my previous claim that `item_text::split(&render)` is dead code that works by accident. The verified facts confirm `render_new_item_from_fm` *does* serialize and prepend the YAML frontmatter before the split occurs. Therefore, `item_text::split` correctly locates the frontmatter boundary, and using it to fix the horizontal rule (`---\n\n`) bug is functionally sound. 

**Valid finding from others: Primitive Obsession**
The consensus that `structured_body: bool` paired with `description: Option<String>` is a weak, leaky model is entirely correct. It forces all non-CLI callers (Intake, Recurrence, Transfer) to initialize a dummy boolean state. An `enum IssueBody { FreeText(String), Structured(String) }` is the objectively correct domain model. 

### 2. Disagreements and Dropped Claims

**Drop: `import` / `export` duplication bug**
Anthropic and DeepSeek accurately identified that `issuectl export` outputs a `body` with the `## Description` wrapper, and `issuectl import` reads this back as free text, resulting in a double-header. However, the exact task constraints explicitly state: *"Intake, core API, recurrence, and import semantics are intentionally preserved as free-text behavior."* Expanding this PR to fix the legacy import round-trip duplication is out of scope. 

**Drop: Plain-prose file behavior is a "regression"**
Anthropic and DeepSeek argued that passing a plain text file via `--body-file` will now leave the text floating without a `## Description` heading, calling this a breaking change. This is false. The documented contract change explicitly guarantees that `--body-file` inserts the content *exactly as provided*. Bypassing the wrapper is the intended feature. If a user wants the wrapper, they use inline `--description` or include the heading in their file.

**Drop: Unclosed fences, duplicate pre-check fidelity, and exact heading grammar**
These are unjustified hardening requests that represent scope creep for a targeted bug fix.

### 3. New Findings Grounded in the Final Diff

**Missing Test for Positive Stub Appending on Structured Bodies**
*`crates/issuectl-core/src/mutate/new_issue.rs:693-714`*
The new test `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` proves the *negative* case: if the user supplies a required section (`## Expected`), the CLI does not duplicate it. 
However, there is no test for the *positive* case: proving that if a structured body *omits* a required schema section, the `do_new_locked` stub-appending logic correctly concatenates the stub to the end of the user's structured markdown without corrupting the layout or swallowing trailing text. Given the fragility of string-concatenating Markdown, this execution path must be covered.

### 4. TOP 3 Final Priorities

**1. Missing coverage for appended stubs on omitted required sections (Warranted in-patch improvement)**
Add a test in `cli_new.rs` or `new_issue.rs` where `--body-file` supplies structured Markdown that intentionally omits a schema-required H2 section. Assert that the required stub is successfully appended to the end of the document, properly separated by newlines.

**2. Primitive Obsession in `NewArgs` (Follow-up / Drop)**
While `structured_body: bool` is a design smell, refactoring `NewArgs` to use a typed `enum` requires rippling changes through the CLI parser, Intake, Recurrence, and Transfer modules. Because the boolean mechanically works and defaults to `false` for legacy paths, this can be deferred as design debt rather than blocking this specific fix.

**3. JSON Import Duplication (Drop)**
The duplication of the `## Description` header during a JSON export-to-import roundtrip is a real, pre-existing bug, but fixing it violates the strict scope defined for this task. Leave it for a separate patch.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d3af4b40a7e14b92b448624d7e755476]

# Final assessment

No production blocker is demonstrated in the final diff. The create-only fix implements the declared contract correctly.

## Corrections to my earlier review

I made two incorrect blocker claims based on an incomplete attachment set:

- **Withdrawn:** template/dogfooded-copy synchronization failure. The final diff updates both shipped templates and both dogfooded copies.
- **Withdrawn:** missing `structured_body` initialization in `cli_tests.rs`. The final diff includes it.

Additional corrected positions:

- **Withdrawn:** empty-body ambiguity. `read_body_file_arg` rejects empty and whitespace-only files.
- **Rejected:** alleged missing blank line before schema stubs. The append logic explicitly ensures `\n\n`.
- **Downgraded:** trailing-whitespace loss is existing, documented normalization, not introduced by this patch.
- **Rejected:** treating plain-prose body files as a regression. Wrapper-free prose is explicitly part of the new contract and is pinned by a black-box test.

## Findings that remain valid

### Warranted in-patch improvement — missing targeted regression for an actually absent schema section

`crates/issuectl-core/src/mutate/new_issue.rs:696-719` tests a structured body containing all required sections across a horizontal rule. It proves the canonical splitter prevents false “missing” results.

It does not test the other half of the documented behavior: a genuinely missing required section is appended correctly to a structured body without reintroducing a wrapper or corrupting spacing.

Add a test using:

```yaml
body_sections:
  bug: [Description, Expected, Quick Test]
```

with a structured body containing `Description` and `Expected`. Assert:

- exactly one `Description`;
- exactly one `Expected`;
- exactly one appended `Quick Test`;
- the horizontal rule and supplied content remain intact;
- canonical `\n\n` separation precedes the appended stub.

This is not evidence of a current bug. It is the main uncovered branch introduced into the documented contract.

### Warranted in-patch improvement — agent documentation contains an overbroad statement

All four synchronized agent-facing files say schema-required H2 stubs may be appended, then later state:

> For epics, add `## Goal`, `## Issues`, `## Phases`, and `## Notes` sections — the CLI does not write these.

That is categorically false when the repository schema requires those sections. Qualify it:

```markdown
For epics, ensure `## Goal`, `## Issues`, `## Phases`, and `## Notes`
exist. The default renderer does not add them, although repository schema
requirements may append missing stubs.
```

Affected locations:

- `crates/issuectl-core/templates/issue-skill.md:596+`
- `crates/issuectl-core/templates/issue-prompt.md:592+`
- `.claude/skills/issue/SKILL.md:596+`
- `.codex/prompts/issue.md:592+`

Keep all copies synchronized.

### Follow-up debt — `Option<description> + bool` is a weak state model

`crates/issuectl-core/src/mutate/new_issue.rs:65-72` and `crates/issuectl-core/src/write.rs:344-350` represent body semantics with:

```rust
description: Option<String>,
structured_body: bool,
```

This admits `description: None, structured_body: true` and requires every constructor to maintain a cross-field invariant. The final diff updates all shown constructors correctly, so there is no current defect.

An enum would be safer:

```rust
enum InitialBody {
    None,
    FreeText(String),
    StructuredMarkdown(String),
}
```

This is follow-up refactoring, not a reason to expand or block this patch. Converting all intake, recurrence, import, core API, CLI, and writer call sites now would add risk disproportionate to this focused fix.

## Findings that must be dropped

### Own-export/import duplicate heading

This is real existing behavior:

- exported `body` aliases to `ImportRecord.description`;
- import intentionally sets `structured_body: false`;
- a body already containing `## Description` can therefore receive another wrapper.

It is explicitly outside this task, which requires import semantics to remain free text. Do not change import mode in this patch. Track it separately if product semantics should change.

The test name `export_json_round_trips_through_import` is overstated because it only tests export and parsing, but renaming that test is unrelated cleanup.

### Intake, recurrence, and core API divergence

The following values are correct under the task:

- `mutate/intake.rs`: `structured_body: false`
- `mutate/new_api.rs`: `structured_body: false`
- `recurrence.rs`: `structured_body: false`
- `transfer.rs`: `structured_body: false`

Calling this inconsistency a bug directly contradicts the required compatibility behavior. Additional intake documentation is optional and should not expand this patch unless the intake help currently makes a conflicting promise.

### Plain-prose compatibility warning or no-H2 warning

The contract says a body file is complete structured Markdown placed directly after the generated preamble. Markdown does not require H2 headings. The updated black-box test explicitly verifies wrapper-free plain prose.

A warning for bodies without H2 headings would penalize valid input and undermine the declared contract. Drop it.

### Duplicate-precheck mismatch

`duplicate_precheck` uses the supplied body content:

```rust
body: args.description.clone().unwrap_or_default(),
```

That remains suitable for semantic duplicate scoring. Generated title/source/Description boilerplate should not affect content similarity. No failing case demonstrates a regression, so this finding is speculative.

### Unclosed-fence validation

A complete Markdown body can be malformed, but creation historically accepts Markdown content and schema scanning is deliberately fence-aware. This patch does not establish a new validation contract requiring balanced fences.

Rejecting such bodies would be a separate CLI behavior change with compatibility implications. It is unjustified hardening here.

### Exact H2 grammar

`all_h2_sections` recognizes the repository’s canonical `## Name` convention. Supporting setext headings, closing `#` markers, or noncanonical spacing would be a broader body-section parser change. Nothing in this patch requires accepting every CommonMark-equivalent heading form.

### Frontmatter/H1 rejection

The CLI already generates issue frontmatter and an H1, so documenting “body content only” could help agents. However, rejecting supplied YAML delimiters or H1 headings would introduce new validation semantics. Do not add validation in this patch without a separate product decision.

A short documentation clarification is optional, not required.

### Trailing Markdown spaces

`read_body_file_arg` and the shared renderer already normalize trailing whitespace with `trim_end()`. The behavior is explicitly documented and shared with body replacement. Whether Markdown hard-break spaces should be preserved is a legitimate separate design issue, not a regression caused by wrapper suppression.

## What is correct in the final diff

- `crates/issuectl/src/cmd/runtime.rs:589` records whether `body_file` was supplied before consuming the option.
- Inline `--description` remains wrapped.
- File and stdin bodies become structured content.
- Source preamble ordering is tested exactly.
- Plain-prose structured input is explicitly tested.
- The canonical frontmatter splitter is correctly applied to a fully serialized item; it is not a no-op.
- Horizontal rules in the body no longer truncate required-section detection.
- Empty input remains rejected at the CLI boundary.
- Schema stub separation remains canonical.
- Historical non-create paths explicitly retain free-text behavior.
- Mutation still routes through core locking and schema validation.
- Shipped templates and dogfooded copies are synchronized.
- Integration-test placement is justified because the tests exercise built-binary argument parsing, file/stdin I/O, and process behavior.

## Top 3 final priorities

1. **Add the structured-body test with one genuinely missing required schema section.**  
   Classification: **warranted in-patch improvement**, not a blocker.

2. **Qualify the agent documentation’s “CLI does not write these” statement to account for schema-generated stubs.**  
   Classification: **warranted in-patch improvement**, not a blocker.

3. **Replace `Option<String> + bool` with an explicit body-mode enum in separate work.**  
   Classification: **follow-up debt**, not an immediate fix.

There is no grounded blocker in the attached final diff. The patch is functionally correct for the declared create-only contract; the remaining in-patch work is test and documentation precision.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_6c3629d4057240d29550996b5ea5ba97]

# Final adjudication

## Corrections to my own record first

- **Round 1:** I filed the import export-body duplication as "should have been part of this fix" — wrong; it demonstrably pre-dates the diff (pre-fix, *every* path wrapped, so self-export re-import already doubled `## Description`). Retracted in round 3; reaffirmed here.
- **Rounds 1–3:** I demanded a no-H2 "structured body" warning as an in-patch requirement (P2). Downgrading: given the task explicitly defines file input as structured Markdown, the CLI help and all four skill copies state it, and empty files are rejected, a heading-less file is documented misuse. The warning is a reasonable *follow-up* UX hardening, not a patch gate.
- **Round 3:** I presented the epic `## Notes` / reserved-heading conflict as introduced by this diff. Partially wrong: the template sentence recommending `## Notes` for epics is pre-existing text the diff merely re-wrapped; what's new is only that `--body-file` makes it a create-time warning path instead of a post-edit one. Follow-up, not in-patch.

## 1. Findings that survive the verified facts

1. **Missing structured-body + omitted-required-section test.** The only agent-facing promise in the diff ("A repository schema may still append stubs for any required H2 sections the content omits", four copies) with zero coverage on the structured path. `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` tests only the all-present case. Cheap, unambiguous, tests exactly the new code path through `do_new_locked`'s stub branch.
2. **CHANGELOG misclassification.** The entry sits under `### Fixed` and describes only the duplicate-heading symptom. The verified fact that the contract change is *intentional* strengthens, not weakens, this finding: an intentional redefinition of `--body-file` semantics — plain-prose files silently lose their `## Description` wrapper, and a schema-required Description arrives as an empty stub after the prose — is `### Changed` material by the repo's own Keep-a-Changelog adherence. One entry move plus one sentence.
3. **Bool + Option representation** (`NewArgs.structured_body` + `description: Option<String>`, `write.rs` mirror). Valid across all four reviews; `(None, true)` remains constructible via the public core API with no renderer guard, and seven boilerplate sites invite the next mistake. But all seven sites are *correct in this diff*, tested, and `issuectl-core`'s `pub` surface is explicitly not a semver contract — so refactoring later costs nothing. Not a patch gate.
4. **Intake/create `--body-file` semantic divergence** — real, intentional per task, and undocumented in the `issue-new` skill. Documentation follow-up.
5. **Import self-export duplication** — real defect, pre-existing, explicitly out of scope. File as its own issue; do not touch in this patch.

## 2. Dropped

- **Gemini: missing blank line before stubs** — false; stub appending verified to ensure `\n\n`.
- **Gemini: render→split→scan→append fragility** — out of scope; the diff *improved* this pipeline (canonical splitter), and `render_new_item_from_fm` verifiably includes frontmatter, so `item_text::split(&render)` is doing real work, not a no-op.
- **OpenAI: template/dogfooded-copy drift** — false; verified the final diff updates both shipped templates and both dogfooded copies in lockstep, and `cli_tests.rs` is updated. The claim came from reviewing a truncated attachment set — an object lesson in AGENTS.md's own rule about verifying findings against the tree.
- **OpenAI: trailing-space loss** — pre-existing, whole-document `trim_end` only, semantically inert; drop.
- **OpenAI: epic-stub wording contradiction** — overstated; "schema may append" (conditional) vs "CLI does not write these" (default) coexist.
- **OpenAI: exact-heading-grammar / unclosed-fence scanning concerns** — pre-existing `all_h2_sections` semantics shared with every other body consumer (fmt, doctor, merge); changing the scanner's grammar inside this patch would be unjustified hardening with cross-module blast radius.
- **DeepSeek: duplicate-precheck mismatch** — pre-existing; the precheck's candidate body was identical pre/post diff modulo one literal heading token that existed before too. Not a regression.
- **DeepSeek: plain-prose "compatibility bug"** — as a *code* finding, dropped; the contract is the declared task. Survives only as the CHANGELOG/migration-visibility item above.
- **H1-exclusion validation, no-H2 warning, test rename** — documentation-note-at-most, follow-up, and nit respectively. None gate the patch.

## 3. New blockers in the final diff

None. Re-checked the final diff against the verified facts: all six constructor sites carry the correct flag value, both templates and both dogfooded copies match (satisfying the `dogfooded_copies_match_templates` invariant), the runtime wiring sets `structured_body` from `body_file.is_some()` before the move, and the three integration tests plus two inline tests pin the CLI-observable shapes. The splitter change is a strict correctness improvement with its own regression test. The patch is shippable.

## 4. Top 3 final priorities

1. **Add the structured-body + omitted-required-section regression test** (inline, `new_issue.rs` tests: schema `[Description, Expected]`, description containing only `## Description`, assert exactly one appended `## Expected` stub) — **warranted in-patch improvement**. It is the sole untested promise the diff makes to consumer agents.
2. **Move the CHANGELOG entry to `### Changed` and state the plain-prose consequence** (prose body files no longer get a `## Description` wrapper; required sections arrive as stubs after the content) — **warranted in-patch improvement**. Two-line edit; the current `### Fixed` framing hides an intentional semantic redefinition from every downstream consumer of the changelog.
3. **`InitialBody` enum refactor + follow-up issues** for (a) the pre-existing export→import body duplication and (b) the intake-skill `--body-file` semantics note / epic `## Notes` template wording — **follow-up/drop from this patch**. The enum prevents the next constructor mistake but nothing is wrong today; the other two are pre-existing and explicitly out of scope.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_16db540a340743ca90ae1a10f88688e1]

## Final reassessment

### Corrections to my earlier independent review

I over-weighted two items that are real but not in-patch defects, and under-weighted one interaction bug that is.

1. **Import own-export JSON duplication** — real, but explicitly out of scope. `transfer::ImportRecord::into_new_args` hardcodes `structured_body: false` and the task intentionally preserves import semantics. Drop from blocker consideration entirely; at most a follow-up ticket.
2. **`duplicate_precheck` fidelity** — genuine divergence, but the affected code is untouched by this diff and has been wrong for both modes. It is a follow-up, not an in-patch requirement.
3. **Orphaned prose under schema-required sections** — I initially framed this as a CHANGELOG/breaking-contract issue. That is the weaker half. The stronger, in-patch functional defect is: *the documented stub-append behavior produces a corrupt body layout for a body file that omits the required H2*.

### Claims from other reviewers that must be dropped or marked false

- **Gemini’s “missing blank line before appended stubs”** — false. `body_sections::append_new_section` and `insert_block_in_section` already emit exactly one blank separator (`if !trimmed.is_empty() { out.push_str("\n\n"); }`). The final diff does not regress spacing.
- **OpenAI’s “trailing-space loss”** — pre-existing and not a regression. `render_new_item_from_fm` already called `d.trim_end()`; it strips only trailing document whitespace, not line-level hard-break spaces. Drop.
- **OpenAI’s “contradictory epic-stub wording”** — pre-existing template prose outside this diff. Drop.
- **Anthropic’s empty-file and intake-as-bug concerns** — already withdrawn; nothing to contest.
- **Anthropic’s own-export import duplication** — confirmed, but pre-existing and explicitly out of scope per the task.

### Remaining valid findings

#### 1. Structured body missing a schema-required H2 orphans its prose

`crates/issuectl-core/src/mutate/new_issue.rs`:

```rust
let body_only = crate::item_text::split(&render).body;
let present = crate::body_sections::all_h2_sections(body_only);
// append stubs for missing required sections
```

`crates/issuectl-core/src/write.rs`:

```rust
if !args.structured_body {
    body.push_str("## Description\n\n");
}
```

Schema:

```yaml
body_sections:
  bug: [Description]
```

Body file:

```text
The login page redirects back to itself.
```

Result:

```markdown
# Login loop

The login page redirects back to itself.

## Description

```

The report text is not inside the required `## Description`; the appended stub is empty. The schema requirement is satisfied vacuously and the content is detached. The templates claim schemas “may still append stubs for any required H2 sections the content omits,” which is technically what happens, but the result is data-layout corruption, not compatibility.

This is the top issue. It needs either:

- a creation warning naming the missing required sections and the consequence, or
- rejection when `structured_body` is true and a required section is absent, or
- wrapping the orphaned prose under the first missing required section (the most useful, but more invasive).

At minimum, a regression test for a body file containing no H2 plus a schema requiring `Description` is mandatory.

#### 2. Breaking CLI contract filed under `### Fixed`

`CHANGELOG.md`:

```markdown
### Fixed
- `create --body-file` now places complete structured Markdown beneath the
  generated title/source preamble without injecting an empty duplicate
  `## Description` heading.
```

For any existing caller passing free text to `--body-file`, on-disk output changes shape after upgrade:

Before:

```markdown
# Title

## Description

First paragraph.

Second paragraph.
```

After:

```markdown
# Title

First paragraph.

Second paragraph.
```

This is a contract change, not a bug fix. It belongs under `### Changed`. The templates and CLI help now describe the new contract, so the documentation half is done, but the changelog’s release-notes classification and the absence of migration guidance are wrong.

#### 3. `duplicate_precheck` still scores raw description, not the rendered body

`crates/issuectl/src/cmd/write.rs`:

```rust
body: args.description.clone().unwrap_or_default(),
```

With two render modes, this matches neither stored shape. For `--body-file`, stored bodies include the generated H1 and preamble; for `--description`, they include `## Description`. Body-overlap duplicate scores will systematically undershoot for structured files.

Pre-existing, but the new mode widens the gap. Follow-up.

#### 4. No guard against a body file containing its own H1

`read_body_file_arg` trims trailing whitespace and rejects empty input, but does not inspect structure. A file starting with `# Something` produces:

```markdown
# Generated title

# Something

...
```

Two H1s. The new “structured Markdown body” contract should define whether an H1 is allowed; if not, it should warn or reject.

#### 5. No unclosed-fence validation on the create body path

`read_body_file_arg` does not call `validate_message` or any fence check. A structured body ending in an unclosed code fence can cause later appended schema stubs to be semantically swallowed by that fence for every fence-aware reader. The note path already rejects unclosed fences; the create body path should at least consider the same guard.

#### 6. `bool structured_body` + `Option<String> description` is an invalid-state magnet

`NewArgs` and `NewIssueArgs` can represent `structured_body: true, description: None`. No current call site creates it, but the domain shape allows it. A three-state enum (`None`, `FreeText(String)`, `Structured(String)`) removes the invalid state and forces every future creation path to choose semantics explicitly.

#### 7. Divergent `--body-file` semantics across `create` and `intake file`

`create --body-file` now means structured Markdown directly under the title/source. `intake file --body-file` still means free verbatim text wrapped in generated `## Description`. The task explicitly preserves intake semantics, so this is not a bug, but the shared flag name with opposite rendering policies is a real agent/usability trap. The `issue-new` skill documents intake’s behavior, and the `issue` skill documents create’s, but neither warns that the same flag name means different things in the two command families.

## TOP 3 final priorities

| # | Finding | Classification |
|---|---------|---------------|
| 1 | Structured body omitting a schema-required H2 orphans prose under an empty stub | **Blocker** — in-patch fix plus regression test |
| 2 | Breaking `--body-file` contract change filed as `Fixed` in CHANGELOG | **Blocker** — move to `Changed`, document migration, consider no-H2 warning |
| 3 | Add the missing no-H2 structured-body regression test; consider enum over bool | **In-patch improvement** — the test belongs in this patch; the enum can be follow-up |

The splitter change (`crate::item_text::split`) is correct and well-tested for horizontal rules. All constructors are updated consistently. Template synchronization is done. The remaining work is targeted: protect the schema-stub interaction and reclassify the release note.

## Fresh adopted-tree supplement — independent responses (all four complete)

[thread_id:group_27056d376f5a418083e938079fd2b864]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_aa2d00d86f0844c9889bb7bf05625a56]

### 1. API Parity and Transfer Bug (Architecture & Correctness)

**Problem:** 
The `structured_body` field cleanly resolves the wrapper duplication for the CLI, but is aggressively hardcoded to `false` in non-CLI creation paths (`crates/issuectl-core/src/transfer.rs` and `crates/issuectl-core/src/mutate/new_api.rs`). This isolates the capability to the CLI and introduces architectural inconsistencies:

* **Transfer Bug Risk (`transfer.rs`):** When transferring an issue, `ImportRecord.description` almost certainly carries the full Markdown body of the source issue (which is already structured and contains headings). Forcing `structured_body: false` here means `issuectl transfer` will suffer from the exact bug this PR claims to fix: it will wrap the entire imported structured body inside a newly generated `## Description` heading.
* **API Limitation (`new_api.rs`):** Programmatic API consumers passing payloads via `NewIssueRequest` have no way to create an issue with a structured body. They are permanently locked into the free-text `## Description` wrapper. 

**Solution:**
Propagate the capability to the API layer and fix the transfer pipeline.

In `crates/issuectl-core/src/mutate/new_api.rs` (and corresponding external schema):
```rust
pub struct NewIssueRequest {
    // ...
    pub description: Option<String>,
    #[serde(default)]
    pub structured_body: bool, 
    // ...
}
```

In `crates/issuectl-core/src/transfer.rs`:
```rust
        let mut args = NewArgs {
            // ...
            description: self.description,
            structured_body: true, // Transferred bodies are inherently structured
```

### 2. Behavioral Regression for Free-Text File Users (Edge Case)

**Problem:** 
Users who historically used `--body-file` to pipe in a long plain-text log (e.g., `issuectl create --body-file crash.log`) relied on the generated `## Description` wrapper to anchor their text. 

With this change, the plain-text file is injected directly under the `# Title`. Worse, because the file lacks headings, if the repository schema enforces a `Description` section, the schema checker will blindly append an empty `## Description` stub at the *bottom* of the log.

Example resulting file:
```markdown
# App Crash

<100 lines of plain text from crash.log>

## Description
```

**Solution:**
While the strict contract change is justified, this UX trap is severe. 
1. Update `CHANGELOG.md` to explicitly warn users about this behavioral shift for plain-text files.
2. Recommend the standard shell workaround in the CLI help (`cmd/mod.rs`): 
   *"To inject plain text from a file into a generated description wrapper, use `--description \"$(cat file.txt)\"` instead."*

### 3. Solid Improvements

* **Horizontal Rule Truncation Fix (`new_issue.rs`):** Replacing `.split("---\n\n").nth(1)` with `crate::item_text::split(&render).body` is an excellent structural fix. The previous code silently truncated bodies containing standard thematic breaks (`---`), causing the schema checker to falsely append duplicate required-section stubs for valid headings located below the break.
* **Prompt/Template Synchronization:** The updates to `.claude/skills/issue/SKILL.md` and agent templates successfully clarify the boundaries between agent-generated structured Markdown and CLI responsibilities.

### Top 3 Priorities

1. **Verify and Fix `transfer.rs`:** Confirm whether `ImportRecord.description` carries a full structured body. If it does, change `structured_body: false` to `true` to avoid wrapping imported issues in duplicate `## Description` headings.
2. **Expose in `NewIssueRequest`:** Add `structured_body: bool` to the programmatic API payload so non-CLI clients are not blocked from utilizing structured bodies.
3. **Document Plain-Text File UX:** Add a changelog warning and CLI help note advising users how to handle long plain-text files without triggering bottom-appended schema stubs.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d4c88be0791f4e09a906f7c962b9385d]

## 1. Concrete problems, ordered by severity

### High, context-dependent: public struct changes may be a SemVer-breaking API change

- `crates/issuectl-core/src/mutate/new_issue.rs:65-72` adds a required field to public `NewArgs`.
- `crates/issuectl-core/src/write.rs:344-351` adds a required field to public `NewIssueArgs<'a>`.

Any downstream crate constructing either type with a struct literal will stop compiling:

```rust
NewArgs {
    kind,
    title,
    // Previously complete; now missing structured_body.
}
```

Updating all in-repository constructors does not preserve external compatibility. If `issuectl-core` exposes these modules/types as supported public API, this fix cannot ship as a patch release without either accepting the breaking change or redesigning the API.

Possible mitigations:

1. Keep the distinction in a private/internal render request rather than adding a public field.
2. Replace the ambiguous pair with a body-mode enum in the next breaking release:

```rust
pub enum InitialBody {
    None,
    Description(String),
    StructuredMarkdown(String),
}
```

3. If compatibility must be retained now, add a separate internal entry point used by the CLI while leaving existing public request structures untouched.

Whether this is a release blocker depends on the crate’s export and compatibility policy.

### Medium: `description + structured_body: bool` creates invalid and contradictory states

- `crates/issuectl-core/src/mutate/new_issue.rs:65-72`
- `crates/issuectl-core/src/write.rs:344-351`
- `crates/issuectl-core/src/write.rs:428-436`

The design allows combinations that have no coherent domain meaning:

- `description: None, structured_body: true`
- `description: Some("plain text"), structured_body: true`
- A caller populating a complete Markdown body but forgetting the boolean
- A future caller forwarding the boolean incorrectly, recreating this bug

The field named `description` no longer necessarily contains a description. This is fragile boolean coupling across `NewArgs`, `NewIssueArgs`, runtime dispatch, and the renderer.

The renderer currently silently accepts all invalid states:

```rust
if !args.structured_body {
    body.push_str("## Description\n\n");
}
if let Some(d) = args.description {
    body.push_str(d.trim_end());
    body.push('\n');
}
```

A typed representation would make the rendering decision exhaustive and local:

```rust
pub enum InitialBody<'a> {
    EmptyDescription,
    Description(&'a str),
    StructuredMarkdown(&'a str),
}

match args.body {
    InitialBody::EmptyDescription => body.push_str("## Description\n\n"),
    InitialBody::Description(text) => {
        body.push_str("## Description\n\n");
        body.push_str(text.trim_end());
        body.push('\n');
    }
    InitialBody::StructuredMarkdown(markdown) => {
        body.push_str(markdown.trim_end());
        body.push('\n');
    }
}
```

Even if the current patch is retained, add debug assertions or constructor methods so arbitrary combinations are not encouraged.

### Medium: the main CLI-to-schema interaction is not tested end to end

The important production path is:

```text
create --body-file
  -> dispatch_primary sets structured_body
  -> do_new_locked renders structured body
  -> schema detects present H2 sections
  -> only missing stubs are appended
```

Current coverage splits this across:

- CLI tests without schema requirements:
  - `crates/issuectl/tests/cli_new.rs:120-219`
- A direct core test that manually sets `args.structured_body = true`:
  - `crates/issuectl-core/src/mutate/new_issue.rs:696-719`

That leaves room for a regression in runtime propagation or repository schema loading that unit tests would not catch. Add an integration test creating a repository schema, invoking the actual binary with `--body-file`, and asserting:

- Existing `## Description` is not duplicated.
- Existing sections after `---` are recognized.
- Only missing sections are appended.
- Appended stubs occur after the supplied body.

For example:

```rust
#[test]
fn body_file_respects_required_sections_without_duplicates() {
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nbody_sections:\n  bug: [Description, Expected, Quick Test]\n",
    ).unwrap();

    // Invoke `issuectl create --body-file ...`.

    assert_eq!(item.matches("## Description").count(), 1);
    assert_eq!(item.matches("## Expected").count(), 1);
    assert_eq!(item.matches("## Quick Test").count(), 1);
}
```

### Low: CLI help omits the schema-appending qualification documented elsewhere

- `crates/issuectl/src/cmd/mod.rs:626-634`

The help now says the file is written directly under the preamble without a wrapper, but does not mention that schema-required H2 stubs can still be appended. The agent templates do include that qualification.

This is not a contradiction, but it is an observable part of `--body-file` behavior and should be visible in `--help`:

```rust
/// Repository schema requirements may append stubs for missing H2 sections.
```

### Low: empty structured bodies have poorly defined output

- `crates/issuectl/src/cmd/runtime.rs:589-596`
- `crates/issuectl-core/src/write.rs:428-436`

An empty file or empty stdin sets `structured_body = true`, suppresses `## Description`, and leaves an issue with only the generated title/source unless schema requirements restore sections. The renderer also appends one extra newline for an empty string.

This may be acceptable under “the file is the complete body,” but it is not documented or tested. Decide explicitly whether empty body files should:

1. Be accepted as a deliberately empty structured body.
2. Fall back to the normal empty `## Description`.
3. Be rejected as invalid input.

The current accidental behavior should not define the contract.

## 2. Questionable decisions and missing edge cases

### The body mode should be derived at the boundary and represented explicitly

`crates/issuectl/src/cmd/runtime.rs:589-596` correctly distinguishes `body_file.is_some()` before consuming the option. The weakness is carrying that distinction through multiple layers as a boolean attached to a misleading `description` field.

The CLI boundary should construct a typed body value once. Core rendering should not need to remember that two independently supplied fields must agree.

### No test defines handling of leading/trailing whitespace in structured bodies

`crates/issuectl-core/src/write.rs:433-436` applies `trim_end()` to structured Markdown. Therefore body-file bytes are not preserved exactly:

- Trailing blank lines are removed.
- Trailing spaces on the final line are removed.
- A single newline is added.

This behavior predates the mode distinction for descriptions, but it matters more now that documentation describes the input as a “complete structured Markdown body.” If exact preservation is not promised, the documentation should avoid implying byte-for-byte insertion. At minimum, add a test fixing the intended normalization behavior.

Leading blank lines are retained, potentially creating more vertical whitespace after the title/source preamble. That is harmless Markdown-wise but currently unspecified.

### No explicit test for a structured body without `## Description`

The modified legacy test at `crates/issuectl/tests/cli_new.rs:1247-1266` verifies that plain paragraphs from a file do not receive a wrapper. That is useful, but schema behavior could substantially alter this:

- Without schema: no `## Description` is produced.
- With `Description` required by schema: an empty `## Description` stub is appended after the supplied paragraphs.

That outcome follows the stated contract, but deserves a test because it may surprise users who supply unsectioned Markdown in a schema-enabled repository.

### No rejection or normalization of issue-level frontmatter/H1 content

The contract says “structured Markdown body,” not a full `item.md`, so accepting input containing YAML frontmatter or another H1 may be intentional. Still, common misuse is likely:

```markdown
---
status: open
---

# My issue

## Description
...
```

The result will embed those constructs beneath the generated H1. This is not a regression introduced by the patch, and it does not need to block the fix, but the documentation could state that `--body-file` expects body Markdown only—no issue frontmatter and no generated title.

### The horizontal-rule fix is valuable but broader than the immediate rendering change

- `crates/issuectl-core/src/mutate/new_issue.rs:351-359`

Replacing:

```rust
render.split("---\n\n").nth(1)
```

with:

```rust
crate::item_text::split(&render).body
```

is the correct architectural direction because body Markdown may contain thematic breaks. However, this relies on `item_text::split` being the canonical parser for rendered item text and correctly handling malformed or delimiter-like content in YAML scalars. The supplied diff does not expose that implementation.

This is not a demonstrated defect, but it should be covered by existing parser tests rather than only indirectly through the new issue test.

### Integration assertions are somewhat brittle

- `crates/issuectl/tests/cli_new.rs:120-219`

Several tests assert exact `ends_with(...)` formatting. That is appropriate for this whitespace-sensitive bug, but it also couples CLI behavior to renderer spacing. Keep one exact structural assertion, but use parsed body/H2 assertions for schema tests so harmless frontmatter or formatting changes do not create noisy failures.

### `run_with_stdin` is safe for the current small fixture but not a robust general helper

- `crates/issuectl/tests/cli_new.rs:55-78`

The parent writes stdin synchronously while stdout and stderr are piped but undrained. A child that emits enough output before consuming all stdin can deadlock. The current small successful body does not trigger that, so this is not an in-scope production issue. Do not reuse this helper for large-input or input-cap tests without concurrent I/O handling.

## 3. Context-dependent or disputed concerns

### Public API break severity

The public-field concern depends on whether:

- `mutate::new_issue::NewArgs` is publicly exported from `issuectl-core`.
- `write::NewIssueArgs` is externally reachable.
- The crate promises SemVer compatibility for these types.
- The pending release is allowed to be breaking.

If either struct is internal despite being declared `pub`, this drops from a high-severity compatibility issue to an internal design smell.

### Empty body-file behavior

Accepting an empty file as an empty structured body is defensible. Falling back to `## Description` would blur the new contract, while rejecting it may be unnecessarily strict. The problem is absence of an explicit contract, not necessarily the chosen behavior.

### Validation of headings inside fenced code and other Markdown constructs

Schema completeness depends on `body_sections::all_h2_sections`. If that parser uses Markdown-aware extraction, the interaction is sound. If it merely scans lines beginning with `## `, headings inside fenced examples could suppress required stubs incorrectly. That appears pre-existing, but structured files are more likely than plain descriptions to contain complex Markdown, so this change increases exposure.

## 4. What is solid

No unconditional blocker is visible in the supplied diff.

The main behavior is propagated consistently across the visible code:

- `create --body-file` sets `structured_body = true`.
- Inline `--description` remains wrapped.
- Intake, recurrence, transfer, and the public new API explicitly retain free-text behavior with `false`.
- `Default` preserves historical behavior.
- Rendering places structured content after the title and optional source.
- Schema section detection now uses the canonical item splitter, avoiding truncation at body horizontal rules.
- File and stdin CLI paths are both exercised.
- Source preamble ordering is tested.
- Shipped templates and dogfooded Claude/Codex artifacts are synchronized.
- The changelog accurately describes the user-visible fix.

## 5. Top 3 priorities

1. **Resolve the public API compatibility risk** around adding fields to `NewArgs` and `NewIssueArgs`; do not ship it as a patch-level library change if those types are supported exports.
2. **Replace the boolean-plus-description representation with a typed body mode**, or at least hide construction behind constructors to prevent invalid combinations.
3. **Add a real CLI integration test with repository schema requirements**, including an existing section after `---` and one missing section that must be appended.

## Context request

- **Kind:** artifact  
  **Need:** `crates/issuectl-core/src/lib.rs`, relevant `mod.rs` re-exports, and `crates/issuectl-core/Cargo.toml` package/version metadata.  
  **Why:** Determines whether adding fields to `NewArgs` and `NewIssueArgs` is an externally observable SemVer-breaking change and therefore a release blocker.

- **Kind:** artifact  
  **Need:** Implementations and tests for `crate::item_text::split` and `crate::body_sections::all_h2_sections`.  
  **Why:** Determines whether schema detection safely handles canonical frontmatter, fenced code blocks, and Markdown thematic breaks; this could elevate the parser interaction from a disputed concern to a correctness defect.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_ce1e9e959b8f40d9be397da57b30df09]

# Review: `create --body-file` structured-body fix

## 1. Concrete problems, by severity

### P1 — This is a behavior change for **all** `--body-file` users, shipped as a "Fixed" entry

The old behavior wrapped *any* body-file content under `## Description`. The new behavior drops the wrapper unconditionally when `--body-file` is used (`crates/issuectl/src/cmd/runtime.rs`, `let structured_body = body_file.is_some();`). A user/script piping free prose via `--body-file -` (a legitimate pattern for long text that doesn't fit argv) previously got a well-formed `## Description` section; now they get headingless prose directly under `# <title>`.

- `CHANGELOG.md` files this under **Fixed**, phrased as if it only affects bodies that already begin with `## Description`. It affects every `--body-file` invocation. This belongs under **Changed** (arguably breaking, depending on your semver posture for CLI output contracts).
- There is no detection, warning, or fallback for a body file containing **zero H2 headings**. If the "structured body" contract is real, the CLI should at minimum warn when the supplied file has no H2s (the machinery exists — `crate::body_sections::all_h2_sections` — and the CLI already warns on legacy `## Notes`).
- An alternative, strictly narrower fix — skip the wrapper only when the body already starts with an H2, or dedupe a leading `## Description` — would have fixed the reported bug with zero behavior change for prose bodies. The chosen contract is defensible long-term, but the diff neither documents this trade-off nor mitigates it.

### P2 — `structured_body: bool` is an API smell that makes an illegal state representable

`NewArgs.structured_body` (`crates/issuectl-core/src/mutate/new_issue.rs:65`) and `NewIssueArgs.structured_body` (`crates/issuectl-core/src/write.rs:344`) are booleans orthogonal to `description: Option<String>`. `structured_body: true` + `description: None` is meaningless but constructible, and in `render_new_item_from_fm` it silently produces a body that is *only* `# Title\n\n` — no section at all (until schema stubs, if any, are appended). The correct shape is a body-source enum:

```rust
pub enum BodySource {
    None,
    FreeText(String),   // gets ## Description wrapper
    Structured(String), // placed verbatim below preamble
}
```

This eliminates the invalid state, removes the six mechanical `structured_body: false` call-site edits, and makes future body sources (templates?) a variant instead of another bool. Since `NewArgs` has public fields and gained a field anyway, downstream struct-literal constructors are already source-broken by this diff — the migration cost of the enum was already being paid. If these crates are published, note this is a semver-breaking struct change either way.

### P2 — `new_api::new_issue` cannot express structured bodies

`crates/issuectl-core/src/mutate/new_api.rs` hardcodes `structured_body: false` and `NewIssueRequest` gains no field. If `new_api` is the stable programmatic surface (MCP server, embedders), those consumers cannot replicate `create --body-file` semantics and are stuck with the wrapper. "Non-create paths preserve prior semantics" is a reasonable contract for `intake::file`, `recurrence::materialize`, and `transfer::ImportRecord`, but `new_api` is the *same* create operation exposed as a library call. This looks like an oversight rather than a decision; if it is a decision, it needs a doc comment on `NewIssueRequest` saying so.

### P3 — Schema-stub matching against user-supplied headings is untrusted-input territory now

`do_new_locked` (`new_issue.rs:351`) computes `missing = required_sections − all_h2_sections(body)`. Previously the H2 set came from the renderer's own output; now it comes from arbitrary user Markdown. Without seeing `all_h2_sections`, I assume exact string matching. Then:

- `## description` / `##  Description` / `## Description:` in the body file → stub `## Description` appended → near-duplicate sections, which is exactly the bug class this diff claims to fix, resurrected via case/whitespace variance.
- H2s inside fenced code blocks in the body file may be counted as present (or a `## Foo` inside a code fence suppresses a stub) depending on whether `all_h2_sections` is fence-aware.

Neither the core test (`structured_body_with_horizontal_rule_does_not_duplicate_required_sections`) nor the CLI tests cover heading-variant or code-fence cases.

### P3 — The `item_text::split` fix is correct but under-tested for the paths it also changed

The old `render.split("---\n\n").nth(1)` bug affected **every** creation path with schema-required sections, including inline `--description` containing an HR (`---`). The fix (`new_issue.rs:351–355`) applies to all paths, but the only new test exercises the structured path. Add a free-text `--description "para\n\n---\n\npara"` + schema test to lock in the fix for the historical path too — that's the path where a silent behavioral change (fewer duplicate stubs) actually shipped to existing users.

### P4 — Test-harness nits in `crates/issuectl/tests/cli_new.rs`

- `run_with_stdin`: `write_all` will panic on broken pipe if the child errors out and closes stdin before reading (usage-error paths). Fine for the current happy-path test; will bite the first person who reuses the helper for a failure case. Ignore `ErrorKind::BrokenPipe` or document the constraint.
- `body_file_is_structured_markdown_without_description_wrapper` and the stdin test assert with `ends_with` on the whole tail — brittle against future renderer changes (e.g., a trailing schema stub if the default repo ever grows a schema), but acceptable as intentional structural pinning.

## 2. Questionable decisions / missing edge cases

- **No H1 collision handling.** A body file starting with `# Some Other Title` yields two H1s under the generated `# <title>`. No warning. Given the docs now call this a "complete structured Markdown body," specify whether an H1 is allowed and warn if present.
- **Empty body file.** `parse_non_empty` guards `--description`; whether `read_body_file_arg` rejects an empty file/stdin is not visible in the diff. Previously an empty file still produced a `## Description` section; now it produces a headingless empty body. If empty input is accepted, that's a degraded output.
- **Body file containing its own YAML frontmatter** (`---\n...\n---`) is embedded verbatim into the body. `item_text::split` on the rendered doc should still find the real frontmatter first, so stub detection survives — but the resulting item has a fake frontmatter block mid-body. Worth a warning.
- **Stub ordering.** Schema stubs append after the structured body regardless of schema order (the test pins `## Quick Test` last even though the schema lists it after `Expected`). Pre-existing behavior, but user-supplied section order + appended stubs makes ordering violations more visible now.
- **Four-way doc duplication** (`.claude/skills/issue/SKILL.md`, `.codex/prompts/issue.md`, `templates/issue-prompt.md`, `templates/issue-skill.md`). Kept in sync here, but if the dotfile copies aren't generated from the templates by a build/check step, drift is inevitable. Pre-existing smell, amplified by every doc-touching change like this one.

## 3. Context-dependent / disputed concerns

- **Import round-trip wrapper duplication.** `transfer.rs ImportRecord::into_new_args` sets `structured_body: false`. If the export side serializes the full body (including `## Description`) into `description`, re-import now double-wraps — pre-existing, not a regression from this diff, but the new flag is exactly the tool to fix it and it wasn't used. Needs the export code to confirm.
- **`all_h2_sections` matching semantics** (case, trailing punctuation, code-fence awareness) determine whether P3 above is real or already mitigated.
- **Published-crate status** of `issuectl-core` determines whether the `NewArgs`/`NewIssueArgs` field additions require a semver-minor/major bump.

## 4. What is solid

The wrapper suppression itself is minimal and correctly placed in the single renderer (`write.rs render_new_item_from_fm`). All six `NewArgs`-constructor call sites are covered — I found no missed site in the diff. The `split("---\n\n")` → `item_text::split` fix is a genuine latent-bug kill with a targeted regression test. CLI/stdin/`--source` interaction is tested at the integration level, the pre-existing `new_body_file_writes_markdown_below_heading` test was correctly updated to the new contract rather than deleted, and help text, skill docs, and changelog were all touched (categorization aside). No blocker to merging, provided the P1 contract change is a deliberate, acknowledged decision.

## 5. Top 3 priorities

1. **Own the P1 behavior change:** move the changelog entry to Changed (or reword to state it affects all `--body-file` inputs), and add a CLI warning when a `--body-file` body contains no H2 headings.
2. **Extend structured-body support to `new_api::new_issue`** or document why the programmatic API deliberately can't express it; replace the `structured_body` bool with a body-source enum while the API is already breaking.
3. **Harden schema-stub matching against user-supplied bodies:** tests (and normalization if needed) for heading case/whitespace variants and H2s inside code fences; add a free-text-with-HR schema test to pin the `item_text::split` fix on the historical path.

## Context request
- Kind: artifact
  Need: `crate::body_sections::all_h2_sections` implementation (crates/issuectl-core/src/body_sections.rs)
  Why: Determines whether P3 (case/whitespace/code-fence heading mismatches causing duplicate stubs) is a real defect or already handled; changes whether normalization work is required.
- Kind: artifact
  Need: `read_body_file_arg` in crates/issuectl/src/cmd/ (runtime.rs or helper module)
  Why: Confirms whether empty body files/stdin are rejected; changes the "headingless empty body" edge-case finding from actual to moot.
- Kind: artifact
  Need: Export-side serialization in crates/issuectl-core/src/transfer.rs (how `description` is populated on export)
  Why: Confirms or refutes the import round-trip double-wrapper concern and whether `ImportRecord` should set `structured_body: true`.
- Kind: clarification
  Need: Are `issuectl-core`/`issuectl` published crates, and is there a build/check step that syncs `.claude/`/`.codex/` copies from `crates/issuectl-core/templates/`?
  Why: Determines semver impact of the `NewArgs` field addition and whether the four-way doc duplication is a real drift risk or generated output.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_a953dee4a7ee49ae9ca03fb7ffaf916e]

## Concrete problems, ordered by severity

### 1. `--body-file` is a user-visible semantic change classified as a bug fix
`crates/issuectl/src/cmd/runtime.rs` now sets:

```rust
let structured_body = body_file.is_some();
```

and `crates/issuectl-core/src/write.rs` now suppresses the generated `## Description` wrapper whenever that flag is set:

```rust
if !args.structured_body {
    body.push_str("## Description\n\n");
}
```

Before this change, every `--body-file` body was placed under a generated `## Description`. After it, the file is emitted verbatim after the generated title/source preamble.

That is not only deduplication. For any existing user with a body file like:

```markdown
First paragraph.

Second paragraph.
```

the old output was:

```markdown
# Title

## Description

First paragraph.

Second paragraph.
```

The new output is:

```markdown
# Title

First paragraph.

Second paragraph.
```

If the repository schema requires `Description`, the user’s text is not inside that section; instead an empty `## Description` stub is appended after the text. If the schema does not require `Description`, the body has no `Description` section at all.

This is a breaking CLI contract and should not be buried under `### Fixed` in `CHANGELOG.md`. It should either be:

- classified as `### Changed`,
- released with a migration/deprecation note, or
- gated so only body files that already begin with structured H2 content skip the wrapper.

Recommended minimal fix:

```rust
let structured_body = body_file.is_some();
```

becomes something like:

```rust
let structured_body = read_body_file_arg(&path)?;
let structured_body = crate::body_sections::starts_with_h2(&structured_body);
```

or add an explicit `--body-file` contract version/flag rather than changing the meaning of an existing flag in place.

---

### 2. `structured_body` is ephemeral; all non-create paths force `false`
The flag is only carried through `NewArgs`/`NewIssueArgs` during CLI creation. Every other initializer introduced in this diff hardcodes `structured_body: false`:

- `crates/issuectl-core/src/mutate/recurrence.rs`
- `crates/issuectl-core/src/transfer.rs`
- `crates/issuectl-core/src/mutate/intake.rs`
- `crates/issuectl-core/src/mutate/new_api.rs`

If any of those paths can receive a complete structured body — for example an imported issue, a recurrence template whose `description` contains `## Description`, or an intake `FileRequest` body — the exact duplicate-wrapper bug reappears because `write.rs` will wrap that structured text under a generated `## Description`.

The current tests only cover CLI `create --body-file`. They do not cover structured content flowing through `recurrence`, `transfer`, `intake`, or `new_api`.

This is the strongest architectural weakness. The semantic distinction between free text and structured Markdown is stored nowhere in the persisted issue model, so it cannot be respected by later materialization paths. Prefer an explicit body-kind marker persisted in the issue/recurrence/import model, not a process-local bool.

---

### 3. Public Rust API break risk
`crates/issuectl-core/src/mutate/new_issue.rs`:

```rust
pub struct NewArgs {
    ...
    pub structured_body: bool,
    ...
}
```

and `crates/issuectl-core/src/write.rs`:

```rust
pub struct NewIssueArgs<'a> {
    ...
    pub structured_body: bool,
    ...
}
```

If these structs are part of a published crate API, adding a public field is a semver-breaking change for any external struct literal construction. Even if they are internal in practice, the change should be explicit. Depending on derive/serde attributes, adding the field can also break deserialization unless `#[serde(default)]` is present.

If compatibility matters, use `#[non_exhaustive]`, a builder, or make the field private with a constructor.

---

### 4. No upgrade warning or validation for ambiguous body-file content
The new help text redefines `--body-file` as “complete structured Markdown”, but there is no validation and no warning when:

- the file has no H2 sections and no schema-required `Description`,
- the file contains a top-level H1, producing multiple titles,
- the file looks like old free-text input.

At minimum, a warning for body-file content with no H2 section would make the breaking change visible rather than silent.

---

### 5. Test coverage misses the CLI/schema composition path
`crates/issuectl-core/src/mutate/new_issue.rs` has a solid unit test for the horizontal-rule/required-sections fix, but there is no CLI integration test combining all of:

- `create --body-file`,
- repository `.schema.yaml` required sections,
- body content with a horizontal rule,
- `--source`.

The existing CLI tests `body_file_is_structured_markdown_without_description_wrapper` and `body_file_stdin_with_source_preserves_preamble_without_description_wrapper` rely on the body already containing all required sections, so they do not prove that the new canonical splitter and schema stub appending behave correctly through `runtime.rs`.

---

## Questionable decisions and missing edge cases

### Boolean instead of an enum
`structured_body: bool` is a weak model for a semantic with more than two future states. A body can be:

- free text requiring a `## Description` wrapper,
- complete structured Markdown,
- potentially frontmatter-bearing Markdown,
- import/recurrence materialized content.

An enum such as:

```rust
enum BodyKind {
    FreeText,
    Structured,
}
```

would be clearer and would prevent accidental `false` propagation. At minimum, the current bool must be renamed to make the semantics unambiguous: `description_is_structured_body` is better than `structured_body`, because `NewArgs` already has multiple body-related fields.

### `body_file.is_some()` is used as the only signal
`crates/issuectl/src/cmd/runtime.rs` derives structured-body behavior from the presence of `--body-file`. This means:

- there is no way to pass free text from a file and retain the historical wrapper,
- there is no content-based detection of old-style input,
- stdin `-` and regular file paths share the same breaking semantic.

If backward compatibility matters, a separate flag such as `--structured-body-file` or `--wrap-body-file` would allow both behaviors.

### Required-section stubs with a body whose text appears before the first H2
With `structured_body = true`, a plain-text body followed by schema-required stubs produces:

```markdown
# Title

Plain text.

## Description

## Expected

## Quick Test
```

The plain text is not part of `## Description`. That may be the intended new contract, but it is surprising and undocumented for users migrating from the old behavior.

### Downstream constructors are silently frozen to legacy semantics
The added `structured_body: false` lines in `intake.rs`, `new_api.rs`, `recurrence.rs`, and `transfer.rs` are easy to miss in future maintenance. There is no comment at those call sites explaining why they intentionally use free-text rendering. If those constructors ever receive structured input, the regression will be silent.

---

## Context-dependent or disputed concerns

- **Is `--body-file` already released?** If this is the same Unreleased cycle that introduced `--body-file`, the semantic change may not be a production compatibility break. If released, Problem 1 is likely a release blocker. The changelog and version context would settle this.
- **Do `intake`, `import`, recurrence templates, or `new_api` descriptions ever carry full structured Markdown bodies?** If yes, Problem 2 is a real in-scope bug. If those paths are always free-text fields by contract, it is only a design weakness.
- **What exactly does `item_text::split` guarantee?** The horizontal-rule fix depends on its behavior for frontmatter, CRLF, missing frontmatter, and body text beginning with `---`. The new unit test covers the main case, but not all edge conditions.
- **Are `NewArgs`/`NewIssueArgs` public API and do they derive `Deserialize`?** If so, the new field may break external Rust consumers or JSON callers unless serde defaults are supplied.

---

## What is solid

- The renderer change correctly removes the generated wrapper only for `create --body-file`.
- Inline `--description` retains the legacy `## Description` wrapper.
- The required-section splitter fix from `render.split("---\n\n")` to `crate::item_text::split(&render).body` is the right direction and directly addresses the horizontal-rule truncation bug.
- The unit test for horizontal rules plus required sections is a good regression test for the original duplicate-stub failure mode.
- CLI tests cover file, stdin, `--source`, and inline description behavior.
- Help text, shipped templates, `.claude` skill, `.codex` prompt, and `CHANGELOG.md` were consistently updated.

---

## Top 3 priorities

1. **Resolve the `--body-file` compatibility break.** Decide whether this is a Changed release with migration guidance, or make the structured-body behavior opt-in/content-dependent rather than implicit for every `--body-file`.
2. **Persist body-kind semantics, or explicitly prove and document why `recurrence`, `transfer`, `intake`, and `new_api` must always use `structured_body: false`.** Add regression tests if any of those paths can transport structured bodies.
3. **Audit `NewArgs`/`NewIssueArgs` public API and serde impact.** Add `#[serde(default)]` or `#[non_exhaustive]`/builder options if these crates have external consumers.

I do not see a panic or data-loss bug in the main `create --body-file` happy path itself. The most serious issue is that the fix silently changes an existing CLI contract and leaves the new structural distinction isolated to one ephemeral constructor path.

## Context request

- Kind: `artifact`
  - Need: `crates/issuectl-core/src/item_text.rs` or wherever `item_text::split` is implemented.
  - Why: Confirms behavior for missing frontmatter, CRLF, and body starting with `---`; could materially affect Problem 4 and the horizontal-rule fix.

- Kind: `artifact`
  - Need: Top of `crates/issuectl-core/src/mutate/new_issue.rs` and `crates/issuectl-core/src/write.rs`, including derives/serde attributes for `NewArgs` and `NewIssueArgs`.
  - Why: Determines whether the new public field is a semver/deserialization break.

- Kind: `clarification`
  - Need: Whether `--body-file` has been released previously or is new in the same Unreleased cycle.
  - Why: Determines whether the semantic change is a production compatibility break or only an in-development contract change.

- Kind: `clarification`
  - Need: Whether `intake::FileRequest` bodies, imported issue descriptions, recurrence template `description`, and `new_api` descriptions are ever expected to be complete structured Markdown.
  - Why: Determines whether the forced `structured_body: false` in those paths is a real unresolved bug or an acceptable legacy free-text contract.

## Fresh adopted-tree supplement — bounded context follow-ups

[thread_id:group_28c3be9efb394e79b7323d7f4d6c4212]

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d4c88be0791f4e09a906f7c962b9385d]

# Revised review

## Conclusions changed by the supplied context

- **Public API compatibility concern: withdrawn.**  
  Although `NewArgs` and `NewIssueArgs` are public Rust structs, `crates/issuectl-core/src/lib.rs` explicitly disclaims API stability, both modules are `#[doc(hidden)]`, and `AGENTS.md` defines the binary CLI as the SemVer contract. Adding `structured_body` is not a release blocker under this repository’s compatibility policy.

- **Frontmatter/parser concern: resolved.**  
  `item_text::split` correctly selects the generated item’s first valid closing frontmatter delimiter. A later body-level `---` cannot truncate the body. `body_sections::all_h2_sections` is fence-aware, including longer CommonMark closing fences, so fenced pseudo-headings do not incorrectly satisfy schema requirements.

- **Empty-input concern: withdrawn.**  
  `crates/issuectl/src/cmd/write.rs::read_body_file_arg` rejects empty and whitespace-only file/stdin input before `structured_body` reaches the renderer. The invalid combination `description: None, structured_body: true` remains representable internally, but it is not reachable through `create --body-file`.

- **Foreign/import body handling: not an in-scope defect.**  
  `transfer::ImportRecord::into_new_args` intentionally retains free-text rendering. The issuectl export/import aliasing problem is a separately reproduced and filed defect requiring a different design. This patch should not opportunistically change it.

- **Overall correctness conclusion:** no actual in-scope production defect is demonstrated by the final diff and supplied context. There is no blocker.

## 1. Concrete problems, ordered by severity

### Medium: no black-box CLI test combines `--body-file` with schema-required sections

Relevant paths:

- `crates/issuectl/src/cmd/runtime.rs::dispatch_primary`
- `crates/issuectl-core/src/mutate/new_issue.rs::do_new_locked`
- `crates/issuectl/tests/cli_new.rs`
- `crates/issuectl-core/src/mutate/new_issue.rs::structured_body_with_horizontal_rule_does_not_duplicate_required_sections`

The tests establish the behavior in two pieces:

1. Black-box CLI tests prove that file/stdin selects structured rendering.
2. A core test proves that structured rendering interacts correctly with schema-required sections and a horizontal rule.

That is strong evidence, but the primary user-visible contract spans both layers. There is no single built-binary test proving:

```text
create --body-file
  -> structured_body = true
  -> repository schema loaded
  -> existing H2 sections retained
  -> only missing stubs appended
```

A regression in `dispatch_primary` could still leave the core schema test green, while a schema regression could leave the simple CLI tests green.

Add one integration test to `crates/issuectl/tests/cli_new.rs`:

```rust
#[test]
fn body_file_schema_appends_only_missing_sections() {
    let tmp = fresh_repo();
    std::fs::write(
        tmp.path().join("issues/.schema.yaml"),
        "version: 1\nbody_sections:\n  bug: [Description, Expected, Quick Test]\n",
    )
    .unwrap();

    let body_path = tmp.path().join("body.md");
    std::fs::write(
        &body_path,
        "## Description\n\nObserved.\n\n---\n\n## Expected\n\nExpected.",
    )
    .unwrap();

    let out = run(
        tmp.path(),
        &[
            "create",
            "--type",
            "bug",
            "--title",
            "Schema body",
            "--slug",
            "schema-body",
            "--body-file",
            body_path.to_str().unwrap(),
        ],
    );

    assert_eq!(out.status.code(), Some(0), "{}", dump(&out));

    let item =
        std::fs::read_to_string(tmp.path().join("issues/schema-body/item.md")).unwrap();

    assert_eq!(item.matches("## Description").count(), 1, "{item}");
    assert_eq!(item.matches("## Expected").count(), 1, "{item}");
    assert_eq!(item.matches("## Quick Test").count(), 1, "{item}");
    assert!(
        item.contains("---\n\n## Expected\n\nExpected.\n\n## Quick Test\n"),
        "{item}"
    );
}
```

This is a coverage gap, not evidence that the implementation is wrong.

### Low: CLI help does not mention schema-appended stubs

- `crates/issuectl/src/cmd/mod.rs`, `PrimaryCommand::Create::body_file`

The help text says:

> write it directly below the `# <title>`/`--source` preamble without adding a wrapper heading

That is accurate, but incomplete: schema-required H2 stubs may be appended after the supplied body. The shipped agent templates explicitly state this qualification, while `--help`—defined by `AGENTS.md` as the source of truth for accepted flags—does not.

Add:

```rust
/// Repository schema requirements may append stubs for missing H2 sections.
```

This omission is minor because it does not contradict actual behavior and the agent contract is already correct.

## 2. Questionable decisions and missing edge cases

### Boolean mode coupled to a field still named `description`

- `crates/issuectl-core/src/mutate/new_issue.rs::NewArgs`
- `crates/issuectl-core/src/write.rs::NewIssueArgs`
- `crates/issuectl-core/src/write.rs::render_new_item_from_fm`

The representation remains weaker than the domain model:

```rust
description: Option<String>,
structured_body: bool,
```

It permits contradictory internal states and makes future constructors responsible for setting two related fields consistently. The current diff propagated `false` through all visible non-CLI creation paths and `true` only through `create --body-file`, so there is no current functional defect. The internal, explicitly unstable crate also removes the earlier compatibility objection.

Still, a typed mode would be harder to misuse:

```rust
enum InitialBody<'a> {
    EmptyDescription,
    Description(&'a str),
    StructuredMarkdown(&'a str),
}
```

This should be treated as a maintainability improvement, not a prerequisite for this bug fix. A narrow boolean patch is defensible here because the required compatibility policy is source-dependent and no content sniffing is intended.

### Missing explicit regression assertions for non-create creation paths

The constructors correctly set `structured_body: false` in:

- `crates/issuectl-core/src/mutate/intake.rs::file`
- `crates/issuectl-core/src/mutate/new_api.rs::new_issue`
- `crates/issuectl-core/src/recurrence.rs::materialize`
- `crates/issuectl-core/src/transfer.rs::ImportRecord::into_new_args`

This is correct propagation and satisfies the stated compatibility contract. However, the new diff mostly proves it through constructor literals rather than targeted behavior tests.

At least one table-style core test could assert that the old free-text mode still produces:

```markdown
## Description

<text>
```

The CLI inline-description integration test already covers the main binary contract. Separate tests for every internal path are not mandatory, but recurrence and intake are particularly vulnerable to future constructor drift.

### No direct boundary test for `read_body_file_arg`

The supplied implementation establishes that:

- Empty input is rejected.
- Whitespace-only input is rejected.
- Leading whitespace is preserved.
- Trailing whitespace is normalized.
- File and stdin reads are capped.
- Invalid UTF-8 is rejected.

The new CLI tests cover successful file and stdin use, but the diff does not add a body-file-specific regression test for empty input or leading indentation. These behaviors appear intentional and documented in the function comment.

Useful focused tests in `crates/issuectl/src/cmd/write.rs` would pin:

```rust
assert!(read_body_file_arg(empty_path).is_err());
assert!(read_body_file_arg(whitespace_path).is_err());
assert_eq!(
    read_body_file_arg(indented_path).unwrap(),
    "    code block"
);
```

This is not a newly introduced risk if generic capped-input tests already cover the underlying helpers.

### Structured body normalization should remain explicit

`read_body_file_arg` calls `trim_end()`, and `render_new_item_from_fm` calls `trim_end()` again. Therefore “written directly below” means semantic Markdown insertion, not byte-preserving insertion. Final trailing blank lines and spaces are normalized, while leading whitespace is preserved.

The comments document this correctly. The duplicated normalization is harmless and idempotent, but the renderer-level `trim_end()` means any future non-CLI structured-body caller also receives normalization. That should remain deliberate.

### Full-document input is accepted as body content

A user can pass a body file containing another H1 or YAML-looking delimiters. The generated title and frontmatter remain authoritative; the supplied material is embedded below them. This follows the established “initial body below generated H1” contract and is not a regression.

Do not add content sniffing. If misuse becomes common, clarify that `--body-file` accepts body Markdown, not a complete `item.md`.

## 3. Context-dependent or disputed concerns

### `item_text::split` fence handling is unusual but not defective for this path

`item_text::split` tracks Markdown fences while scanning what is nominally YAML frontmatter. That design is broader than standard frontmatter parsing, but for generated items the frontmatter is known-shape serialized YAML and immediately followed by a canonical closing delimiter. The new use in `do_new_locked` is safe.

Most importantly, it fixes the concrete failure of:

```rust
render.split("---\n\n").nth(1)
```

which could mistake a body thematic break for a delimiter and omit headings after it from schema detection.

### H2 detection is intentionally exact

`all_h2_sections` recognizes canonical lines beginning exactly with `## ` outside fences. It does not implement every possible CommonMark heading form, such as alternate syntax or deeply container-nested headings. That is an existing body-section schema convention, documented as H2 sections, not a regression caused by structured body files.

### Integration helper deadlock concern remains out of scope

`crates/issuectl/tests/cli_new.rs::run_with_stdin` writes stdin synchronously while stdout/stderr are piped. It could deadlock if reused with very large input and a child that fills output pipes before consuming stdin. The current fixture is tiny and the successful create path emits negligible output. This is test-helper hardening, not a production concern and not relevant to accepting this patch.

## 4. What is solid

- `dispatch_primary` derives the mode from the source before consuming `body_file`:
  ```rust
  let structured_body = body_file.is_some();
  ```
  This avoids content sniffing and cleanly distinguishes file input from inline free text.

- File and `-` stdin paths both flow through `read_body_file_arg`, preserving:
  - the 10 MiB cap,
  - UTF-8 validation,
  - terminal-stdin rejection,
  - empty-input rejection,
  - the literal `./-` escape.

- Rendering order is correct:
  1. Generated H1.
  2. Optional source line.
  3. Structured body directly, or generated `## Description` for free text.

- All visible non-create constructors explicitly preserve free-text behavior with `structured_body: false`.

- `NewArgs::default()` preserves prior behavior.

- Schema handling is correct:
  - `item_text::split` extracts the actual rendered body.
  - Horizontal rules in the body do not truncate scanning.
  - `all_h2_sections` ignores headings inside fenced code.
  - Existing required sections are not duplicated.
  - Missing sections are appended after the supplied body.

- Tests cover:
  - file input,
  - stdin input,
  - optional source ordering,
  - structured body beginning with `## Description`,
  - inline description retaining its wrapper,
  - unsectioned file Markdown receiving no unconditional wrapper,
  - horizontal rules plus schema-required sections at the core layer.

- The changelog accurately describes the fix.

- Both bundled templates and dogfooded Claude/Codex copies are synchronized. The enforced template-copy test further reduces drift risk.

## 5. Top 3 priorities

1. **Add one black-box CLI integration test combining `--body-file`, schema requirements, a horizontal rule, existing H2s, and one missing stub.**
2. **Add the schema-stub qualification to `create --body-file` help text.**
3. **Optionally replace `description + structured_body` with a typed body mode in a later cleanup; do not expand this bug fix merely to achieve that refactor.**

No blocker remains after reviewing the supplied context. The intended compatibility contracts leave no demonstrated in-scope defect; the remaining issues are test completeness, minor help precision, and internal API robustness.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_ce1e9e959b8f40d9be397da57b30df09]

# Revised review with supplied context

## Conclusions that change

**P1 (behavior change for all `--body-file` users) — withdrawn as a defect, retained as a changelog nit.** The v0.7.0 documented contract already stated the file's markdown is "written below the `# <title>` heading" — the injected `## Description` wrapper was the code contradicting its own docs, not the contract. Callers relying on the wrapper for prose body-files were relying on undocumented behavior that the skill templates never promised. The "Fixed" categorization in `CHANGELOG.md` is therefore defensible. Remaining nit: the entry describes only the duplicate-heading symptom; a prose-only `--body-file` also loses its wrapper now, and one clause acknowledging that would spare a confused bisect. The alternative content-sniffing fix I proposed is explicitly not intended, so that criticism is moot.

**P2 (semver-breaking struct change) — withdrawn entirely.** `issuectl-core/src/lib.rs` marks every module `#[doc(hidden)]` with an explicit no-stable-API disclaimer, and AGENTS.md states the semver contract is the binary CLI surface only. The `NewArgs`/`NewIssueArgs` field additions carry no compatibility obligation. The **enum-over-bool design point stands** but is downgraded from API smell to internal style preference: `structured_body: true` + `description: None` remains a constructible-but-meaningless state that renders a body with no section at all; a `BodySource` enum would eliminate it. Non-blocking.

**P2 (`new_api::new_issue` can't express structured bodies) — withdrawn as an oversight.** The task contract explicitly requires the core new API, intake, recurrence, and foreign import descriptions to retain free-text semantics. All six `structured_body: false` sites are contract-conforming, not missed. Residual gap: `NewIssueRequest.description` (`new_api.rs`) carries no doc comment stating this is deliberate, unlike `NewArgs.structured_body` which does. One sentence there prevents the next reviewer from re-filing my mistaken finding.

**P3 (fence-awareness of stub matching) — half withdrawn.** `all_h2_sections` routes through `scan_outside_fences` (`body_sections.rs`), so an H2 inside a fenced code block in the user's body neither suppresses a required stub nor counts as present, and `rest.trim_end()` handles trailing-whitespace heading variants. The code-fence and trailing-space halves of my concern are already handled correctly. **What survives, downgraded to edge case:** heading matching is exact-case (`## description` or `## Description:` in a body file → a near-duplicate `## Description` stub appended). Exact-case is consistent with the tool's heading conventions everywhere else (`LEGACY_SECTION_ALIASES`, `extract_section_text`), so this is a documented-behavior edge, not a defect — but it is untested, and it is precisely the duplicate-section symptom class this patch fixes.

**Empty-input concern — withdrawn.** `read_body_file_arg` (`cmd/write.rs`) rejects empty/whitespace-only input and trims only trailing whitespace, with a doc comment explaining the leading-whitespace preservation. The headingless-empty-body scenario cannot occur.

**Import round-trip double-wrap — confirmed out of scope by decision.** Independently reproduced and filed separately; distinguishing aliased `body` from foreign `description` needs its own design. `ImportRecord::into_new_args` setting `structured_body: false` is correct under the stated contract.

**Doc four-way duplication — withdrawn.** `skill::tests::dogfooded_copies_match_templates` enforces sync; the diff updates all four copies identically.

## Remaining findings (all minor, none blocking)

1. **Test gap — the `item_text::split` fix on the free-text path** (`new_issue.rs:351`). The old `split("---\n\n").nth(1)` bug affected inline `--description` bodies containing an HR too; the fix changes that path's stub behavior, but the only regression test uses `structured_body = true`. Add a `--description "a\n\n---\n\nb"` + schema case.
2. **Test gap — case-variant heading vs. schema stub.** Pin that `## description` in a structured body yields an appended `## Description` stub (or decide it shouldn't and normalize). Either way, lock the behavior.
3. **`run_with_stdin` in `cli_new.rs` panics on broken pipe** if the child exits before consuming stdin. Fine for the current happy-path tests; ignore `ErrorKind::BrokenPipe` or comment the constraint before someone reuses it for an error-path test.
4. **No advisory for degenerate structured bodies.** A body file with zero H2 headings, or containing its own H1, is accepted silently. The CLI already warns on `## Notes` via `reserved_section_warnings`; a "body-file contains no H2 sections" warning would fit the same channel. Enhancement, not defect — the H1 case is even structurally safe, since `title_heading` takes the first H1 and the generated title precedes any user H1.
5. **`NewIssueRequest.description` doc comment** stating free-text semantics are deliberate (see above).

## In-scope defect verdict

After re-verification against the actual implementations: **no in-scope defect remains.** The wrapper suppression is correct and minimal; all call sites conform to the stated per-path contracts; `all_h2_sections` is fence-aware and whitespace-tolerant; empty input is rejected; the `item_text::split` substitution genuinely fixes the HR-truncation bug and the splitter's own-line/fence rules make it safe against structured bodies containing HRs, fenced `---`, or embedded pseudo-frontmatter (the real frontmatter closes first). No blocker. The deliverable gaps are two regression tests, one doc comment, one changelog clause, and one test-helper hardening.

## Unresolved gaps

None material. The case-sensitive stub matching is the only behavior I'd flag as potentially surprising to users, and whether to normalize it is a product decision, not a correctness question — the current exact-match behavior is internally consistent.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_a953dee4a7ee49ae9ca03fb7ffaf916e]

## Revised verdict

The supplied context resolves my original three highest-priority concerns. I no longer see a release blocker in this patch after adversarial review.

Specific reversals:

- **Public API compatibility concern — withdrawn.** `crates/issuectl-core/src/lib.rs` explicitly states the crate has no stable public API and `pub` exists only for the sibling binary; the binary CLI surface is the semver contract. Adding `structured_body` to `NewArgs`/`NewIssueArgs` is therefore not an API break. `new_api.rs` deserializes `NewIssueRequest`, not `NewArgs`, so no serde-default issue arises.
- **Parser/fence concern — resolved.** `crates/issuectl-core/src/item_text.rs` is fence-aware, handles BOM/CRLF/missing frontmatter, and returns the body after the true frontmatter closing delimiter. Replacing `render.split("---\n\n")` with `item_text::split` is a correctness improvement, not a risk.
- **Empty-input concern — resolved.** `read_body_file_arg` rejects empty/whitespace-only file/stdin and trims only trailing whitespace, consistent with the `body set --from-file` convention.
- **Non-create constructors forced `structured_body: false` — resolved as intended.** The task contract explicitly keeps intake, recurrence, `new_api`, and foreign import free-text. The own-export/import structured-body defect is separately filed and tracked.
- **`--body-file` semantic change — no longer a blocker.** The pre-existing documented contract was “below the generated H1”; the implementation’s generated `## Description` wrapper contradicted that for complete structured bodies. This patch realigns the implementation with the documented structured-body contract. The `### Fixed` classification is defensible.

---

## Concrete problems, ordered by severity

### 1. Dogfooded skill/prompt templates still instruct `## Notes` for epics, contradicting the CLI and body-section code
This existed before the diff, but the diff rewrote exactly these sentences and left the wrong heading in place.

Affected files, all edited by this patch:

- `.claude/skills/issue/SKILL.md`
- `.codex/prompts/issue.md`
- `crates/issuectl-core/templates/issue-skill.md`
- `crates/issuectl-core/templates/issue-prompt.md`

Text around line 596–603:

> For epics, ensure `## Goal`, `## Issues`, `## Phases`, and `## Notes` sections exist.

But `crates/issuectl/src/cmd/mod.rs` help for `--body-file` says:

> A body using a reserved legacy section heading (`## Notes` — use `## Comments`) is accepted but warns; `issuectl doctor --fix` migrates it later.

And `crates/issuectl-core/src/body_sections.rs` confirms:

```rust
pub const LEGACY_SECTION_ALIASES: &[(&str, &str)] = &[("Notes", COMMENTS)];
```

`reserved_section_warnings` flags any raw description containing `## Notes`, and `do_new_locked` surfaces those warnings. An agent following the shipped template will produce an issue that immediately warns on create and is later migrated by `doctor --fix`.

Since AGENTS.md says the skill is “the only contract those agents see,” this is a real production-facing inconsistency, even though it is pre-existing. The `CommonMark` template text should say `## Comments` for epics, or the body-section code should exempt `Notes` for epic types. The latter would be a much larger change and is not warranted.

### 2. No CLI-level integration test for the exact regression composition
`crates/issuectl-core/src/mutate/new_issue.rs::structured_body_with_horizontal_rule_does_not_duplicate_required_sections` directly tests the `item_text::split` plus required-stub interaction.

But the black-box CLI tests in `crates/issuectl/tests/cli_new.rs` only cover:

- file with structured body and no schema,
- stdin with source and no schema,
- inline description.

There is no test that:

1. writes `issues/.schema.yaml` requiring `Description`/`Expected`/`Quick Test`,
2. invokes the binary with `create --body-file <file>`,
3. puts a horizontal rule in the body before a required H2.

The core test covers the intended fix, but AGENTS.md reserves `tests/` integration tests for black-box behavior, and the original bug was precisely at this wire-up boundary. This is a test gap, not a runtime bug.

### 3. `run_with_stdin` duplicates `run` and can deadlock for larger stdin/output
`crates/issuectl/tests/cli_new.rs`:

```rust
fn run_with_stdin(root: &std::path::Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_issuectl"))
        ...
        .spawn()
        .expect("spawn issuectl");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for issuectl")
}
```

It duplicates all of `run`’s env/command setup rather than sharing a builder. More importantly, it writes all stdin synchronously before `wait_with_output`, so if a future test supplies a body large enough to fill the child’s stdout/stderr pipe buffers, this deadlocks: the child blocks writing output while the parent blocks writing stdin. Current bodies are tiny, so it is test-only and low risk.

---

## Questionable decisions and missing edge cases

### `structured_body` is a process-local bool
The distinction between free text and structured Markdown is not represented in any persisted model. The supplied context accepts this for now and has the own-export/import consequence already filed. The remaining weakness is purely design scope: every future creation path must remember to set `structured_body: false` or the duplicate-wrapper bug will reappear. An enum named `BodyKind` would make the contract clearer, but this is not a defect given current call sites.

### No warning for `--body-file` with an unstructured body
With `structured_body = true` and a plain-text file in a repo whose schema requires `Description`, the output is:

```markdown
# Title

Plain prose.

## Description
```

The prose sits outside `## Description` and the required stub is appended after it. This is now the documented contract, but the previous behavior placed prose under the wrapper. A one-line validation warning could flag a body file with no H2 sections, but the supplied context says no content-sniffing mode is intended, so I treat this as an accepted edge.

### Changelog wording is defensible but subtle
`CHANGELOG.md` classifies the change as `### Fixed`. Given the previous help text said the body was written below the H1, this is defensible. A user who supplied plain-text `--body-file` previously got a `## Description` wrapper and will no longer get one, so the effect is broader than the “duplicate heading” wording suggests. If reviewers want zero ambiguity for already-released `--body-file` users, moving it to `### Changed` would be clearer. This is a documentation judgment, not a correctness problem.

---

## Context-dependent or disputed concerns

- **`## Notes` epic-template inconsistency:** pre-existing, but the diff touched the sentence and should have fixed it. If there is an unstated convention that epics intentionally use `## Notes`, the code has no such exception — so the doc/code conflict is real on the current tree.
- **`Fixed` vs `Changed` changelog entry:** resolved as acceptable under the old “below H1” documentation, but is still arguable for unstructured body files.
- **`run_with_stdin` deadlock risk:** hypothetical under current test inputs; worth refactoring opportunistically but not a production issue.
- **No structured-body flag on `new_api`/intake/recurrence/import:** intentionally rejected by the task contract; separate issue exists for own-export/import.

---

## What is solid

- `write::render_new_item_from_fm` suppresses the generated wrapper only when `structured_body` is true, preserving inline `--description` free-text semantics.
- Propagation to every constructor is correct: `intake.rs`, `new_api.rs`, `recurrence.rs`, `transfer.rs`, `cli_tests.rs`, and `runtime.rs` are all updated.
- `structured_body` is derived only from `--body-file`, which is exactly the intended boundary.
- The required-section splitting fix using `crate::item_text::split(&render).body` is correct and improves on the original naive split.
- The new unit test for horizontal rules and required sections is a direct regression test for the original failure.
- CLI tests cover file, stdin, `--source`, and inline-description behavior.
- Help text, shipped templates, dogfooded copies, CLI help, and changelog were updated coherently, aside from the `## Notes` inconsistency.

---

## Top 3 priorities

1. **Fix the `## Notes` → `## Comments` contradiction in the four skill/prompt template copies** so the shipped contract stops telling agents to create a section that warns and is later migrated.
2. **Add a `tests/cli_new.rs` integration test combining `.schema.yaml` required sections, `--body-file`, and a horizontal rule before a required H2.**
3. **Refactor `run_with_stdin` to share the `run` command builder and avoid the stdin/stdout pipe deadlock pattern** before any larger body tests are added.

## Fresh adopted-tree supplement — cross-review round 1 (DeepSeek provider failure disclosed)

[thread_id:group_3a3d37725ff2452f8459d1f5d0761ad3]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_aa2d00d86f0844c9889bb7bf05625a56]

### 1. Agreement with Valid Findings

I agree with Anthropic and DeepSeek regarding the latent deadlock risk in the `run_with_stdin` test helper.
```rust
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for issuectl")
```
Because `write_all` is synchronous and `stdout`/`stderr` are piped but not actively read during the write, writing an input larger than the OS pipe buffer (typically 64KB) will block. If the subprocess simultaneously fills its `stdout` or `stderr` pipe buffer (e.g., with debug logs or clap errors) before finishing its stdin read, both processes will deadlock. This is low-priority test debt since the current inputs are tiny strings and `issuectl` consumes stdin eagerly, but it is a real flaw in the test harness.

### 2. Disagreement with Out-of-Scope Concerns

I explicitly disagree with the previous review's assertions that the `structured_body: false` hardcoding in `transfer.rs` and `new_api.rs` represents an architectural flaw blocking this PR.
* Given the repository context that internal APIs are non-semver and non-create paths intentionally preserve free-text semantics for historical compatibility, isolating the new behavior to the CLI is correct. 
* Since own-export/import corruption is explicitly filed as a separate issue, conflating it with this targeted `create --body-file` fix is scope creep. Using a `structured_body: bool` field to strictly bound the behavior change to the requested CLI entry point is the safest path forward.

### 3. New Findings in the Latest Edits

The latest edits are fundamentally solid and introduce no new regressions:
* **Horizontal Rule Truncation Fix:** Swapping `.split("---\n\n").nth(1)` for the fence-aware `crate::item_text::split(&render).body` is an excellent fix. It guarantees that valid structural markers in the imported body (like thematic breaks) don't fool the schema enforcement into duplicating H2 sections.
* **Template Alignment:** Replacing `## Notes` with the canonical `## Comments` in `.claude/skills/issue/SKILL.md` and related prompts cleanly resolves the template contradiction DeepSeek identified.
* **Black-Box Testing:** The addition of `body_file_schema_appends_only_missing_sections` proves the entire rendering pipeline works end-to-end, confirming the schema missing-section stubs interact correctly with a structured body file.

### 4. Priorities

**No blocker.** 

The PR accurately fulfills the described contract shift, cleanly isolates the behavioral changes from legacy API paths, and tests the exact edge cases (horizontal rules, schemas, stdin) that matter.

**Minor Debt (Non-blocking):**
1. **Test-only Deadlock:** In `crates/issuectl/tests/cli_new.rs`, consider rewriting `run_with_stdin` to spawn a thread for `write_all` or use standard async I/O to avoid pipe buffer deadlocks if large test payloads are ever introduced.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_d4c88be0791f4e09a906f7c962b9385d]

## Current assessment

### Blocker: `NewArgs` constructor propagation appears to have regressed

The earlier diff included this required update:

- `crates/issuectl/src/cmd/cli_tests.rs`, around the `NewArgs` test fixture near line 133:
  ```rust
  structured_body: false,
  ```

The current attached diff no longer includes that hunk, while `crates/issuectl-core/src/mutate/new_issue.rs::NewArgs` still adds the non-defaulted field:

```rust
pub structured_body: bool,
```

If the `NewArgs` struct literal in `crates/issuectl/src/cmd/cli_tests.rs` still exists as previously shown, test compilation fails with a missing-field error. Rust does not apply `Default` to an explicit complete struct literal.

This is the only credible blocker visible in the current diff. Restore:

```rust
description: None,
structured_body: false,
custom_fields: vec![],
```

Then run the full workspace test build. If the constructor was independently removed or changed in the current tree, this finding does not apply, but the current diff gives no evidence of that.

## Valid findings from other reviewers

### Valid and now fixed: touched templates contradicted canonical section naming

DeepSeek’s `## Notes` finding was real. The current edits correctly change the epic advice to `## Comments` in all four touched generated/dogfooded copies:

- `.claude/skills/issue/SKILL.md`
- `.codex/prompts/issue.md`
- `crates/issuectl-core/templates/issue-skill.md`
- `crates/issuectl-core/templates/issue-prompt.md`

This also aligns with the existing reserved-alias warning documented in CLI help.

### Valid and now fixed: missing black-box schema interaction coverage

The new test in `crates/issuectl/tests/cli_new.rs::body_file_schema_appends_only_missing_sections` covers the complete production path:

- Built CLI parsing
- `--body-file` mode selection
- Horizontal rule preservation
- Existing required H2 recognition
- Missing-stub appending
- No duplicate `Description` or `Expected`

That closes the material coverage gap identified previously.

### Valid and now fixed: CLI help omitted schema-appended stubs

`crates/issuectl/src/cmd/mod.rs`, `PrimaryCommand::Create::body_file`, now explicitly says schema requirements may append missing H2 stubs. The help and agent templates no longer differ on that behavior.

### Valid but low priority: stdin test helper can deadlock if reused for large I/O

`crates/issuectl/tests/cli_new.rs::run_with_stdin` writes all stdin before draining piped stdout/stderr. A sufficiently noisy child that blocks writing output before reading all input could deadlock.

That is irrelevant to the current small successful fixture and is test-only debt. It is not a reason to block this change. Do not reuse the helper for 10 MiB limit tests without concurrent pipe handling.

## Incorrect or exaggerated findings

### Transfer and `new_api` parity allegations are wrong

These paths deliberately retain free-text semantics:

- `crates/issuectl-core/src/mutate/new_api.rs::new_issue`
- `crates/issuectl-core/src/transfer.rs::ImportRecord::into_new_args`
- `crates/issuectl-core/src/mutate/intake.rs::file`
- `crates/issuectl-core/src/recurrence.rs::materialize`

Their explicit:

```rust
structured_body: false,
```

is required compatibility behavior, not incomplete propagation. `create --body-file` is the only structured creation source in scope.

The own-export/import alias defect is separate and should not be smuggled into this patch.

### Prose-only body-file behavior is not a regression

A body file containing only paragraphs now remains only paragraphs beneath the generated H1. It does not receive `## Description`. That follows the explicitly selected structured-file contract and the historical documentation saying file content is written beneath the generated title.

Content sniffing—structured if it begins with H2, free text otherwise—would make behavior implicit and unstable. The current source-based distinction is better.

### Empty structured bodies are not reachable through the CLI

`crates/issuectl/src/cmd/write.rs::read_body_file_arg` rejects empty and whitespace-only files/stdin. Claims that `create --body-file` can silently produce only an H1 are incorrect.

The internal boolean representation can technically express `structured_body: true` with `description: None`, but that is a maintainability issue, not a CLI defect.

### Public API/SemVer concerns do not apply

`issuectl-core` explicitly disclaims a stable API in:

- `crates/issuectl-core/src/lib.rs`
- `crates/issuectl-core/Cargo.toml`
- `AGENTS.md`

Adding fields to `NewArgs` and `NewIssueArgs` is not a release-contract violation for this repository.

### Fence and horizontal-rule concerns are resolved correctly

`crate::item_text::split(&render).body` is safer than the removed string split. `item_text::split` identifies canonical frontmatter, and `body_sections::all_h2_sections` ignores headings inside fenced code.

The new core and black-box tests both exercise a body thematic break before an existing required H2. There is no demonstrated parser defect here.

### Additional free-text horizontal-rule and exact-case tests are optional

Inline descriptions still use the same body-section scanner after rendering. Exact case matching is the existing schema convention. More tests could document those rules, but they do not address a credible regression introduced by this patch.

## New findings from the latest edits

Apart from the apparently dropped `crates/issuectl/src/cmd/cli_tests.rs` constructor update, the latest changes introduce no new behavioral defect.

The added CLI+schema test is correctly scoped. Its substring counts are adequate for the controlled fixture, and the exact ordering assertion proves that the heading after the horizontal rule remains visible to schema detection.

The `## Notes` to `## Comments` template correction is consistent across bundled and dogfooded copies.

## Top 3 current issues

1. **Restore or verify the missing `structured_body: false` field in the `NewArgs` literal in `crates/issuectl/src/cmd/cli_tests.rs`.** As evidenced by the earlier diff, its omission will break test compilation.
2. **Run the complete workspace green gate, especially `cargo test --workspace`,** because constructor-field additions are compile-time propagation hazards and the current diff appears to have dropped one.
3. **Optionally harden `run_with_stdin` before using it for large-input tests.** This is low-priority test infrastructure debt, not part of the production fix.

Subject to resolving the missing constructor field, there is no behavioral blocker. The intended compatibility contracts leave no actual in-scope correctness defect in the implementation.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_ce1e9e959b8f40d9be397da57b30df09]

# Cross-review round 1 — current diff

## 1. Agreement with valid findings

- **DeepSeek's `## Notes` template contradiction was the one real find of the round, and the fix is correct.** The old epic advice told agents to add `## Notes` — a heading `LEGACY_SECTION_ALIASES` reserves and `reserved_section_warnings` flags at authoring time, so the diff's own new help text ("A body using a reserved legacy section heading (`## Notes` …) is accepted but warns") directly contradicted the skill's advice. All four copies now say `## Comments`, consistently.
- **OpenAI's two recommendations landed correctly.** `body_file_schema_appends_only_missing_sections` exercises the full binary → `structured_body=true` wiring → schema-stub path black-box, including the HR case, and the `cmd/mod.rs` help text now mentions schema stubs. Both are proper fixes, not cosmetic.
- **Bool-vs-enum as maintainability debt only** — agreed. `structured_body: true` + `description: None` remains constructible and renders a section-less body, but with `issuectl-core` explicitly non-semver this is refactor-later material.

## 2. Disagreements

- **DeepSeek's "stdin helper deadlock" is mischaracterized.** `run_with_stdin` (`crates/issuectl/tests/cli_new.rs`) cannot deadlock with these inputs: the payloads are far below pipe-buffer size, so `write_all` completes regardless of child behavior, and `wait_with_output` drains stdout/stderr. The actual risk is a **panic on `BrokenPipe`** if the helper is ever reused for a usage-error path where the child exits before reading stdin. Low test-only debt, correctly deprioritized, wrongly named.
- **Gemini's transfer/new_api "parity defects" were never defects.** The context established these free-text retentions are the task contract, and the own-export/import corruption is separately filed. Re-litigating them against this diff would be wrong; they should stay withdrawn.
- **Any residual "prose `--body-file` regression" framing is exaggerated.** The v0.7.0 documented contract said the file's markdown lands below the H1; the wrapper was the code contradicting the docs. The current behavior matches the documented contract.

## 3. New findings from the latest edits

- **Nothing broken was introduced.** The template edits are confined to the two touched hunks, applied identically across `issue-skill.md`, `issue-prompt.md`, and both dogfooded copies (which `dogfooded_copies_match_templates` enforces), and the frontmatter-strip relationship between skill and prompt variants is preserved.
- **Minor redundancy:** `body_file_schema_appends_only_missing_sections` substantially duplicates the inline core test `structured_body_with_horizontal_rule_does_not_duplicate_required_sections` (same schema, same HR body, same assertions). Its black-box value is the CLI→core `structured_body` wiring plus schema interaction, which the inline test can't observe, so it clears the AGENTS.md `tests/` bar — but a leaner version could have asserted only the wiring-visible delta. Not actionable.
- **Marginal advice quality:** telling epic authors to "ensure `## Comments` exists" is slightly off — `note` creates `## Comments` on demand via `append_block`, so pre-creating it is unnecessary (though harmless). Cosmetic.

## 4. Top 3 current issues — no blocker

1. **Still-missing regression test: free-text `--description` containing a `---` HR under a schema with required sections.** The `item_text::split` fix (`new_issue.rs:351`) changed stub behavior on the *historical free-text path* too (the old `split("---\n\n").nth(1)` truncated there as well), and every test added in both rounds pins only the structured path. This is the one silent behavior change shipped to pre-existing users with zero test coverage.
2. **Case-sensitive schema-stub matching remains untested and undocumented.** A structured body with `## description` or `## Description:` gets an appended `## Description` stub — a near-duplicate of the exact symptom class this patch fixes. Exact-match is internally consistent with the rest of the codebase, so I accept it as behavior, but pin it with a test either way.
3. **Two documentation crumbs:** the CHANGELOG entry still describes only the duplicate-heading symptom (a prose-only `--body-file` also loses its wrapper — one clause saves a future bisect), and `NewIssueRequest.description` in `new_api.rs` still lacks the "deliberately free-text" doc comment that `NewArgs.structured_body` has — the exact omission that caused two reviewers to file false parity findings this round.

All three are test/doc completeness items. The functional change is correct, contract-conforming, and adequately covered on its primary path. **No blocker.**

Residual uncertainty: none material. I have not seen the free-text-with-HR schema behavior exercised anywhere (item 1), so my claim that the split fix is *correct* on that path rests on reading `item_text::split` rather than on a test — which is precisely why the test should exist.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro]

**Error:** API request failed with status 402 Payment Required: {"error":{"message":"Insufficient Balance","type":"unknown_error","param":null,"code":"invalid_request_error"}}

### DeepSeek finite retry result

The retry on the same DeepSeek thread failed with:

```text
API request failed with status 402 Payment Required: Insufficient Balance
```

No second supplementary cross-review round was attempted because the finite recovery bound was exhausted. The complete authoritative preserved review above already contains DeepSeek's recovered independent response and both required complete four-model cross-review rounds.
