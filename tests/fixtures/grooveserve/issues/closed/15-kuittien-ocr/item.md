---
created: 2026-04-26
updated: 2026-05-01
closed: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
labels: [integraatio, tositteet]
related: ["#7", "#8", "#2"]
commits:
  - hash: "1563037"
    summary: "feat(server): tighten OCR extraction prompt + parser (#15)"
  - hash: "5a4a741"
    summary: "fix(server): address /llm-review round-1 findings on OCR tightening (#15)"
  - hash: "a292c5b"
    summary: "docs(issues): file #109/#110/#111 spinoffs from /llm-review of #15"
---

# 15. Kuittien OCR — tositteiden lukeminen kuvista ja PDF:stä

_Source: matkalaskupalvelu_

## Description

Integraatio kuittien ja tositteiden automaattiseen lukemiseen (OCR). Palvelu saa kuitit sähköpostiliitteinä (kuvat, PDF) ja ne pitää muuntaa rakenteiseksi dataksi (summa, päivämäärä, maksutapa, myyjä, ALV).

## Scope

- [x] API-tutkimus (mitä rajapintoja on tarjolla, autentikointi, rajoitukset)
- [x] Arkkitehtuurisuunnitelma (`design.md`)
- [x] MVP-toteutus — tiukennettu prompt + robusti parser + per-field-confidence + multi-currency-block + not_receipt-haara
- [x] Testaus — 13 unit-testiä parserille + 12 fixture-pohjaista integraatiotestiä; pre-existing 9 extraction_rescue-testiä passaa edelleen

## Päätös

LLM-pohjainen lähestymistapa: kuva lähetetään suoraan multimodaaliselle mallille, joka palauttaa strukturoidun JSON:n. Ei erillistä OCR-pipelinea.

MVP-kandidaatit: Gemini 2.5 Flash (~$0.003/kuitti) tai Claude Haiku 4.5 (~$0.005/kuitti). Toteutetaan kun agenttinen looppi (#2) on valmis — tämä tulee osaksi agentin ominaisuuksia.

**MVP-toteutusvalinta (C1, 2026-05-01)**: pidetään yksi malli (`ANTHROPIC_MODEL` = Sonnet 4.6, jolla agenttilooppi jo pyörii) extraction-puolellakin. Trait-rajaus ja Gemini Flash / Haiku 4.5 -fallback on filed follow-upiksi (kts. `design.md`). Live-LLM-precision-benchmark anonymisoidulla testidatasetillä on toinen follow-up.

## Analysis

Katso [analysis.md](analysis.md).

## Design

Katso [design.md](design.md).
