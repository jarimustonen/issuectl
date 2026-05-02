---
created: 2026-04-25
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: high
epic: 5
labels: [ai, email]
---

# 2. Agenttinen loop — sähköpostivastaukset AI-agentilla

_Source: ydintuote_

## Description

AI-agentti joka vastaanottaa kaikki grooveserve.com:iin saapuvat sähköpostit ja vastaa niihin. Tämä on järjestelmän ydin — agenttinen loop joka toimii palvelimella.

## Scope

- [x] LLM-integraatio (agentin "aivot") — Anthropic Messages API, reqwest + serde
- [x] Sähköpostin vastaanotto → agentin käsittely → vastauksen lähetys
- [x] Keskusteluhistorian ylläpito per käyttäjä/ketju — PostgreSQL conversations-taulu
- [x] Virhetilanteiden käsittely (agentin epävarmuus, virheelliset viestit)
- [ ] Tool use (kuitin OCR, kalenterihaku, veropäiväraha)
- [ ] Rakenteinen tilaobjekti (matkalaskuluonnos)
- [ ] ANTHROPIC_API_KEY SOPS-salattu tuotantoympäristöön
