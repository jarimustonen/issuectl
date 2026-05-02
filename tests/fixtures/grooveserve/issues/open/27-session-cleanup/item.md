---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26"]
labels: [auth, maintenance]
---

# 27. Session cleanup — vanhentuneiden sessioiden siivous

_Source: #26 LLM review_

## Description

Vanhentuneet sessiot (expires_at < NOW()) kertyvät sessions-tauluun ikuisesti ilman siivousmekanismia. Tarvitaan periodinen job joka poistaa vanhentuneet rivit.

## Scope

- [ ] Periodinen siivousjob vanhentuneille sessioille (`DELETE FROM sessions WHERE expires_at < NOW()`)
- [ ] Sama vanhentuneille auth_tokens-riveille
- [ ] Toteutus: joko erillinen tokio-taski tai login-yhteydessä tapahtuva siivous
- [ ] Vanhentuneiden invitation-rivien status-päivitys ('pending' → 'expired')
