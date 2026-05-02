---
created: 2026-04-25
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: testing
priority: normal
epic: 56
labels: [web]
---

# 11. Verkkosivu tositteiden katseluun

_Source: tositteiden haku_

## Description

Yksinkertainen verkkosivu jossa käyttäjä voi kirjautua sisään ja katsella omia tositteitaan — listaus, haku ja yksittäisen tositteen tiedot.

## Scope

- [x] Kirjautuminen (sama käyttäjätili kuin rekisteröitymisessä) — toteutettu #26:ssa
- [x] Tositelistaus (aika, summa, kategoria) — `GET /receipts`, paginointi 50/sivu
- [x] Tositteiden haku ja suodatus — vapaa hakukenttä (vendor + raw_text ILIKE) + date-range
- [x] Yksittäisen tositteen näkymä (kuva + jäsennetyt tiedot) — `GET /receipts/:id` ja inline image `GET /receipts/:id/attachments/:aid`

## Implementation notes (worktree `a8-receipt-list-page`)

- **Ops** (`crates/ops/src/receipts/view.rs`): `list_receipts_page`
  (rows + total via `COUNT(*) OVER ()` — single trip, can't drift),
  `get_receipt`, `list_receipt_attachments`, `load_receipt_attachment`
  → `AttachmentLoad` (`Found` / `TooLarge` / `NotFoundOrNotOwned`).
  `list_receipts` laajeni `query`/`offset`-kentillä; shared
  `ilike_contains_pattern`-helper.
- **Routes** (`crates/server/src/http/routes/receipts.rs`): kolme
  routea `require_session`-middlewaren takana.
- **Security defenses (post-LLM-review):**
  - Inline-attachment whitelist (`image/jpeg|png|webp|gif`); muut
    pakotettu `application/octet-stream` + `attachment`-disposition
    + `X-Content-Type-Options: nosniff`. Detail-sivun `<img>`
    käyttää samaa whitelistia (ei SVG:tä `<img>`:iin).
  - `Cache-Control: private, no-store` attachment-vasteille (ei
    diskcache-vuotoa logoutin jälkeen).
  - RFC 5987 `filename*=UTF-8''…` ei-ASCII-tiedostonimille,
    fallback ASCII-puhdistettu `filename=…` legacy-asiakkaille.
  - `rel="noopener noreferrer"` `target="_blank"`-linkeille.
  - 25MB hard cap attachment-tavuihin (413 omistajalle, 404
    luvattomalle pyynnölle — erottelu säilyttää
    indistinguishability-takuun).
  - Page-overflow + query-pituus-cap (`page` ≤ 1_000_000,
    `q` ≤ 200 char, `saturating_sub`/`saturating_mul`).
- **Authz**: kaikki kyselyt scope-rajattuja `(session.tenant_id,
  session.user_id)`-paritukseen. Cross-tenant **ja** same-tenant-
  different-user testattu eksplisiittisesti.
- **Testit**: 18 sqlx-integraatiotestiä (cross-tenant + same-
  tenant-different-user `get_receipt`/`list_receipts_page`/
  `load_receipt_attachment`, extraction-id link-path,
  message_id-link-path, unrelated-attachment block,
  `COUNT(*) OVER ()` total-konsistenssi, query-substring,
  `%`/`_` literal-escape, offset-paginointi) + 12 unit-testiä
  reittien helper-funktioille (urlencode, format_amount,
  ceil_div, truncate_chars, content_disposition_header
  ASCII / control-strip / RFC 5987 / non-ASCII fallback,
  safe-inline-image-whitelist).

## Verification

Käyttäjän verifoitavissa lokaalisti:
1. `gsdev instance ensure` (Postgres + Mailpit)
2. `cargo run -p grooveserve-server`
3. Login + selaa `/receipts`
4. `cargo test --workspace` — 570 testiä, 0 failures.

## Spin-off issues filed from `/llm-review`

- **#88** — receipt ↔ attachment-link tightening (drop message_id
  branch tai junction-taulu). Doc-comment vs. SQL-semantiikka-drift
  kirjattu rehellisesti tämän PR:n koodikommentteihin, mutta
  rakenteellinen korjaus on oma issuensa.
- **#89** — receipts page scaling (object storage, trigram/tsvector
  index, keyset pagination, locale-aware currency formatting).
  CLAUDE.md:n MVP-periaatteen mukaisesti deferattu.
