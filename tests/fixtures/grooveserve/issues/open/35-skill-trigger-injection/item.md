---
created: 2026-04-28
updated: 2026-04-28
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#33"]
labels: [agent, tools, optimization]
---

# 35. Skill-sisällön ennakoiva injektio system-promptiin

_Source: `services/email/src/agent.rs`, `services/email/src/tools/`_

## Description

#33 toteuttaa skill-pohjaisen tool-arkkitehtuurin, jossa agentti kutsuu `read_skill`-meta-toolia ennen kunkin uuden työkalun käyttöä. Tämä lisää LLM-kierroksia, mikä on hyväksyttävää MVP:ssä mutta optimoinnin paikka myöhemmin.

Vaihtoehtona on **ennakoiva injektio** (proactive skill injection): tunnistetaan käyttäjän viestin sisällöstä mitä tooleja todennäköisesti tarvitaan, ja injektoidaan niiden skill-sisältö suoraan system-prompttiin tai kontekstiin ennen ensimmäistä LLM-kutsua. Tämä säästää kierroksia.

Mahdolliset tunnistustavat:

- **Avainsana­matchaus** (esim. liite + sana "kuitti" → injektoi `save_receipt`-skill).
- **Ekstraktio-pohjainen** (jos liite on jo OCR-prosessoitu, tiedämme että kuittikäsittely on tulossa).
- **Erillinen luokitin-LLM-kutsu** (kevyt malli päättää mitä skill-sisältöä tarvitaan).
- **Edellisten viestien historian perusteella** (sama keskustelu jatkuu, samat skillit ovat relevantteja).

Riskejä:

- Brittiläinen avainsanamatchaus monikielisyyden kanssa.
- Cache-invalidaatio jos system-prompt muuttuu per kutsu.
- Vääriä positiivisia (turha skill injektoidaan) ja negatiivisia (oikea skill jää injektoimatta).

## Tehtävä

Kun #33 toimii ja telemetria on kerätty:

1. Mittaa kuinka monta `read_skill`-kutsua per keskustelu tehdään keskimäärin / 90-persentiilissä.
2. Mittaa kuinka paljon ne lisäävät loop-iteraatioita ja latenssia.
3. Päätä onko optimointi tarpeen.
4. Jos tarpeen, suunnittele ja vertaile injektiotapoja.
5. Mittaa optimointivaikutus.

## Riippuvuudet

- #33 (skill-pohjainen tool-arkkitehtuuri ja sen telemetria).

## Ei tehdä ennen MVP:tä

Juuren `AGENTS.md` linjaa että MVP-vaiheessa optimointeja ei tehdä ennen kuin perustoiminnallisuus toimii ja todelliset kustannukset ovat tiedossa.
