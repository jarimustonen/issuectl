---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#11"]
labels: [perf, ux, post-mvp]
---

# 89. Receipts page scaling — object storage, search index, pagination, currency formatting

_Source: `/llm-review` of #11 worktree `a8-receipt-list-page`_

## Description

The `/receipts` MVP (issue #11) ships with deliberate optimisation
shortcuts. CLAUDE.md's "MVP-vaihe: toiminnallisuus ensin" principle
says these are out-of-scope for the first cut. This issue collects
them so the team can revisit when real volume arrives.

## Sub-items

### 89.1 Object storage for attachment bytes

`load_receipt_attachment` reads the entire `attachments.data`
`BYTEA` into memory and returns it via `Body::from(Vec<u8>)`. We've
capped this at 25MB per request (see #11 fix), but that still means
N concurrent requests can allocate `25 * N` MB. Postgres `BYTEA` is
not designed for unbounded binary serving.

Migration plan:
1. New `attachments.object_key` column (TEXT, nullable).
2. Ingest writes both `data` (legacy) and uploads to S3/R2/GCS
   under `tenant/<id>/attachment/<sha256>`.
3. `load_receipt_attachment` returns a signed URL or streams from
   storage.
4. After cutover: backfill `object_key` for existing rows, drop
   `data` column.

### 89.2 Search index for `raw_text` ILIKE

Today's free-text query does `vendor ILIKE %q% OR raw_text ILIKE %q%`
with leading wildcards. PostgreSQL btree can't help with leading-`%`
patterns, so this becomes a sequential scan as `receipts` grows.

Options:
- **`pg_trgm` GIN index** on `vendor` + `raw_text` for substring
  search.
- **`tsvector` generated column** + GIN index for full-text search.
  Better for `raw_text` (OCR output is large), worse for vendor
  substrings.
- Consider only enabling `raw_text` search behind an explicit
  toggle so the default page is fast.

### 89.3 Keyset pagination

`OFFSET 50 * page` becomes slow at large page numbers (Postgres has
to scan + skip). MVP cap is `MAX_PAGE = 1_000_000` which prevents
runaway, but late-page navigation still degrades.

Replace with keyset pagination on
`(COALESCE(receipt_date, created_at::date), id)`:
```sql
WHERE (COALESCE(receipt_date, created_at::date), id) <
      ($cursor_date, $cursor_id)
ORDER BY COALESCE(receipt_date, created_at::date) DESC, id DESC
LIMIT $limit
```

Requires:
- New expression index:
  `(tenant_id, user_id, COALESCE(receipt_date, created_at::date) DESC, id DESC)`.
- Cursor encoding in URL (base64 or two query params).
- Pagination UI redesign — no page numbers, just "next/previous".

### 89.4 Locale-aware currency formatting

`format_amount` produces `"12.34 EUR"` via `Decimal::Display`. Issues:
- `Decimal::Display` doesn't force 2 decimal places (`12.00` → `"12"`).
- Wrong decimal separator for non-US locales (Finnish/Swedish use `,`).
- No thousands grouping (`1234.56` should be `"1 234,56 €"` for fi-FI).

Either pull in `icu_decimal` / `unic-langid` or hand-roll
`format_eur_fi` / `format_usd_en` helpers.

## Decision criteria

Don't implement until measurement says it matters:
- 89.1: any individual receipt ≥10MB, or memory pressure visible
  in production.
- 89.2: list-page p95 latency >500ms for an active tenant.
- 89.3: same — late pages slow.
- 89.4: customer feedback or first non-FI/SV locale ships.

## Out of scope

- Full revision-history viewer on detail page (separate issue).
- Receipt-edit form on detail page (separate Phase 3 issue).
