---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
labels: [extraction, cost-control, idempotency]
related: ["#15", "#46"]
---

# 111. Vision-API-kutsu duplikoituu IMAP reclaim -poluilla

_Source: matkalaskupalvelu_

## Description

`crates/server/src/ingest/extraction.rs::process_attachment` -funktio kutsuu Anthropic-vision-APIa (`extract_with_vision`) **ennen** `record_extraction`-tallennusta. `record_extraction` on idempotent (UNIQUE attachment_id, ON CONFLICT DO UPDATE — migraatio 013), joten DB-rivi pysyy yhtenä kun reclaim-haara kutsuu uudelleen.

Mutta itse vision-kutsu ajetaan joka kerta:

```
attempt 1: save_attachment → vision call ($) → record_extraction
attempt 2 (reclaim): save_attachment (ON CONFLICT no-op) → vision call ($) again → record_extraction (UPDATE)
attempt 3 (reclaim): vision call ($) again → ...
```

Jokainen reclaim maksaa vision-tokenit uudelleen. PoC-vaiheessa retry-volyymit ovat pieniä, joten tämä ei ole akuutti, mutta efficiency-improvement.

## Scope

- [ ] Lisää `ops::extractions::exists_for_attachment(pool, ctx, attachment_id) -> bool` -kysely
- [ ] `process_attachment` tarkistaa olemassaolon `save_attachment`-jälkeen ennen `extract_with_vision`-kutsua
- [ ] Jos olemassa, lue `extracted_data` takaisin ja palauta yhteenveto (sama formaatti kuin first-run)
- [ ] Tarkkana ettei rikota olemassa olevaa "reclaim updates extraction_data" -käyttäytymistä — joka oikeuttaa **prompt change → reprocess** -skenaarion. Eli olemassaolocheckin pitää olla **ajallisesti tai versionoidusti rajattu**, esim. "skip vision jos extraction tallennettu samalla mallilla viimeisen 24h aikana".
- [ ] Päivitä `extraction_rescue.rs`-testit:
  - Vahvista että reclaim **ei kutsu** vision-APIa kun extraction on jo olemassa
  - Vahvista että reclaim **kutsuu** vision-APIa kun model on muuttunut

## Why this is subtle

Olemassa oleva testi `reclaim_updates_extraction_data` (extraction_rescue.rs:507) sanoo nimenomaan että reclaim *pitää* kutsua vision-APIa uudelleen:
> "If a Reclaim happens after a model upgrade or prompt change, the
>  ON CONFLICT DO UPDATE clause must refresh extracted_data so the
>  newer extraction wins."

Eli pelkkä unconditional skip rikkoo tämän. Korjaus vaatii että lasketaan ehto jolla skip aktivoituu — esim. (model unchanged AND `record_extraction.created_at` < 24h sitten).

Tai vaihtoehtoisesti: trace-loki kertoo monta kertaa vision-kutsu on tehty tälle attachment_id:lle ja mitataan kustannukset. Jos reclaim-volyymi on pieni, tämän voi jättää ottamatta käyttöön.

## Background

Filed C1-worktreessä `/llm-review`-kierroksen löydösten pohjalta:
- Gemini §1.2 (idempotency flaw causes severe LLM token burn)
- Liittyy #46:een (extraction-rescue) — siellä rakennettiin idempotency mutta vain DB-tasolla
