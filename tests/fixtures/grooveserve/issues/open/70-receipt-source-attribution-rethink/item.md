---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#38", "#57"]
labels: [ops, receipts, schema, audit, design]
---

# 70. `receipts.source`-malli laajemmaksi: kuka, milloin, mistä kanavasta

_Source: `crates/ops/migrations/013_create_receipts.sql`, `019_create_receipt_revisions.sql`, ja kaikki `source*`/`*_by_*`-kentät receipts-/revisions-perheessä_

## Description

`receipts.source` on tällä hetkellä yksinkertainen `TEXT`-enum: `('email', 'web', 'api', 'cli')`. Se vastaa kysymykseen "minkä *teknisen kanavan* kautta tämä rivi luotiin", mutta ei mitään muuta. Suunnilleen samaa tehtävää hoitavat `receipt_revisions.captured_by_message_id` ja `captured_by_tool` revision-tasolla, mutta nekin ovat kapeita.

Kun tarkastellaan kuittikohtaisia kysymyksiä joihin systeemin halutaan vastaavan:

- "Kuka loi tämän rivin (ihminen, agentti, admin)?"
- "Milloin se luotiin? Milloin se muuttui? Milloin agentti viimeksi kosketti?"
- "Mistä kanavasta (sähköposti, web-UI, REST-API, CLI, batch-import)?"
- "Onko tämä rivi *peräisin* OCR-extractionista vai käyttäjän käsin syöttämä? (extraction_id kertoo osan tästä, mutta ei kaikkea.)"
- "Onko jokin kentistä **käyttäjän lukitsema** ettei agentti yli-kirjoita?"
- "Mikä viesti / mikä run liittyy tähän riviin?"

Nämä eivät kaikki mahdu nykyiseen `source TEXT`-sarakkeeseen. Käytännössä ne hajautuvat:

- `source` (kanava) — `receipts`
- `extraction_id` (OCR-pohjaisuus) — `receipts`
- `created_at`/`updated_at` (aika) — `receipts` ja `receipt_revisions`
- `captured_by_tool`/`captured_by_message_id` — vain `receipt_revisions`
- `OpContext.actor_user_id`/`channel`/`trace_id` — **ei tallenneta mihinkään**

C2:n LLM-arviossa GPT-5.5 ja Claude nostivat eri kulmista esiin että `receipt_revisions.user_id` on kuitin omistaja, ei välttämättä toimijan id (admin-flow:ssa nämä eroavat). Lisäksi sähköposti- ja web-vetoiset päivitykset ovat tällä hetkellä audit-jäljessä erottamattomia ilman `source_message_id`-puuttumisen päättelyä.

## Goals

Tämän issuen tarkoitus on **avata suunnittelukysymys** — ei lyödä ratkaisua lukkoon. Mahdolliset suunnat:

1. **Laajenna `receipts.source` rakenteiseksi.** Esim. siirry kategoriasta yhteen kapeaan saraksesta useampaan sarakkeeseen tai yhteen JSONB:hen jossa on kanava + actor + trace. Vrt. agent_runs (`crates/ops/migrations/018_create_agent_trace.sql`) joka tallentaa per-run jo tarkkana toimijaa.
2. **Lisää eksplisiitti `actor_user_id`** sekä `receipts`:iin että `receipt_revisions`:iin. Tällä hetkellä `user_id` on omistaja; toimija voi tulevassa admin-flow:ssa olla eri (esim. taloushallinto kirjaa kuitin toisen käyttäjän puolesta).
3. **Aikaleimat per-kenttä?** Joillakin kentillä voi olla "milloin viimeksi koskettiin"-tieto; tämä auttaa konflikti-policyä (#68).
4. **Kytke `receipts` → `agent_runs`** suoraan: jos rivin loi/koski agent-run, tallenna `agent_run_id`. Tämä korreloi #57:n asiantuntijanäkymän kanssa.
5. **Nimeämiskonventiot:** `source` vs. `source_tool` vs. `captured_by_tool` vs. `actor` vs. `channel` — semantiikka on tällä hetkellä sumea. Päätä vakiintuneet nimet ja dokumentoi.

## Non-goals

- **Ei** lopeta käyttöä `receipts.source TEXT`-saraketta välittömästi — tarvitaan suunnitelma siirtymälle.
- **Ei** lisää `actor_user_id` & co. spekulatiivisesti ilman konkreettista käyttötapaa. CLAUDE.md:n MVP-periaate: ei ennalta-suunniteltua schemaa hypoteettisille tarpeille. Sen sijaan: päätä konkreettisesti milloin actor ≠ owner alkaa olla mahdollinen (admin-impersonation-flow #22?), ja lisää sarakkeet siinä yhteydessä.

## Background

Nousi kahdesta paikasta:

1. C2:n LLM-arvio (`history/review-c2-receipt-revision-history.md`) — useat reviewerit huomasivat että revisions-rivien provenance on kapea (ei actor/channel/trace), ja `source_tool` vs. `captured_by_tool` -nimien semantiikka ei ole itsestäänselvä. Naming-osa korjattiin C2:ssa (rename → `captured_by_*`); skema-laajennus jätettiin tähän erilliseen issueen.
2. C2:n yhteydessä huomattiin että `receipts.source CHECK` ei sallinut `'cli'`-arvoa vaikka `dev-cli/AGENTS.md` ja `SaveReceiptInput.source`-doccomment viittasivat siihen. C2 lisäsi `'cli'` CHECK-listaan välitömänä korjauksena, mutta tämä paljasti että lähde-attribuutio on jo nyt epäsiisti — useita ad-hoc-arvoja, ei selkeää mallia.

Pre-existing rakenne-ongelma, ei C2:n aiheuttama, mutta C2:n review-kierros teki sen näkyväksi.

## Related

- #38 — Receipt revision-history. C2 lisäsi `captured_by_*`-kentät; tämä issue jatkaa siitä.
- #57 — Auditoitavuus / asiantuntijanäkymä. `agent_runs.run_uuid`-linkitys receipts:iin olisi luonnollinen sivutuotos.
- #22 — Admin-näkymä käyttäjähallintaan; admin-impersonation tekee `actor ≠ owner` -tapauksesta konkreettisen.
- #11 — Tositteiden korjaus webistä; web-driven päivitysten erottaminen agent-driven-päivityksistä helpottuu.
- #68 — `save_receipt`-kenttäkonflikti. Riippuu osittain siitä mitä actor-/channel-tietoa on saatavilla ratkaisemassa konflikteja.
