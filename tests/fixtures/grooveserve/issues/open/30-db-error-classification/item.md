---
created: 2026-04-27
updated: 2026-04-27
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26"]
labels: [reliability, error-handling]
---

# 30. Tietokantavirheiden luokittelu ja käsittelystrategia

_Source: 4-LLM review (#26 Phase 1 implementation)_

## Description

`AgentError::Database` luokittelee kaikki tietokantavirheet transienteiksi (`is_transient() = true`). Todellisuudessa:

- **Transientit:** yhteyskatkos, timeout, deadlock → retry OK
- **Pysyvät:** CHECK violation, schema mismatch, puuttuva sarake → retry turhaa

Tarvitaan:
- Tool-handlereiden virheiden luokittelu: onko virhe sellainen jonka LLM voi korjata (virheellinen syöte), sellainen joka johtuu järjestelmävirheestä (yhteyskatkos), vai sellainen joka on bugi (schema mismatch)
- Käyttäjälle ei saa vuotaa raakoja tietokantavirheilmoituksia
- Kutsuvassa koodissa tunnistetaan virhetyyppi ja wrapataan käyttäjäystävälliseksi
- Pysyvät virheet eivät laukaise retry-looppia
