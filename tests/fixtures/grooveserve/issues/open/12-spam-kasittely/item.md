---
created: 2026-04-25
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: 1
labels: [email, infra]
commits: []
---

# 12. Spam-käsittely sähköpostipalvelussa

_Source: email-service, Stalwart_

## Description

Stalwartin sisäänrakennettu spam-filtteri on disabloitu (`[spam-filter] enable = false`) koska se luokittelee kaiken spämiksi tuoreessa asennuksessa (ei koulutettua mallia).

Grooveservessä tarvitaan räätälöity spam-käsittely eri osoitteille:

- **`healthcheck@`** — ei spam-filtteriä (pitää vastata aina)
- **`assistant@`** — AI-pohjainen luokittelu osana agentin käsittelyä (ei perinteinen spam-filtteri)
- **`postmaster@`** — perus spam-filtteri (DMARC-raportit sallitaan)

Perinteinen spam-filtteri ei sovi koska matkalaskupalvelun kontekstissa "spam" on eri asia kuin normaali sähköposti-spam. Kuitit, laskut ja kulutusilmoitukset voivat näyttää spämiltä perinteiselle filtterille.

## Vaihtoehtoja

1. **AI-pohjainen luokittelu** — LLM arvioi viestin osana agentin käsittelyä
2. **Stalwartin oma filtteri koulutettuna** — vaatii training-dataa
3. **Kolmannen osapuolen spam-API** (SpamAssassin, Rspamd)
4. **Yksinkertainen rule-based** — SPF/DKIM/DMARC pass + allow-lista

## Nykytila

Spam-filtteri disabloitu. Kaikki viestit tulevat läpi. Tämä on OK MVP:lle mutta pitää ratkaista ennen tuotantoa.
