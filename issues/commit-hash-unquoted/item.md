---
created: 2026-11-21
updated: 2026-11-21
type: bug
reporter: claude-opus-5-5
status: fixed
priority: normal
provenance: other
provenance_detail: seen in a consuming repository
source_ref: aggountant:5acb692
closed: 2026-11-21
closed_by: claude-opus-5-5
commits:
- hash: 29b25ae
  summary: 'fix(parser): read unquoted numeric commit hashes with a warning'
---

# Closing an issue writes all-digit commit hashes unquoted, and the item no longer parses

## Description

## Description

When an issue is closed, issuectl writes the related commits into the item's
frontmatter as `- hash: <short-hash>` without quotes. A short hash made only of
digits (for example `2206072` or `4527735`; about 3 % of 7-character hashes)
is then a YAML integer, not a string. issuectl itself cannot read the file
afterwards: `issuectl dag` warns `invalid YAML frontmatter in
./issues/<slug>/item.md: invalid type: integer `2206072`, expected a string`,
and the item drops out of the DAG until someone quotes the hash by hand.

Seen in a consuming repository on 2026-11-21 after `issuectl close … --stamp`
recorded three commits, two of them all-digit.

## Expected

Commit hashes (and any other string field whose value can look like a number,
a boolean or null) are always written quoted, so a round trip through
issuectl's own writer always parses. The reader may also accept an integer for
`hash` and turn it back into its string form, but note that a hash with
leading zeros loses them as an integer, so quoting on write is the real fix.
A regression test closes an issue with an all-digit hash and reads it back.

## Resolution

### 2026-11-21T10:23:32Z · @claude-opus-5-5

Root cause: issuectl's writer was not at fault. In the observed case (aggountant @migrate-from-chrono, 2026-11-21) the worker closed through issuectl, which wrote quoted hashes, then rebased and rewrote the hashes with a hand `sed -i 's/hash: c23cfb4/hash: 2206072/…'`, which dropped the quotes. serde_yaml already quotes every string that YAML 1.2 (issuectl's own reader) would resolve to a non-string. That covers all-digit and leading-zero hashes, float and hex lookalikes, booleans and null, for every field issuectl writes. New tests pin this for scalar fields, string lists, commit hashes and summaries, plus a close with all-digit hashes that is read back. YAML 1.1-only forms (yes/no/on/off, 12:30) stay unquoted; they read back as strings in issuectl and are not hash shapes. Reader: it now tolerates a numeric hash, read as its decimal text, and emits a warning. The reason: one bad hash used to default the whole typed frontmatter and drop the issue from the DAG, which is out of proportion, and the Mapping-level writer already coerced numeric hashes in the same way. The warning stays because the integer reading can lose leading zeros (0123456 becomes 123456). Mutations refuse items with parser warnings, so issuectl never persists a guessed hash; the user quotes the real one.
