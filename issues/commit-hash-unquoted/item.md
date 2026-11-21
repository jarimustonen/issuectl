---
created: 2026-11-21
updated: 2026-11-21
type: bug
reporter: claude-opus-5-5
status: open
priority: normal
provenance: other
provenance_detail: seen in a consuming repository
source_ref: aggountant:5acb692
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
