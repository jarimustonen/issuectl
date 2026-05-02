---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#11", "#88"]
labels: [schema, evidence-model]
---

# 113. Receipt 1:N attachment via junction table

_Source: `/llm-review` follow-up to #88 option A — moved here when the
team chose "drop the message_id branch now, junction later"._

## Description

Today (post-#88-A) the receipt ↔ attachment link is **strict 1:1**
through `receipts.extraction_id → extractions.attachment_id`. The
read-side query and the schema's existing `extractions.UNIQUE
(attachment_id)` together pin every receipt to at most one source
attachment.

The product's actual evidence model is:

```
expense (laskurivi, 1) ←→ (N) receipt (kuitti, 1) ←→ (N) attachment (kuva)
```

Where one expense row may be backed by multiple receipts, and one
receipt may be backed by multiple supporting images:

- Front + back of a paper receipt.
- Multi-page PDF where the inbound mail provided each page as a
  separate file.
- Receipt photo + supporting document (boarding pass, expense-policy
  proof, hotel folio).
- Receipt assembled from email body text where the attached image is
  a supporting context (parking ticket scan, invoice page).
- User-uploaded images attached to an existing receipt after the
  initial OCR pass.

None of these shapes can be expressed under the current 1:1 link.

## Triggers (one of these starts the work)

1. **First non-extraction receipt-creation path enters design** — web
   upload, manual entry from `/me`, support-tool ingestion, calendar-
   based receipt drafting.
2. **Tripwire fires in production** — `save_receipt` already emits
   `tracing::warn!` with `event = "email_receipt_without_extraction"`
   when `source = 'email' && extraction_id IS NULL`. A non-trivial
   warn rate (e.g. > 1% of inbound receipts over a week) is the
   "this assumption broke in the wild" signal.
3. **Multi-attachment use case surfaces from product** — a real
   user/customer asks for "show me both receipt photos" or "attach
   the boarding pass to this hotel receipt".

Until one of those triggers fires, the strict 1:1 read-side stays.

## Schema sketch (from #88's draft, confirmed by user data model)

```sql
CREATE TABLE receipt_attachments (
    receipt_id    BIGINT NOT NULL,
    attachment_id BIGINT NOT NULL,
    tenant_id     BIGINT NOT NULL,
    user_id       BIGINT NOT NULL,
    -- Optional role tag: 'source' (the OCR'd image), 'supporting',
    -- 'user_uploaded'. Lets the UI distinguish the primary evidence
    -- from add-ons without baking the order into row position.
    role          TEXT NOT NULL DEFAULT 'source',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (receipt_id, attachment_id),
    FOREIGN KEY (tenant_id, user_id)
        REFERENCES tenant_users (tenant_id, user_id),
    FOREIGN KEY (receipt_id, tenant_id, user_id)
        REFERENCES receipts (id, tenant_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (attachment_id, tenant_id, user_id)
        REFERENCES attachments (id, tenant_id, user_id) ON DELETE CASCADE,
    CHECK (role IN ('source', 'supporting', 'user_uploaded'))
);

CREATE INDEX idx_receipt_attachments_receipt
    ON receipt_attachments (receipt_id);
CREATE INDEX idx_receipt_attachments_attachment
    ON receipt_attachments (attachment_id);
```

Composite-FK chain on `(*, tenant_id, user_id)` keeps the same-owner
invariant the current 1:1 design enjoys.

## Acceptance criteria

- Migration creates `receipt_attachments` with the above shape.
- Backfill: for every existing receipt with non-NULL
  `extraction_id`, insert one `(receipt_id, extraction.attachment_id,
  ..., role = 'source')` row. After backfill, the read side can
  switch to the junction without losing any current attachment.
- `crates/server/src/ingest/extraction.rs` and / or `crates/ops/src/
  receipts/save.rs` writes a `receipt_attachments` row inside the
  same transaction as the `receipts` INSERT (when the extraction
  came from the email pipeline).
- `crates/ops/src/receipts/view.rs::list_receipt_attachments` and
  `load_receipt_attachment` read through the junction (`JOIN
  receipt_attachments` instead of `JOIN extractions`). The strict
  1:1 SQL is replaced.
- Tests:
  - Multi-attachment receipt: two rows in junction → list returns 2.
  - Same attachment, two receipts (multi-receipt PDF): both list it.
  - Cross-tenant / cross-user: composite-FK rejects the row at
    write time; read-side returns empty for the wrong owner.
  - `ON DELETE CASCADE` on receipts and attachments removes the
    junction rows.
- `tracing::warn!` tripwire in `save_receipt` is removed (the
  assumption it tracked is no longer load-bearing once the read side
  goes through the junction).
- Decision recorded in #56 decision log.

## Out of scope (file separately if needed)

- A standalone `/attachments/:id` route surface — the junction is
  the data shape, the URL design is a separate UI question.
- Forensic / admin "all attachments in this inbound email" view —
  different access pattern; design when support flow needs it.
- `extraction.id` vs. `attachment.id` as the canonical evidence key —
  this issue keeps `attachment.id` since that's what the bytes route
  serves.
