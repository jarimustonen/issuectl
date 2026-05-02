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
labels: [reliability, email]
---

# 29. Retry-backoff ja transienttien virheiden aikarajat

_Source: 4-LLM review (#26 Phase 1 implementation)_

## Description

Transientti virhe (esim. Anthropic API timeout) jättää viestin näkymättömäksi INBOX:iin ja status = "retryable". Seuraava IMAP IDLE -herääminen prosessoi sen heti uudelleen, ilman backoffia. Tämä aiheuttaa tiiviin retry-loopin joka polttaa API-krediittejä.

Tarvitaan:
- `attempt_count` ja `next_attempt_at` -sarakkeet `email_processing`-tauluun
- Eksponentiaalinen backoff (esim. 30s, 2min, 10min, 1h)
- Max retry count (esim. 5), jonka jälkeen status → `failed`
- `try_claim_message` kieltäytyy ennen `next_attempt_at`:ia

Lisäksi: mainin puolella on tehty retry-logiikkaa — tarkistetaan merge-yhteydessä onko se jo riittävä.
