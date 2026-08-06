---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: in-progress
priority: normal
---

# show --json omits blocked_by (and derived blocks)

## Description

The \`--json show\` output does not include the \`blocked_by\` field at all — the key is absent from the object, not just \`null\`. Dependencies written via \`issuectl depend add --blocked-by\` land correctly in the item.md frontmatter, but are invisible to any agent that reads issue state through \`--json show\`.

## Reproduce
\`\`\`
issuectl depend add markdown-template-render --blocked-by prose-theme   # writes frontmatter OK
# item.md frontmatter now shows:  blocked_by: [\x27@prose-theme\x27]
issuectl --json show markdown-template-render | jq keys
# -> the array has NO \"blocked_by\" (nor \"blocks\") key
issuectl --json show markdown-template-render | jq .blocked_by
# -> null
\`\`\`

## Observed vs expected
- Observed: \`show --json\` \`keys\` = [assignee, body, closed, commits, created, epic, extra, folder, labels, owner, priority, related, reporter, slug, status, title, type, updated, version] — no \`blocked_by\`, no \`blocks\`.
- Expected: \`blocked_by\` present (e.g. \`[\"@prose-theme\"]\`), and ideally the derived read-time \`blocks\` view too, so JSON consumers see the same dependency graph the frontmatter and \`depend\` subcommand manage.

## Impact
Breaks programmatic DAG / head-of-line computation: the stint execution-DAG head-of-line eligibility rule reads \`blocked_by\` live via \`--json\`. With the field absent, an agent cannot tell a blocked issue from a ready one through the JSON API, and must fall back to grepping item.md frontmatter — defeating the point of \`--json\`.

Discovered while orchestrating a glasspad 0.3.0 round (2026-08-06).
