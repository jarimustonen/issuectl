---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#11"]
labels: [schema, correctness]
---

## Resolution (2026-05-01)

**Option A applied.** Verified against `grooveserve_email_main_main`
(only DB with production-shape data): `SELECT COUNT(*) FROM receipts
WHERE extraction_id IS NULL` = 0.

**Blast-radius diff query** (per `/llm-review` follow-up — verifies
that no attachment becomes globally invisible after the
`message_id`-branch removal):

```sql
SELECT a.id
FROM attachments a
WHERE NOT EXISTS (SELECT 1 FROM extractions e WHERE e.attachment_id = a.id)
  AND EXISTS (
    SELECT 1 FROM receipts r
    WHERE r.tenant_id = a.tenant_id
      AND r.user_id = a.user_id
      AND r.message_id = a.message_id
  );
```

Result: **0 rows.** Every attachment has its own extraction → its own
receipt, so per-receipt isolation does not orphan any image. The
within-email cross-link query (`r.message_id = a.message_id` AND no
extraction-id link) returns 6 rows for one inbound email with 3
attachments and 3 receipts, all of which resolve to the *intended*
per-receipt scoping post-fix.

The `message_id` fallback branch was removed from
`crates/ops/src/receipts/view.rs::list_receipt_attachments` and
`load_receipt_attachment`. Both now go strictly through
`receipts.extraction_id → extractions.attachment_id`, with the
composite-FK chain enforcing same-owner. Tests pivoted:
`list_receipt_attachments_finds_by_message_id` →
`list_receipt_attachments_ignores_shared_message_id` (raw INSERT +
explicit fixture invariant assertion), and a new
`list_receipt_attachments_separates_per_receipt_in_same_email` pins
the multi-receipt-per-email shape with `r.message_id` populated so
the regression really would have fired against the pre-fix SQL.
`load_receipt_attachment_blocks_unrelated_attachment` updated to use
the *same* shared message_id (the dangerous case). End-to-end
`save_receipt_with_extraction_id_resolves_attachment` test added.

**Tripwire**: `save_receipt` emits `tracing::warn!` when
`source = 'email' && extraction_id IS NULL` (PoC tolerates missing
extractions for unsupported MIMEs / OCR failures, but we want them
observable). No schema CHECK — migration 013 explicitly admits null
`extraction_id` for future web/manual entry.

**Junction table (option B)** moved to spin-off **#113** (Receipt 1:N
attachment via dedicated junction table), triggered by:
- (a) first non-extraction receipt-creation path entering design,
- (b) tripwire warns landing in production logs, or
- (c) a real use case for multiple attachments per receipt
  (front/back photos, supporting documents, multi-page evidence).

# 88. Tighten receipt ↔ attachment link: drop message_id branch or add junction table

_Source: `/llm-review` of #11 worktree `a8-receipt-list-page`_

## Description

`crates/ops/src/receipts/view.rs::list_receipt_attachments` and
`load_receipt_attachment` link receipts to attachments via two SQL
branches:

1. `receipts.extraction_id` → `extractions.attachment_id` (1:1, FK
   enforces same-owner — strict, correct).
2. `receipts.message_id` = `attachments.message_id` (per-email
   grouping — over-broad).

Branch 2 is **per-email grouping, not per-receipt**. If one inbound
email yields receipts R1 + R2 and attachments A1 + A2:

- R1's detail page renders both A1 and A2.
- The auth check accepts `/receipts/R1/attachments/A2` despite A2 not
  being the source of R1.

This is **not a cross-user leak** (still tenant + user scoped) but
it's a correctness / authorization design weakness. Three of four
LLM reviewers flagged it; OpenAI in particular argued for an
explicit junction table.

The doc-comment in the worktree's first round said "1:N — one
receipt per attachment". The SQL implements M:N within a `message_id`.
That comment was rewritten in the round-2 fix-up to be honest about
the relation, but the semantics gap remains.

## Options

**A) Drop the message_id branch entirely.**
Cheap if every production receipt has `extraction_id` set. Verify by
querying production / dev DB:
```sql
SELECT COUNT(*) FROM receipts WHERE extraction_id IS NULL;
```
If `0` (or only legacy / hand-entered / web-entered rows), drop the
branch and the doc-comment becomes accurate again.

**B) Add a `receipt_attachments` junction table.**
```sql
CREATE TABLE receipt_attachments (
    receipt_id bigint NOT NULL REFERENCES receipts(id) ON DELETE CASCADE,
    attachment_id bigint NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    tenant_id bigint NOT NULL,
    user_id bigint NOT NULL,
    PRIMARY KEY (receipt_id, attachment_id),
    FOREIGN KEY (tenant_id, user_id) REFERENCES tenant_users (tenant_id, user_id),
    FOREIGN KEY (attachment_id, tenant_id, user_id)
        REFERENCES attachments (id, tenant_id, user_id),
    FOREIGN KEY (receipt_id, tenant_id, user_id)
        REFERENCES receipts (id, tenant_id, user_id)
);
```
Then ingest writes one row per (receipt, source-attachment) pair.
Strict 1:N from receipt's perspective, multiple attachments allowed
per receipt without ambiguity.

## Recommendation

Start with (A) — verify the assumption first. If web-entered or
hand-typed receipts (`extraction_id IS NULL`) are common, they need
some way to attach images, and (B) becomes necessary.

## Acceptance criteria

- Either: `message_id`-branch removed and route doc-comment matches
  reality (1:1 via extraction_id), OR
- A `receipt_attachments` junction table lands with migration +
  ops-layer writes from the ingest pipeline + tests.
- `list_receipt_attachments_finds_by_message_id`-test pivots to
  whichever path is canonical.
- Decision documented in #56 decision log.
