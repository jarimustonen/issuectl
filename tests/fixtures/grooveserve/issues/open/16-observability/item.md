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
labels: [ai, ops]
---

# 16. Observability: AI-metriikat ja kustannusseuranta

_Source: email-service AI agent_

## Description

AI-agentin toiminnasta puuttuu seuranta. Tarvitaan metriikat ja alertit tuotantokäyttöä varten.

## Scope

- [ ] Inbound messages by account/action (counter)
- [ ] AI success/failure/fallback counts
- [ ] Anthropic latency (histogram)
- [ ] Token usage per request/sender
- [ ] Retry counts ja status code distribution
- [ ] SMTP failure count
- [ ] Alertit kriittisistä virhetilanteista
