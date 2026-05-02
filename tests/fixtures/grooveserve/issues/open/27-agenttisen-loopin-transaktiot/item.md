---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26", "#2"]
labels: [ai, architecture]
---

# 27. Agenttisen loopin transaktionaalisuus

_Source: AI-agentti, tool_use-käsittely_

_Continues: #26_

## Description

Suunnittele ja toteuta mekanismi, jolla agenttisen loopin tool-kutsut voidaan suorittaa atomisesti. Nykyisessä designissa (#26) jokainen tool-kutsu commitoituu erikseen, mikä aiheuttaa osittaista tilaa jos loop katkeaa kesken.

## Vaihtoehdot

### A: start_transaction / end_transaction -työkalut

Agentti kutsuu `start_transaction` alussa ja `end_transaction` lopussa. Välissä olevat operaatiot kerätään "pinoon" ja suoritetaan atomisesti end_transaction-kohdassa.

- Suojaudutaan end_transaction unohtamista vastaan muistuttamalla system promptissa
- Timeout: jos end_transaction ei tule X sekunnissa, rollback

### B: Pending-status tietokantariveissä

Jokainen tool-kutsu kirjoittaa rivin `status = 'pending'`. end_transaction muuttaa kaikki `confirmed`:ksi. Timeout: pending-rivit siivotaan.

### C: Implisiittinen transaktio per assistant-vastaus

Kaikki tool_use-blockit yhdestä assistant-vastauksesta wrapataan automaattisesti yhdeksi operaatioksi.

## Scope

- Arkkitehtuuripäätös: mikä vaihtoehto valitaan
- Virheenkäsittely: mitä tapahtuu timeout/keskeytyksessä
- Miten peruutetaan keskeneräinen transaktio
- Vaikutus tool-handler-rajapintaan
