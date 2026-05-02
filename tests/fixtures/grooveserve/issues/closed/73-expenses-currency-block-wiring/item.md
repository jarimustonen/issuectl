---
created: 2026-04-30
updated: 2026-05-01
closed: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#28", "#71"]
labels: [accounting]
commits:
  - hash: bcbbf48
    summary: "feat(ops/expenses): wire multi-currency block end-to-end (#73)"
---

# 73. Expenses currency-block -kytkentä (#28:n loppupala)

_Source: C3 (#28) post-implementation /llm-review_

## Description

C3 (#28) lisäsi migraatio 020:ssa neljä currency-saraketta (`original_currency`, `original_amount`, `exchange_rate`, `exchange_rate_date`) sekä `receipts`- että **`expenses`**-tauluun, mukaan lukien identtiset CHECK-rajoitteet. Receipts-puoli on täysin kytketty (input-structit, validointi, INSERT/UPDATE, revision-snapshot). **Expenses-puoli on schema-only** — `add_expense`/`update_expense` (ops + server-tools) eivät kanna kenttiä, joten kaikki uudet `expenses`-rivit saavat all-NULL currency-block:n riippumatta siitä mitä linkitetyssä kuitissa on.

Tämä on #28:n vajaaksi jäänyt osa, ei uusi feature. C3:n Worktree-loki merkitsi #28:n `done`-tilaan koska receipts-puoli kattaa MVP-tarpeen, mutta expenses-puoli pitää kytkeä ennen kuin matkalasku-aggregointi (Phase 2/3) voi näyttää sekavalyytta-kuluja oikein.

## Scope

### Ops-kerros (`crates/ops/src/expenses/` — luotava jos ei vielä ole)

Tällä hetkellä `expenses`-domain elää `crates/server/src/tools/expenses/`:ssa eikä `crates/ops`:ssa. Kytkentä vaatii joko:
- Päätös: pidetäänkö expenses-domain server-cratessa vai siirretäänkö se ops:iin (#56 Phase 5b suosittaa ops::receipts-pintaa, sama looginen polku expenses:lle).

Jos ops-siirto tehdään tämän yhteydessä:
- `AddExpenseInput` / `UpdateExpenseInput` -structit saavat 4 uutta `Option<...>`-kenttää
- `validate_currency_block`-helper jaetaan `ops::expenses`:n kanssa (uudelleenkäyttö currency.rs:stä)
- INSERT/UPDATE-lauseet kantavat kentät
- Revision-snapshot expenses:lle (vastaava `expense_revisions`-malli? Vai ei revision-historiaa expenses:lle?)

Jos expense pidetään server-puolella:
- Sama `validate_currency_block` re-exportataan ops:sta serveriin
- INSERT/UPDATE-lauseet `crates/server/src/tools/expenses/*.rs`:ssa kantavat kentät

### Server-tools

- `add_expense.rs` JSON-skeema saa 4 uutta valinnaista kenttää
- `update_expense.rs` sama
- `resolve_currency_block`-helper jaetaan kuittien kanssa (yhteinen util `crates/server/src/tools/util.rs`:ssa tai oma moduuli)
- Skill-markdownit päivitetään (worked example USD-kululle)

### Receipt → expense -propagointi

Kun `add_expense` luodaan kuitista (`receipt_id` annettu), kuitin currency-block kopioidaan oletuksena expense:lle. Agentti voi yliasettaa, mutta default tulee kuitista.

### Testit

- `add_expense` USD-input → rivi tallennettu currency-blockilla
- `update_expense` USD → SEK -muutos: schema-CHECK pidätelee inkoherentin tilan
- Receipt → expense -propagointi: kuitti USD, expense default USD:llä
- Schema-CHECK direct-INSERT-testit (atomic, EUR-coherence, ISO 4217)

### Dokumentaatio

- `crates/ops/AGENTS.md` (jos ops-siirto): expense-osio
- `crates/server/AGENTS.md`: expense-tool-pinta päivitetty

## Riippuvuudet ja vuorovaikutus

- **#71 (Kuitti vs. laskutustapahtuma + FX-kulut)** vaikuttaa myös tähän: jos `total_amount`:n semantiikkaa muutetaan (#71:ssä), `expenses.amount`:n semantiikka pitää muuttaa samanaikaisesti. Suositus: **odota #71:n päätöstä** ennen tämän työn aloitusta, tai koordinoi rinnakkain.
- **#56 Phase 5b** ehdottaa että receipts-toiminnot siirretään `ops::receipts`-kerrokseen Phase 1:n jälkeen — tämä työ voi ottaa expense-domain:n samalla.

## Out of scope

- Mileage / per-diem / meal-allowance -rivit eivät tarvitse currency-blockia (ovat per määritelmä EUR-määräisiä Verohallinnon korvausmäärinä). Currency-block jää NULL:ksi näille `expense_type`:eille.
- `expense_revisions`-historia (jos sellainen halutaan) — oma issue jos tarvetta.

## Liittyvät

- `#28` — alkuperäinen monivaluuttatuki-issue (C3, receipts-puoli valmis)
- `#71` — semantic-päätös joka voi vaikuttaa toteutukseen
- `#56` Phase 5 / Phase 5b — receipts/expenses siirto ops-kerrokseen
- `crates/ops/migrations/020_currency_fields.sql` — `expenses`-taulun kentät jo lisätty
- `crates/server/src/tools/expenses/{add,update}_expense.rs` — kytkettävät tool-wrapperit
