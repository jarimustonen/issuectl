---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: 5
labels: [integrations, netvisor]
---

# 18. Netvisor-integraatio — matkalaskujen vienti

_Source: services/backend (tuleva)_

## Description

Integraatio Visma Netvisor -taloushallintojärjestelmään matkalaskujen vientiä varten. Grooveserven AI-agentti koostaa matkalaskun ja lähettää sen Netvisoriin API:n kautta.

Netvisor on suomalainen pilvipalvelu taloushallintoon. API on REST-tyylinen, XML-pohjainen, ja käyttää HMAC-SHA256-autentikointia. Matkalaskujen vienti tapahtuu `tripexpense.nv`-endpointin kautta.

## Scope

- Netvisor API -clientin toteutus Rustilla
- HMAC-SHA256-autentikointi
- Matkalaskujen vienti (`tripexpense.nv`)
- Kulurivit, km-korvaukset, päivärahat
- Liitteet (kuitit)

## Status

- [x] API-tutkimus ja analyysi (2026-04-26)
- [x] Kumppanihakemus lähetetty Vismalle (2026-04-26)
- [ ] Testiympäristö ja API-tunnukset saatu — odottaa Visman vastausta
- [ ] Auth-moduuli (HMAC-SHA256)
- [ ] TripExpense-mallit + XML-serialisointi
- [ ] Integraatiotestaus testiympäristössä

## Analysis

Katso [analysis.md](analysis.md).
