---
created: 2026-04-27
updated: 2026-04-27
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26"]
labels: [ai, ux]
---

# 32. Viestin intent-tunnistus — matkalaskuasia vai yleinen kysymys

_Source: keskustelu Phase 1 review -yhteydessä_

## Description

Agentti vastaa tällä hetkellä kaikkiin viesteihin matkalaskuassistenttina. Pitäisi tunnistaa:

1. **Matkalaskuasia:** kuitteja, kuluja, matkoja koskevat viestit → käsittele normaalisti
2. **Yleinen kysymys:** "Mikä on Suomen pääkaupunki?" → ei kuulu palvelun piiriin, vastaa kohteliaasti ettei käsittele yleisiä kysymyksiä
3. **Epäselvä:** → kysy tarkennusta

Tämä on iso riski kustannuksille: jokainen LLM-kutsu maksaa, ja yleisiin kyselyihin vastaamisesta ei ole hyötyä.

Vaihtoehtoisia lähestymistapoja:
- Kevyt esitarkistus (pikaparsinta ennen agenttista looppia)
- System promptin tiukentaminen (kieltää vastaamasta muuhun kuin matkalaskuasioihin)
- Ensimmäisen vastauksen tarkistus (jos ei tool-kutsuja → todennäköisesti turha vastaus)
