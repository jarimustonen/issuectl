---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26", "#2"]
labels: [ai, agent]
---

# 29. Agent tool_result -virheiden serialisointi

_Source: #26 LLM review_

## Description

Kun AI-agentin tool_use-kutsu epäonnistuu (esim. `OpError::AlreadyExists`, `OpError::Forbidden`), virhe pitää serialisoida `tool_result`-blokiksi siten, että LLM ymmärtää mitä tapahtui ja voi antaa käyttäjälle järkevän vastauksen.

Tarvitaan:
- Standardisoidut virhekoodit (`user_not_found`, `email_already_taken`, `permission_denied`, `last_admin_protected`)
- Selkeä JSON-muoto tool_result-virheille
- Testattava mapping OpError → tool_result

## Scope

- [ ] Suunnittele `OpError` → `tool_result` -mappaus
- [ ] Standardisoi virhekoodit (koneluettavia, LLM:n ymmärrettäviä)
- [ ] Testaa prompteilla: näyttääkö agentti järkeviä virheilmoituksia käyttäjälle
- [ ] Dokumentoi tool_result-virheen JSON-muoto
