---
created: 2026-04-29
updated: 2026-04-30
closed: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#33"]
labels: [agent, tools, sql, ocr]
commits:
  - hash: a193206
    summary: "feat(ops): add receipt_revisions table + capture pre-state on save/update (#38)"
  - hash: 79d2895
    summary: "test(ops/receipts): integration tests for revision history (#38)"
---

# 38. Säilytä kuittien aiemmat versiot OCR-uudelleenajossa

_Source: `services/email/src/tools/receipts/save_receipt.rs`_

## Description

Nykyinen `save_receipt` käyttää `ON CONFLICT (tenant_id, user_id, idempotency_key) DO UPDATE`-kuviota: kun sama fyysinen kuitti tulee uudelleen (esim. retry-jonon jälkeen tai mallin korjatessa OCR-tulosta), olemassa oleva rivi **kirjoitetaan kokonaan yli** EXCLUDED-arvoilla (paitsi `raw_text` / `items` / `confidence`, joissa on `COALESCE`). Tämä tarkoittaa, että aiemmat OCR-versiot katoavat lopullisesti.

LLM-tarkastuksessa Anthropic ehdotti `confidence`-pohjaista upsert-policya: päivitä vain jos uusi `confidence > vanha`. Idea ei ole kuitenkaan riittävä, koska:

1. **OCR-onnistuminen on käyttötapauskohtainen.** Jos käyttötapaus on ALV-tarkistus, "onnistuminen" tarkoittaa että ALV-rivi on luettavissa — vaikka kuitin alaosa olisi epäselvää. Toisessa käyttötapauksessa (kategorian valinta) onnistuminen on eri kriteeri. Yksi globaali `confidence`-luku ei kerro mihin on luotettu ja mihin ei.
2. **Päällikön korjaus ei välttämättä ole "huonompi".** Jos käyttäjä pyytää korjausta agentille, agentti voi joutua kirjoittamaan rivin uudelleen jossa OCR-confidence on matala mutta käyttäjän intent on selvä.

Parempi ratkaisu: **säilytä kaikki OCR-versiot revision-historiana** kuten `user_profile_revisions` tekee profiilipuolella. Sama fyysinen kuitti = sama `idempotency_key` = sama `receipt_id`, mutta jokaisesta päivityksestä jää audit-rivi `receipt_revisions` -tauluun (tai vastaavaan).

## Goals

- `save_receipt`-retryn jälkeen kuitin nykytila on uusin versio (kuten nyt — ei muutosta käyttäjäkokemukseen).
- Aiemmat OCR-versiot saatavilla revision-historiana: `(receipt_id, version, vendor, total_amount, currency, category, payment_method, raw_text, items, confidence, source_message_id, created_at, tool)`.
- Agentilla työkalu `restore_receipt_revision(receipt_id, version)` tai vastaava, jolla aiempi versio voidaan palauttaa nykytilaksi.
- Audit-trail kattaa myös käyttäjälähtöiset päivitykset (`update_receipt`).

## Non-goals

- Ei ole tämän issuen scopea ratkaista OCR-confidence-kenttäkohtaista esitystä — se on oma ongelmansa joka helpottuu kun versionhistoria on olemassa.
- Ei lisätä `restore_receipt_revision` -työkalua välittömästi; ensin riittää että data säilyy. Restore-polku voidaan toteuttaa kun käyttäjäpyyntö osoittaa tarpeen.

## Open questions

- Säilytetäänkö revision-rivit pysyvästi vai onko TTL? Verolainsäädäntö (#36) saattaa pakottaa pysyvyyden — verifioitava.
- Linkitetäänkö revision `extraction_id`:hen vai vain `receipt_id`:hen? `extraction_id` antaa tarkemman provenenssin (mistä OCR-passista versio tuli).
- Käytetäänkö samaa `record_profile_revision_tx`-tyylistä JSON-pohjaista snapshotia vai erillistä taulua jossa per-kenttäsarakkeet?

## Related

- #33 (skill-based tools) — perussiirto valmis, tämä on jatkoaskel.
- #34 (compound tools) — voi vaikuttaa jos `save_receipt + add_expense` tehdään yhdeksi transaktioksi.
- #36 (suomen lainsäädännön mukaisuus) — verolainsäädäntö voi vaatia revision-historiaa muutenkin.

## Background

Tämä spin-off on lähtöisin LLM-tarkastuksen löydöksestä #11 (post-fix-commits-arvostelu, 2026-04-29): "ON CONFLICT overwrites with worse OCR". Kun keskustelimme confidence-gatingia vaihtoehtona, päädyimme siihen, että revision-historia on parempi ratkaisu, koska se ei sido meitä yhteen onnistumismittariin (confidence) eikä menetä dataa.

## Resolution (C2, 2026-04-30)

Toteutettu worktree `C2-receipt-revision-history`:ssä uutta `crates/ops/src/receipts/`-pintaa vasten (A4b:n jälkeen).

**Schema (migraatio 019):** `receipt_revisions` per-kenttä-sarakkeilla (vertaa `user_profile_revisions`-mallin JSONB-snapshotia). Per-kenttä valittiin koska receipts-rivin shape on rakenteinen ja per-sarake-haut historiaan ("miten confidence on muuttunut", vendor-arvot) ovat luettavampia ja indeksoitavissa. Composite-FK:t `receipts(id, tenant_id, user_id)` ja `extractions(id, tenant_id, user_id)`:hin pitävät omistajaketjun ehjänä. Versionumerointi monotonic per `receipt_id`, alkaen 1:stä. Retentio toistaiseksi pysyvä (#36 voi muuttaa tämän).

**Toteutus:** uusi helper `receipts::revision::lock_and_record_revision_tx` lukee + lukitsee olemassa olevan rivin `SELECT ... FOR UPDATE`:lla ja kirjoittaa pre-staten revision-rivinä. `save_receipt` kutsuu helperiä ennen `INSERT...ON CONFLICT DO UPDATE`:a (ensimmäinen save ei kirjoita revisionia, kun vastaava rivi puuttuu). `update_receipt` kutsuu helperiä ennen UPDATE:a; `captured_by_message_id` lisätty `UpdateReceiptInput`-structiin että käyttäjälähtöiset korjaukset säilyttävät provenenssin. Sekä agentin (`save_receipt`) että käyttäjän (`update_receipt`) päivitykset auditoidaan.

**Out of scope:** `restore_receipt_revision`-tool (issue non-goal). Confidence-kenttäkohtainen esitys (oma ongelmansa). TTL/verolainsäädäntö (#36).

**Testit:** 10 sqlx-integraatiotestiä `crates/ops/src/receipts/tests.rs`:ssä — first-save-no-revision, monotonic versioning, save→save→update -ketju, extraction-id provenance, NotFound-haara, NULL captured_by_message_id (web-driven), cross-tenant isolation, ON DELETE RESTRICT blocks delete.

## LLM-review fix-up (2026-04-30)

`/llm-review` (4 mallia × 2 kierrosta) tunnisti useita parannuksia, joista seuraavat sovellettiin samaan worktreeseen ennen merge:ä:

- **`ON DELETE CASCADE` → `ON DELETE RESTRICT`** receipts-FK:lle migraatiossa 019. Audit-rivit eivät pyyhkiydy receiptin poiston yhteydessä. Jos hard-delete-tarve tulee myöhemmin, lisätään `deleted_at`-soft-delete-sarake; audit pysyy ehjänä.
- **Snapshot-sarakkeiden constraintit peilaavat parent-tauluun.** `status NOT NULL CHECK(...)`, `source NOT NULL CHECK(...)`, `payment_method CHECK(...)` migraatiossa 019. Estää mahdottomien historiarivien syntyä jos joku tekee suoria SQL-INSERTejä.
- **`source_tool` CHECK pudotettu**, kenttä uudelleennimetty `captured_by_tool` (vain `length() > 0`-CHECK jäljellä). Jokainen uusi tool ei vaadi schema-migraatiota.
- **`source_message_id` → `captured_by_message_id` rename.** Naming-rename: kentät kuvaavat nyt yksiselitteisesti *revision-tapahtuman* (= operaation joka korvasi pre-staten) provenanssia, ei snapshotin sisällön provenanssia. Pre-staten oma `message_id`/`source` jäävät snapshot-sarakkeisiin.
- **Versioning-mallin doc-kommentti** selvennetty migraatiossa (SCD-Type-4: nykytila receipts:issa, historia revisions:issa).
- **Redundantti indeksi `idx_receipt_revisions_receipt_version` poistettu** — `UNIQUE (receipt_id, version)` luo btreen jota PostgreSQL voi lukea takaperin.
- **`'cli'`-arvo lisätty `receipts.source` CHECK-listaan** (migraatio 013). Doc/koodi-drift `dev-cli/AGENTS.md`:n esimerkin kanssa korjattu.
- **3 uutta sqlx-testiä:** NULL captured_by_message_id (web-driven update), cross-tenant isolation (alice ei voi koskea bobin riviin), ON DELETE RESTRICT blocks delete.

Ratkaisematta jätetyt löydökset (omiksi issueikseen):
- **#68** `save_receipt`-upsertin kenttäkohtainen yli-kirjoituspolitiikka (receipt_date/payment_method NULL-overwrite + laajempi conflict-policy).
- **#69** Tool-wrapperien syöte-validointi yhtenäiseksi (currency/payment_method-validointi update_receipt:n wrapperissa).
- **#70** `receipts.source`-malli laajemmaksi (actor/channel/trace, naming-konventiot).

Concurrent-first-save -race ja "audit-grade vs convenience-cache"-kysymys: drop'attu — race on RARE (per-account single-task IMAP), ei osu MVP-skenaariossa. Jos #55 multi-worker IMAP tulee, palataan tähän.

**Review-raportti:** `history/review-c2-receipt-revision-history.md`.
