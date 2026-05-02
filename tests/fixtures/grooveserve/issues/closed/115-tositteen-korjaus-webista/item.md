---
created: 2026-05-01
updated: 2026-05-02
closed: 2026-05-02
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#11", "#38", "#57", "#114"]
labels: [web, receipts, user-facing, edit]
commits:
  - hash: 00ce92a
    summary: "feat(receipts): add edit form, revision history, and restore (#115)"
  - hash: 614f3b5
    summary: "refactor(receipts): tri-state Patch<T> replace Option<T> for update semantics"
  - hash: 3fbfac0
    summary: "fix(receipts): apply review fixes — clear fields, is_no_op guard, full restore"
spin_offs: ["#117", "#118", "#119"]
---

# 115. Tositteen korjaus webistä — edit-näkymä + revision-historia

_Source: #56 Phase 3 UUSI-issuena_

## Description

Käyttäjä näkee tositteen detail-sivun (#11) mutta ei voi korjata virhettä
webistä — esim. agentti kirjasi vendor-nimen väärin tai päivämäärä on
yhtä päivää sivussa. Sähköpostipohjainen agent-flow toimii (käyttäjä voi
vastata "kuitin summa oli 12,50 €, ei 1,25 €") mutta web-edit on monelle
käyttäjälle nopeampi reitti.

Receipt-revision-historia (#38, C2-worktree) on jo olemassa
`receipt_revisions`-tauluna, ja `update_receipt`-ops-funktio kantaa
revision-snapshot:in. Tarvitsemme web-pinnan jonka kautta käyttäjä voi:

1. Avata tositteen muokkauksen
2. Muuttaa kenttiä (vendor, date, total, currency-block, category,
   payment_method, vat, raw_text editing? ehkä ei MVP:ssä)
3. Tallentaa → revision-rivi syntyy, tosite päivittyy
4. Nähdä revision-historian ja palauttaa aiempi versio jos halutaan
   (undo) — kytkeytyy #57:n expert-undo-pintaan, mutta käyttäjä saa
   undon vain omille riveilleen

## Scope

- [ ] **`GET /me/receipts/:id/edit`** — muokkauslomake, esitäyttö
  current-arvoilla
- [ ] **`POST /me/receipts/:id/edit`** — tallennus, kutsuu
  `ops::receipts::update_receipt`-funktiota, revision-snapshot kirjautuu
  automaattisesti (ks. #38)
- [ ] **`GET /me/receipts/:id/history`** — revision-historian listaus,
  diff per rivi (vendor: "X" → "Y", date: "..." → "...")
- [ ] **`POST /me/receipts/:id/restore/:revision_id`** — palauttaa
  aiemman version uutena revisionina (ei poista, vaan kirjoittaa uuden
  rivin joka kopioi snapshot-tilan)
- [ ] **Audit-rivi** `audit_events`-tauluun jokaisesta web-muokkauksesta
  ja restore:sta (channel = `Channel::Web`). Linkki revision-id:hen.
- [ ] **i18n** en/fi/sv
- [ ] **Auth-rajaus**: `(tenant_id, user_id)`-skooppinen, vain oma kuitti.
  #86 (entinen #67) v1.1 lukitsee tämän pinnan `own (write)` -varianttiin
  `user`-roolille.

## Out of scope

- Tositteen kuvan korvaaminen / uudelleenajo OCR:n läpi — voi olla
  tulevaisuudessa "uudelleenajo agentin kautta" (#57 Phase 4 expert-
  käyttö), ei MVP-suunnitelmassa.
- Kategorian muokkaus voidaan rajata lukittuihin valuesin enuman tasolla.
- Agent-tasolle palaava korjauspyyntö ("agentti, korjaa tämä") — eri
  flow, kuuluu #57 expert-pintaan.

## Files to Examine

- `crates/ops/src/receipts/update.rs` — olemassa oleva `update_receipt`
- `crates/ops/src/receipts/revision.rs` — revision-historian writer
  (#38)
- `crates/ops/src/receipts/view.rs` — `get_receipt` reader
- `crates/server/src/http/routes/receipts.rs` — existing `/receipts/:id`
  reader-route, laajennetaan /edit + /history + /restore -reiteillä
- `issues/open/56-toimiva-testattava-perusta/item.md` Phase 3
- `issues/closed/38-receipt-revision-history/` — revision-schema
- `crates/ops/AGENTS.md`

## Acceptance criteria

- Edit-, history-, ja restore-reitit toimivat. CSRF-suojaus kuten muut
  POST-reitit.
- `update_receipt`-call tuottaa revision-snapshot:in (kuten ennenkin —
  ei muutoksia ops-puolelle, vain web-route).
- Restore tuottaa **uuden** revisionin (ei poista historiaa).
- 8–10 sqlx-/integration-testiä: happy path edit, cross-tenant reject,
  cross-user reject, restore, audit-rivin sisältö, revision-diff
  oikeellisuus.
- i18n-stringit en/fi/sv.
- `cargo test --workspace --tests` puhdas.
- AGENTS.md päivitetty.

## Notes

Kytkeytyy #114 tapahtumalokiin: tapahtumalokin "Pyydä korjausta" -nappi
linkittää tähän edit-sivuun. Linkki kannattaa tehdä #114-worktreessa
mutta itse edit-sivu rakennetaan tässä issuessa. Worktreet voivat
mennä rinnakkain — eri reitit, eri ops-funktiot.
