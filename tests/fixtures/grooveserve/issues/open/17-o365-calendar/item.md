---
created: 2026-04-26
updated: 2026-04-27
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
epic: 5
labels: [calendar, integration, o365]
---

# 17. O365/Outlook Calendar -integraatio

_Source: services/email, AI-agentti_

## Description

Microsoft 365 / Outlook -kalenterin lukeminen Microsoft Graph API:n kautta. Mahdollistaa käyttäjän kalenteritapahtumien (kokoukset, matkat, poissaolot) hyödyntämisen matkalaskujen automaattisessa koostamisessa.

Kalenteridata yhdistettynä kuitteihin ja käyttäjäprofiiliin antaa AI-agentille kontekstin: milloin käyttäjä oli matkalla, missä, ja kenen kanssa.

## Scope

- Microsoft Graph API calendar events -lukuoikeus (`calendarView`)
- OAuth 2.0 Authorization Code + PKCE (confidential client, delegated permissions)
- Rust-toteutus (`reqwest` + `oauth2`, ei `graph-rs-sdk`)
- Vertailu Google Calendar -integraatioon (yhteinen trait myöhemmin, ei MVP:ssä)
- GDPR/tietosuoja: datan minimointi, yksityiset tapahtumat, irtikytkentä
- Virheenkäsittely: Graph API -virheet, retry/backoff, token refresh rotation

## Dependencies

- Entra ID (Azure AD) app registration (multi-tenant organizations, verified publisher)
- Käyttäjän OAuth-suostumus (consent) + admin-consent flow B2B-asiakkaille
- Token-hallinta (refresh token rotation, AEAD-salaus, PostgreSQL)
- Aikavyöhykekirjasto (`chrono-tz`)

## Arvioitu kesto

- Prototyyppi: 4-7 päivää
- Tuotanto-MVP: 3-4 viikkoa
