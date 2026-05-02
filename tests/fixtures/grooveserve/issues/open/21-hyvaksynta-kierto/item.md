---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: high
epic: 5
labels: [email, approval, security]
---

# 21. Sähköpostipohjainen hyväksyntäkierto

_Source: services/email_

## Description

Matkalaskun hyväksyntäkierto sähköpostilla. Kun matkalaskun koostaminen on valmis, järjestelmä lähettää esimiehelle hyväksyntäpyynnön sähköpostiin. Esimies hyväksyy tai hylkää vastaamalla viestiin.

Keskeiset osa-alueet:
- Hyväksyntäpyynnön lähettäminen esimiehelle
- Vastausviestin tulkinta AI-agentilla (vapaa teksti)
- Agenttisen loopin konteksti-injektio (identiteetti, auth, expense)
- Tool-pohjainen oikeustarkistus
- Turvallisuus: DMARC-validointi, runtime-konteksti

## Context

Email-service käsittelee jo saapuvia viestejä IMAP IDLE:llä ja lähettää vastauksia SMTP:llä. Hyväksyntäkierto on agentin tool — ei erillinen järjestelmä.
