---
created: 2026-04-26
updated: 2026-04-26
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#2"]
labels: [ai, security]
---

# 15. Spoofing-suojaus: SPF/DKIM/DMARC-validointi ennen AI-käsittelyä

_Source: email-service AI agent_

## Description

Keskusteluhistoria on avainnettu lähettäjän sähköpostiosoitteella. Ilman SPF/DKIM/DMARC-tarkistusta kuka tahansa voi väärentää From-otsikon ja lukea/saastuttaa toisen käyttäjän keskusteluhistorian. Matkalaskuissa käsitellään henkilökohtaista taloustietoa.

## Scope

- [ ] Parsitaan Stalwartin `Authentication-Results`-otsikko email-servicessä
- [ ] Hylätään tai merkitään epäluotettavat lähettäjät (SPF/DKIM fail)
- [ ] Keskusteluhistorian avaimeen lisätään autentikoinnin tila
- [ ] Dokumentoidaan tietoturvamalli
