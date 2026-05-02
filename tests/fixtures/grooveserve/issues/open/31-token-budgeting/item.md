---
created: 2026-04-27
updated: 2026-04-27
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26", "#14"]
labels: [opex, ai]
---

# 31. Token-budjetointi ja historian hallinta

_Source: 4-LLM review (#26 Phase 1 implementation)_

## Description

Keskusteluhistoria ladataan nyt kokonaisuudessaan ilman LIMIT:iä. Pitkien keskustelujen token-käyttö kasvaa rajatta.

Tarvitaan:
- Token-estimaatti per viesti (esim. content_json:n koko / 4)
- Historian katkaisu token-budjetin mukaan, ei rivimäärän
- Vanhojen tool_result-viestien tiivistäminen (korvaa iso JSON-blob yhteenvedolla)
- Per-tenant/per-user päiväkohtainen kustannuskatto
- Kustannusten seuranta ja hälytykset

Liittyy: #14 (opex-hallinta)
