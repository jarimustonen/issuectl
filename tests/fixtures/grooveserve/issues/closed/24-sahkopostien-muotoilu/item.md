---
created: 2026-04-26
updated: 2026-04-26
type: improvement
reporter: jari
assignee: jari
status: done
closed: 2026-04-26
priority: normal
labels: [email, ux]
commits:
  - hash: d645f86
    summary: "feat(email): add HTML email formatting with multipart support"
  - hash: b51c859
    summary: "fix(email): improve email rendering, add button support, and fix multiple issues"
  - hash: 98d71b0
    summary: "fix(email): describe email rendering pipeline in system prompt"
---

# 24. Sähköpostien muotoilu — HTML + plaintext multipart

_Source: services/email_

## Description

Grooveserven lähettämät sähköpostit ovat tällä hetkellä pelkkää plaintext-muotoa. Tämä issue kattaa siirtymisen HTML + plaintext multipart -viesteihin, jotka ovat brändin mukaisia ja ammattimaisen näköisiä.

Kattaa:
1. **Multipart/alternative**: HTML + plaintext kaikissa viesteissä
2. **Markdown → HTML**: LLM tuottaa markdownia → `comrak` konvertoi HTML:ksi
3. **HTML-template**: brändin mukainen (header, footer, fontti, värit)
4. **Yhtenäinen footer/allekirjoitus**: kaikissa viesteissä
5. **Per-viestityyppi templateit**: AI-vastaus, healthcheck, error/fallback

## Viestityyppit

| Tyyppi | Lähettäjä | Sisältö |
|--------|-----------|---------|
| AI-vastaus | assistant@ | Claude-vastaus matkalaskuasiassa (markdown → HTML) |
| Healthcheck | healthcheck@ | Ping-pong status |
| Virhe/fallback | assistant@ | Virheilmoitus käsittelyongelmasta |
